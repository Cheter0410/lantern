//! Videos on slides, played with FFmpeg (shipped next to Lantern, else the
//! one on the system). FFmpeg runs as its own program: one process decodes
//! the picture (RGBA, at the size it has on the output, constant frame
//! rate), one the sound (stereo float at the mixer's rate). The sound
//! actually played is the clock; the picture follows it.

use crate::audio::{Mixer, Voice};
use crate::deck::Image;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

/// FFmpeg: next to Lantern (the packages ship it), else on the system.
pub fn ffmpeg() -> Option<PathBuf> {
    let name = exe_name();
    let mut c: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            c.push(dir.join(name));
            // macOS app bundle: Contents/MacOS/lantern -> Contents/Resources.
            c.push(dir.join("../Resources").join(name));
            // Linux install: …/bin/lantern -> …/lib/lantern.
            c.push(dir.join("../lib/lantern").join(name));
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        c.extend(std::env::split_paths(&paths).map(|d| d.join(name)));
    }
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        c.push(Path::new(d).join(name));
    }
    c.into_iter().find(|p| p.is_file())
}

fn command(ffmpeg: &Path) -> Command {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-nostdin"]);
    crate::office::hide_window(&mut cmd);
    cmd
}

/// What FFmpeg says about a file.
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    /// Seconds.
    pub duration: Option<f64>,
    pub audio: bool,
    pub video: bool,
}

/// `Duration: 00:01:02.50,` -> 62.5
fn parse_duration(text: &str) -> Option<f64> {
    let rest = text.split("Duration: ").nth(1)?;
    let t = rest.split(',').next()?.trim();
    let mut parts = t.split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

fn parse_probe(text: &str) -> Probe {
    // Stream lines look like "  Stream #0:1[0x2](und): Audio: aac …".
    let has = |kind: &str| {
        text.lines()
            .any(|l| l.trim_start().starts_with("Stream #") && l.contains(kind))
    };
    Probe {
        duration: parse_duration(text),
        audio: has(": Audio:"),
        video: has(": Video:"),
    }
}

pub fn probe(ffmpeg: &Path, file: &Path) -> Result<Probe, String> {
    // Without an output FFmpeg just describes the input (and exits with 1).
    let out = command(ffmpeg)
        .arg("-i")
        .arg(file)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("FFmpeg could not be started: {e}"))?;
    let text = String::from_utf8_lossy(&out.stderr);
    let p = parse_probe(&text);
    if !p.video && !p.audio {
        let last = text.lines().last().unwrap_or("").trim().to_string();
        return Err(format!("FFmpeg can't read this file: {last}"));
    }
    Ok(p)
}

/// A slide's video, ready to play.
#[derive(Clone, Debug)]
pub struct Clip {
    pub file: PathBuf,
    pub probe: Probe,
}

/// How to play a clip.
pub struct Start<'a> {
    pub ffmpeg: &'a Path,
    pub clip: &'a Clip,
    /// Picture size in pixels.
    pub w: u32,
    pub h: u32,
    pub fps: u32,
    pub looped: bool,
    pub mixer: &'a Arc<Mixer>,
    /// 0..1.
    pub gain: f32,
    /// Start this many seconds in.
    pub from: f64,
    /// Decode the picture (false: sound only, e.g. an audio clip).
    pub picture: bool,
}

/// A clip playing.
pub struct Playing {
    children: Vec<Child>,
    frames: Receiver<Image>,
    /// Index of the next frame the queue gives.
    next_index: u64,
    shown: Option<Image>,
    /// Frame waiting for its time.
    pending: Option<Image>,
    fps: f64,
    pub voice: Option<Arc<Voice>>,
    rate: u32,
    started: Instant,
    /// Position the processes started at (after a seek).
    offset: f64,
    /// Paused since, and all the time paused before.
    paused_at: Option<Instant>,
    paused_for: Duration,
    /// Sound over (short track): the clock goes on from here by the wall
    /// (clock, wall seconds then).
    sound_end: Mutex<Option<(f64, f64)>>,
    /// To start again somewhere else.
    ffmpeg: PathBuf,
    clip: Clip,
    size: (u32, u32),
    fps_in: u32,
    picture: bool,
    mixer: Arc<Mixer>,
    gain: f32,
    /// The decoder finished and every frame was taken.
    drained: bool,
    pub duration: Option<f64>,
    pub looped: bool,
}

impl Drop for Playing {
    fn drop(&mut self) {
        for c in &mut self.children {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Playing {
    pub fn start(s: Start<'_>) -> Result<Playing, String> {
        let (w, h) = (s.w.max(2) & !1, s.h.max(2) & !1);
        let fps = s.fps.clamp(1, 120);
        let from = s.from.max(0.0);
        let mut input: Vec<String> = Vec::new();
        if s.looped {
            input.extend(["-stream_loop".into(), "-1".into()]);
        }
        if from > 0.0 {
            input.extend(["-ss".into(), format!("{from:.3}")]);
        }
        let mut children = Vec::new();
        let (tx, frames) = sync_channel::<Image>(6);
        let picture = s.picture && s.clip.probe.video;
        if !picture {
            // No picture: the queue is closed (drained at once).
            drop(tx);
        } else {
            let mut vc = command(s.ffmpeg)
                .args(["-loglevel", "error"])
                .args(&input)
                .arg("-i")
                .arg(&s.clip.file)
                .args(["-an", "-sn", "-vf"])
                .arg(format!("fps={fps},scale={w}:{h}:flags=bicubic"))
                .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("FFmpeg could not be started: {e}"))?;
            let mut out = vc.stdout.take().ok_or("FFmpeg: no output")?;
            children.push(vc);
            let size = (w * h * 4) as usize;
            std::thread::Builder::new()
                .name("lantern-video".into())
                .spawn(move || loop {
                    let mut buf = vec![0u8; size];
                    if out.read_exact(&mut buf).is_err() {
                        break;
                    }
                    let img = Image {
                        w,
                        h,
                        rgba: Arc::new(buf),
                    };
                    if tx.send(img).is_err() {
                        break;
                    }
                })
                .map_err(|e| e.to_string())?;
        }

        // Sound.
        let rate = s.mixer.rate();
        let voice = if s.clip.probe.audio {
            let mut ac = command(s.ffmpeg)
                .args(["-loglevel", "error"])
                .args(&input)
                .arg("-i")
                .arg(&s.clip.file)
                .args(["-vn", "-sn", "-ac", "2", "-ar"])
                .arg(rate.to_string())
                .args(["-f", "f32le", "pipe:1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("FFmpeg could not be started: {e}"))?;
            let mut out = ac.stdout.take().ok_or("FFmpeg: no output")?;
            children.push(ac);
            let v = Voice::new(s.gain);
            let v2 = v.clone();
            std::thread::Builder::new()
                .name("lantern-video-sound".into())
                .spawn(move || {
                    let mut buf = vec![0u8; 16 * 1024];
                    let mut samples = Vec::new();
                    let mut carry: Vec<u8> = Vec::new();
                    loop {
                        // Stay about half a second ahead of the output.
                        while v2.queued() > rate as usize && !v2.is_gone() {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        if v2.is_gone() {
                            break;
                        }
                        let n = match out.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        carry.extend_from_slice(&buf[..n]);
                        let whole = carry.len() / 4 * 4;
                        samples.clear();
                        samples.extend(
                            carry[..whole]
                                .chunks_exact(4)
                                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                        );
                        carry.drain(..whole);
                        v2.push(&samples);
                    }
                    v2.eof.store(true, Ordering::Relaxed);
                })
                .map_err(|e| e.to_string())?;
            s.mixer.add(v.clone());
            Some(v)
        } else {
            None
        };
        Ok(Playing {
            children,
            frames,
            next_index: (from * f64::from(fps)).floor() as u64,
            shown: None,
            pending: None,
            fps: f64::from(fps),
            voice,
            rate,
            started: Instant::now(),
            offset: from,
            paused_at: None,
            paused_for: Duration::ZERO,
            sound_end: Mutex::new(None),
            drained: false,
            duration: s.clip.probe.duration,
            looped: s.looped,
            ffmpeg: s.ffmpeg.to_path_buf(),
            clip: s.clip.clone(),
            size: (s.w, s.h),
            fps_in: s.fps,
            picture: s.picture,
            mixer: s.mixer.clone(),
            gain: s.gain,
        })
    }

    /// Seconds since the start, without the time paused.
    fn wall(&self) -> f64 {
        let now = self.paused_at.unwrap_or_else(Instant::now);
        now.duration_since(self.started)
            .saturating_sub(self.paused_for)
            .as_secs_f64()
    }

    /// Seconds into the clip (counting on over loops): the sound played so
    /// far, else the wall clock.
    pub fn clock(&self) -> f64 {
        let wall = self.wall();
        let Some(v) = &self.voice else {
            return self.offset + wall;
        };
        let t = self.offset + v.played() as f64 / f64::from(self.rate.max(1));
        if !v.is_gone() {
            return t;
        }
        let Ok(mut end) = self.sound_end.lock() else {
            return t;
        };
        let (at, then) = *end.get_or_insert((t, wall));
        at + (wall - then)
    }

    /// Position within the clip (seconds), for the timeline.
    pub fn position(&self) -> f64 {
        let t = self.clock();
        match self.duration.filter(|d| *d > 0.0) {
            Some(d) if self.looped => t % d,
            Some(d) => t.min(d),
            None => t,
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    /// Stop where it is (the picture stays), or play on.
    pub fn set_paused(&mut self, paused: bool) {
        match (paused, self.paused_at) {
            (true, None) => self.paused_at = Some(Instant::now()),
            (false, Some(at)) => {
                self.paused_for += at.elapsed();
                self.paused_at = None;
            }
            _ => return,
        }
        if let Some(v) = &self.voice {
            v.set_paused(paused, false);
        }
    }

    /// Go to `to` seconds (start the decoders there again). Keeps paused
    /// or playing, the volume and the picture until the new one comes.
    pub fn seek(&mut self, to: f64) -> Result<(), String> {
        let end = self.duration.unwrap_or(f64::MAX);
        let to = to.clamp(0.0, (end - 0.05).max(0.0));
        let mut new = Playing::start(Start {
            ffmpeg: &self.ffmpeg,
            clip: &self.clip,
            w: self.size.0,
            h: self.size.1,
            fps: self.fps_in,
            looped: self.looped,
            mixer: &self.mixer,
            gain: self.gain,
            from: to,
            picture: self.picture,
        })?;
        if self.is_paused() {
            new.paused_at = Some(new.started);
            if let Some(v) = &new.voice {
                v.set_paused(true, true);
            }
        }
        new.shown = self.shown.take();
        if let Some(v) = &self.voice {
            v.stop(0.02, self.rate);
        }
        *self = new;
        Ok(())
    }

    /// The frame for now; `true` when it changed.
    pub fn update(&mut self) -> bool {
        let want = (self.clock() * self.fps).floor() as u64;
        let mut changed = false;
        loop {
            let next = match self.pending.take() {
                Some(f) => f,
                None => match self.frames.try_recv() {
                    Ok(f) => f,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.drained = true;
                        break;
                    }
                },
            };
            if self.next_index > want && self.shown.is_some() {
                // Not yet.
                self.pending = Some(next);
                break;
            }
            self.shown = Some(next);
            self.next_index += 1;
            changed = true;
        }
        changed
    }

    pub fn frame(&self) -> Option<&Image> {
        self.shown.as_ref()
    }

    /// Played to the end (never when looping).
    pub fn ended(&self) -> bool {
        !self.looped
            && self.drained
            && self.pending.is_none()
            && self.voice.as_ref().is_none_or(|v| v.is_gone())
    }

    /// Seconds left (within the current round when looping).
    pub fn remaining(&self) -> Option<f64> {
        let d = self.duration.filter(|d| *d > 0.0)?;
        let t = self.clock();
        Some(if self.looped {
            d - t % d
        } else {
            (d - t).max(0.0)
        })
    }

    /// Fade the sound to `gain` within `secs`.
    pub fn set_gain(&mut self, gain: f32, secs: f32) {
        self.gain = gain;
        if let Some(v) = &self.voice {
            v.set_gain(gain, secs, self.rate);
        }
    }

    /// Fade the sound out; the processes stop when this is dropped.
    pub fn fade_out(&self, secs: f32) {
        if let Some(v) = &self.voice {
            v.stop(secs, self.rate);
        }
    }

    /// Faded out completely (safe to drop without a click).
    pub fn silent(&self) -> bool {
        self.voice.as_ref().is_none_or(|v| v.is_gone())
    }
}

/// Paste `video` (already at its pixel size) into a copy of `base` with its
/// top left corner at `at`.
pub fn paste(base: &Image, video: &Image, at: (u32, u32)) -> Image {
    let mut out = (*base.rgba).clone();
    let (x0, y0) = at;
    let cols = video.w.min(base.w.saturating_sub(x0)) as usize;
    for y in 0..video.h.min(base.h.saturating_sub(y0)) {
        let s = (y * video.w) as usize * 4;
        let d = ((y + y0) * base.w + x0) as usize * 4;
        out[d..d + cols * 4].copy_from_slice(&video.rgba[s..s + cols * 4]);
    }
    Image {
        w: base.w,
        h: base.h,
        rgba: Arc::new(out),
    }
}

/// Where a video goes in a picture of `w` x `h` showing a slide of `aspect`
/// (letterboxed): pixel position and size.
pub fn place(rect: [f32; 4], aspect: f32, w: u32, h: u32) -> ((u32, u32), (u32, u32)) {
    let (fw, fh) = (w as f32, h as f32);
    let (sw, sh) = if fw / fh > aspect {
        (fh * aspect, fh)
    } else {
        (fw, fw / aspect)
    };
    let (ox, oy) = ((fw - sw) / 2.0, (fh - sh) / 2.0);
    let x = (ox + rect[0] * sw).round().max(0.0) as u32;
    let y = (oy + rect[1] * sh).round().max(0.0) as u32;
    let vw = ((rect[2] * sw).round() as u32)
        .min(w.saturating_sub(x))
        .max(2)
        & !1;
    let vh = ((rect[3] * sh).round() as u32)
        .min(h.saturating_sub(y))
        .max(2)
        & !1;
    ((x, y), (vw, vh))
}

/// Text for a time: `1:05`, `1:02:03`.
pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn reads_ffmpegs_description() {
        let t = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'x.mp4':\n  Duration: 00:01:02.50, start: 0.000000, bitrate: 220 kb/s\n  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p, 480x270, 25 fps\n  Stream #0:1[0x2](und): Audio: aac (LC), 48000 Hz, stereo, fltp, 48 kb/s\n";
        let p = parse_probe(t);
        assert_eq!(p.duration, Some(62.5));
        assert!(p.audio && p.video);
        assert!(!parse_probe("  Stream #0:0: Video: h264").audio);
    }

    #[test]
    fn places_videos_on_letterboxed_slides() {
        // 16:9 slide in a 16:9 picture: right half, lower half.
        assert_eq!(
            place([0.5, 0.5, 0.5, 0.5], 16.0 / 9.0, 1920, 1080),
            ((960, 540), (960, 540))
        );
        // 4:3 slide in 16:9: pillarboxed by 240 px.
        assert_eq!(
            place([0.0, 0.0, 1.0, 1.0], 4.0 / 3.0, 1920, 1080),
            ((240, 0), (1440, 1080))
        );
        assert_eq!(fmt_time(65.4), "1:05");
        assert_eq!(fmt_time(3723.0), "1:02:03");
    }

    #[test]
    fn pastes_into_a_copy() {
        let base = Image::black(4, 2);
        let v = Image {
            w: 2,
            h: 1,
            rgba: Arc::new(vec![9; 8]),
        };
        let out = paste(&base, &v, (2, 1));
        assert_eq!(out.rgba[(4 + 2) * 4], 9);
        assert_eq!(out.rgba[0], 0);
        assert_eq!(base.rgba[(4 + 2) * 4], 0, "base unchanged");
    }

    #[test]
    fn plays_the_demo_video_in_sync() {
        let Some(ff) = ffmpeg() else { return };
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/video.pptx");
        let file = std::env::temp_dir().join(format!("lantern-play-{}.mp4", std::process::id()));
        crate::pptx::extract(&p, "ppt/media/media1.mp4", &file).unwrap();
        let probe = probe(&ff, &file).unwrap();
        assert!(probe.audio && (probe.duration.unwrap() - 4.0).abs() < 0.1);
        let mut out = crate::audio::Output::default();
        out.apply(&crate::show::AudioCfg {
            enabled: false,
            ..Default::default()
        });
        let clip = Clip { file, probe };
        let mut pl = Playing::start(Start {
            ffmpeg: &ff,
            clip: &clip,
            w: 64,
            h: 36,
            fps: 25,
            looped: false,
            mixer: &out.mixer,
            gain: 1.0,
            from: 0.0,
            picture: true,
        })
        .unwrap();
        let t0 = Instant::now();
        let mut frames = 0;
        while !pl.ended() {
            assert!(t0.elapsed() < Duration::from_secs(10), "never ended");
            if pl.update() {
                frames += 1;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let took = t0.elapsed().as_secs_f64();
        assert!((3.8..6.0).contains(&took), "played in {took} s");
        assert!(frames > 80, "{frames} frames shown");
        assert_eq!(pl.frame().unwrap().w, 64);
        let _ = std::fs::remove_file(&clip.file);
    }

    /// Pause holds picture and clock, play goes on, seek jumps; also
    /// with the sound only.
    #[test]
    fn pauses_resumes_and_seeks() {
        let Some(ff) = ffmpeg() else { return };
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/video.pptx");
        let file = std::env::temp_dir().join(format!("lantern-seek-{}.mp4", std::process::id()));
        crate::pptx::extract(&p, "ppt/media/media1.mp4", &file).unwrap();
        let clip = Clip {
            probe: probe(&ff, &file).unwrap(),
            file,
        };
        let mut out = crate::audio::Output::default();
        out.apply(&crate::show::AudioCfg {
            enabled: false,
            ..Default::default()
        });
        let run = |pl: &mut Playing, ms: u64| {
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_millis(ms) {
                pl.update();
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        for picture in [true, false] {
            let mut pl = Playing::start(Start {
                ffmpeg: &ff,
                clip: &clip,
                w: 64,
                h: 36,
                fps: 25,
                looped: false,
                mixer: &out.mixer,
                gain: 1.0,
                from: 0.0,
                picture,
            })
            .unwrap();
            // Under load FFmpeg may take a moment to start: wait for it.
            let t0 = Instant::now();
            while pl.position() <= 0.0 {
                assert!(t0.elapsed() < Duration::from_secs(5), "never started");
                pl.update();
                std::thread::sleep(Duration::from_millis(5));
            }
            run(&mut pl, 600);
            let a = pl.position();
            assert!((0.3..1.2).contains(&a), "played {a}");
            pl.set_paused(true);
            run(&mut pl, 100);
            let held = pl.position();
            run(&mut pl, 500);
            assert!((pl.position() - held).abs() < 0.03, "paused stands");
            assert!(!pl.ended());
            pl.set_paused(false);
            run(&mut pl, 400);
            assert!(pl.position() > held + 0.25, "plays on");

            // Seek while paused: goes there, stays paused, shows that frame.
            pl.set_paused(true);
            pl.seek(3.0).unwrap();
            run(&mut pl, 300);
            assert!(pl.is_paused());
            assert!((pl.position() - 3.0).abs() < 0.05, "at {}", pl.position());
            if picture {
                assert!(pl.frame().is_some(), "the frame there is shown");
            }
            pl.set_paused(false);
            let t0 = Instant::now();
            while !pl.ended() {
                assert!(t0.elapsed() < Duration::from_secs(3), "end after seek");
                pl.update();
                std::thread::sleep(Duration::from_millis(5));
            }
            let took = t0.elapsed().as_secs_f64();
            assert!((0.6..2.0).contains(&took), "1 s left took {took}");

            // From the end back to the start.
            pl.seek(0.0).unwrap();
            run(&mut pl, 300);
            assert!(!pl.ended() && pl.position() < 0.6, "{}", pl.position());
        }
        let _ = std::fs::remove_file(&clip.file);
    }
}
