//! Sound out: one mixer plays the videos' sound (and a test tone) on the
//! chosen device and channels, fades voices in and out, measures the level
//! and hands a copy of the mix to NDI.
//!
//! The mixer works in stereo at the output's sample rate. Without a device
//! (switched off or failed) a clock thread keeps it running, so videos still
//! play in sync and NDI still gets the sound.

use crate::show::AudioCfg;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Sample rates offered in the settings (those the device supports).
pub const RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];
const FALLBACK_RATE: u32 = 48_000;

/// One sound playing: interleaved stereo samples written by a producer
/// (the video's decoder), a gain that ramps, and how much has been played.
pub struct Voice {
    buf: Mutex<VecDeque<f32>>,
    ramp: Mutex<Ramp>,
    played: AtomicU64,
    /// The producer has written everything.
    pub eof: AtomicBool,
    /// Fades to silence, then leaves the mixer.
    stopping: AtomicBool,
    gone: AtomicBool,
    /// Paused: fades out quickly, then takes nothing and the clock stands.
    paused: AtomicBool,
}

#[derive(Clone, Copy)]
struct Ramp {
    gain: f32,
    target: f32,
    /// Change per frame.
    step: f32,
    /// Pause fader (1 = playing, 0 = paused), multiplied with the gain.
    hold: f32,
}

/// Pause and resume fade within this, so there is no click.
const HOLD_SECS: f32 = 0.03;

impl Voice {
    pub fn new(gain: f32) -> Arc<Voice> {
        Arc::new(Voice {
            buf: Mutex::new(VecDeque::new()),
            ramp: Mutex::new(Ramp {
                gain,
                target: gain,
                step: 1.0,
                hold: 1.0,
            }),
            played: AtomicU64::new(0),
            eof: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            gone: AtomicBool::new(false),
            paused: AtomicBool::new(false),
        })
    }

    /// Pause (fades out within a few milliseconds) or play on. `now`: no
    /// fade (for a voice that has not played yet).
    pub fn set_paused(&self, paused: bool, now: bool) {
        self.paused.store(paused, Ordering::Relaxed);
        if now {
            if let Ok(mut r) = self.ramp.lock() {
                r.hold = if paused { 0.0 } else { 1.0 };
            }
        }
    }

    /// Add interleaved stereo samples.
    pub fn push(&self, samples: &[f32]) {
        if let Ok(mut b) = self.buf.lock() {
            b.extend(samples.iter().copied());
        }
    }

    /// Samples waiting to be played.
    pub fn queued(&self) -> usize {
        self.buf.lock().map(|b| b.len()).unwrap_or(0)
    }

    /// Frames played so far (the playback clock).
    pub fn played(&self) -> u64 {
        self.played.load(Ordering::Relaxed)
    }

    /// Ramp the gain to `target` within `secs` (at `rate` frames per second).
    pub fn set_gain(&self, target: f32, secs: f32, rate: u32) {
        if let Ok(mut r) = self.ramp.lock() {
            r.target = target.clamp(0.0, 2.0);
            let frames = (secs.max(0.0) * rate as f32).max(1.0);
            r.step = ((r.target - r.gain).abs() / frames).max(1e-7);
        }
    }

    /// Fade out within `secs`, then leave the mixer.
    pub fn stop(&self, secs: f32, rate: u32) {
        self.set_gain(0.0, secs, rate);
        self.stopping.store(true, Ordering::Relaxed);
    }

    /// Out of the mixer (faded out, or played to the end).
    pub fn is_gone(&self) -> bool {
        self.gone.load(Ordering::Relaxed)
    }
}

/// The stereo mix of all voices.
pub struct Mixer {
    rate: AtomicU32,
    voices: Mutex<Vec<Arc<Voice>>>,
    /// Copy of the mix for NDI (interleaved stereo), when wanted.
    tap: Mutex<Option<VecDeque<f32>>>,
    /// Peak since the last look, per side (f32 bits).
    peak: [AtomicU32; 2],
    /// From handing samples to the device to hearing them (µs, as the
    /// device says), and the last mix: when and how many frames. Together
    /// they say how far the sound heard is behind the sound played.
    latency_us: AtomicU32,
    last_frames: AtomicU32,
    last_at: Mutex<Option<Instant>>,
}

impl Mixer {
    fn new(rate: u32) -> Self {
        Mixer {
            rate: AtomicU32::new(rate),
            voices: Mutex::new(Vec::new()),
            tap: Mutex::new(None),
            peak: [AtomicU32::new(0), AtomicU32::new(0)],
            latency_us: AtomicU32::new(0),
            last_frames: AtomicU32::new(0),
            last_at: Mutex::new(None),
        }
    }

    fn set_latency(&self, d: Duration) {
        let us = d.as_micros().min(2_000_000) as u32;
        self.latency_us.store(us, Ordering::Relaxed);
    }

    /// Seconds the sound heard now is behind what the voices have played:
    /// the device's latency plus the part of the last buffer not heard yet.
    pub fn delay(&self) -> f64 {
        let latency = f64::from(self.latency_us.load(Ordering::Relaxed)) / 1e6;
        let buffer =
            f64::from(self.last_frames.load(Ordering::Relaxed)) / f64::from(self.rate().max(1));
        let since = self
            .last_at
            .lock()
            .ok()
            .and_then(|t| *t)
            .map_or(0.0, |t| t.elapsed().as_secs_f64());
        (latency + buffer - since).clamp(0.0, latency + buffer)
    }

    pub fn rate(&self) -> u32 {
        self.rate.load(Ordering::Relaxed)
    }

    pub fn add(&self, v: Arc<Voice>) {
        if let Ok(mut vs) = self.voices.lock() {
            vs.push(v);
        }
    }

    /// Keep a copy of the mix for NDI (on) or not (off).
    pub fn set_tap(&self, on: bool) {
        if let Ok(mut t) = self.tap.lock() {
            match (on, t.is_some()) {
                (true, false) => *t = Some(VecDeque::new()),
                (false, true) => *t = None,
                _ => {}
            }
        }
    }

    /// The mix since the last call (interleaved stereo).
    pub fn take_tap(&self) -> Vec<f32> {
        self.tap
            .lock()
            .ok()
            .and_then(|mut t| t.as_mut().map(|q| q.drain(..).collect()))
            .unwrap_or_default()
    }

    /// Peak level (0..1, left and right) since the last call.
    pub fn take_peak(&self) -> [f32; 2] {
        [0, 1].map(|i| f32::from_bits(self.peak[i].swap(0, Ordering::Relaxed)))
    }

    /// Mix `frames` stereo frames into `out` (resized to `frames * 2`).
    pub fn mix(&self, frames: usize, out: &mut Vec<f32>) {
        self.last_frames.store(frames as u32, Ordering::Relaxed);
        if let Ok(mut t) = self.last_at.lock() {
            *t = Some(Instant::now());
        }
        out.clear();
        out.resize(frames * 2, 0.0);
        if let Ok(mut vs) = self.voices.lock() {
            for v in vs.iter() {
                let (Ok(mut buf), Ok(mut ramp)) = (v.buf.lock(), v.ramp.lock()) else {
                    continue;
                };
                let paused = v.paused.load(Ordering::Relaxed);
                let hold_to = if paused { 0.0 } else { 1.0 };
                let hold_step = 1.0 / (HOLD_SECS * self.rate() as f32).max(1.0);
                let avail = (buf.len() / 2).min(frames);
                let mut n = 0;
                while n < avail {
                    if paused && ramp.hold <= 0.0 {
                        break;
                    }
                    let (l, r) = (
                        buf.pop_front().unwrap_or(0.0),
                        buf.pop_front().unwrap_or(0.0),
                    );
                    let g = ramp.gain * ramp.hold;
                    out[n * 2] += l * g;
                    out[n * 2 + 1] += r * g;
                    if ramp.gain < ramp.target {
                        ramp.gain = (ramp.gain + ramp.step).min(ramp.target);
                    } else if ramp.gain > ramp.target {
                        ramp.gain = (ramp.gain - ramp.step).max(ramp.target);
                    }
                    if ramp.hold < hold_to {
                        ramp.hold = (ramp.hold + hold_step).min(1.0);
                    } else if ramp.hold > hold_to {
                        ramp.hold = (ramp.hold - hold_step).max(0.0);
                    }
                    n += 1;
                }
                // The clock only moves on with sound actually played (or
                // after the producer is done, so a short track can't hold it).
                let moved = if n == 0 && !paused && v.eof.load(Ordering::Relaxed) {
                    frames
                } else {
                    n
                };
                v.played.fetch_add(moved as u64, Ordering::Relaxed);
                let silent =
                    (ramp.gain <= 0.0 && ramp.target <= 0.0) || (paused && ramp.hold <= 0.0);
                if (v.stopping.load(Ordering::Relaxed) && silent)
                    || (v.eof.load(Ordering::Relaxed) && buf.is_empty())
                {
                    v.gone.store(true, Ordering::Relaxed);
                }
            }
            vs.retain(|v| !v.is_gone());
        }
        for side in 0..2 {
            let p = out
                .iter()
                .skip(side)
                .step_by(2)
                .fold(0.0f32, |p, s| p.max(s.abs()));
            // Non-negative floats order like their bits.
            self.peak[side].fetch_max(p.to_bits(), Ordering::Relaxed);
        }
        if let Ok(mut t) = self.tap.lock() {
            if let Some(q) = t.as_mut() {
                q.extend(out.iter().copied());
                // NDI not reading: keep at most a second.
                let max = self.rate() as usize * 2;
                if q.len() > max {
                    let extra = q.len() - max;
                    q.drain(..extra);
                }
            }
        }
    }
}

/// Output devices by name.
pub fn devices() -> Vec<String> {
    cpal::default_host()
        .output_devices()
        .map(|ds| ds.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// What a device offers: most channels, sample rates (of [`RATES`]), its own rate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Caps {
    pub channels: u16,
    pub rates: Vec<u32>,
    pub default_rate: u32,
}

fn find_device(name: &str) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if name.is_empty() {
        host.default_output_device()
    } else {
        host.output_devices()
            .ok()?
            .find(|d| d.name().ok().as_deref() == Some(name))
    }
}

pub fn caps(name: &str) -> Option<Caps> {
    let dev = find_device(name)?;
    let default_rate = dev.default_output_config().ok()?.sample_rate().0;
    let ranges: Vec<_> = dev.supported_output_configs().ok()?.collect();
    let channels = ranges.iter().map(|r| r.channels()).max().unwrap_or(2);
    let rates = RATES
        .iter()
        .copied()
        .filter(|r| {
            ranges
                .iter()
                .any(|c| c.min_sample_rate().0 <= *r && *r <= c.max_sample_rate().0)
        })
        .collect();
    Some(Caps {
        channels,
        rates,
        default_rate,
    })
}

/// The sound output: the mixer, and the device stream (or the clock thread
/// standing in for it).
pub struct Output {
    pub mixer: Arc<Mixer>,
    cfg: Option<AudioCfg>,
    stream: Option<cpal::Stream>,
    ticker: Option<Arc<AtomicBool>>,
    /// What is open, or what went wrong.
    pub status: String,
    pub ok: bool,
}

impl Default for Output {
    fn default() -> Self {
        Output {
            mixer: Arc::new(Mixer::new(FALLBACK_RATE)),
            cfg: None,
            stream: None,
            ticker: None,
            status: "Off".into(),
            ok: false,
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.stop_ticker();
    }
}

impl Output {
    /// Open the device the settings ask for (again only when they changed).
    pub fn apply(&mut self, cfg: &AudioCfg) {
        if self.cfg.as_ref() == Some(cfg) {
            return;
        }
        self.cfg = Some(cfg.clone());
        self.stream = None;
        self.stop_ticker();
        let r = if cfg.enabled {
            self.open(cfg)
        } else {
            Err("Off – videos play without sound here (NDI still gets it)".into())
        };
        match r {
            Ok(s) => {
                self.status = s;
                self.ok = true;
            }
            Err(e) => {
                self.status = e;
                self.ok = false;
                let rate = if cfg.sample_rate > 0 {
                    cfg.sample_rate
                } else {
                    FALLBACK_RATE
                };
                self.mixer.rate.store(rate, Ordering::Relaxed);
                self.start_ticker();
            }
        }
    }

    /// Open again (e.g. after a device came back).
    pub fn reopen(&mut self) {
        if let Some(c) = self.cfg.take() {
            self.apply(&c);
        }
    }

    fn stop_ticker(&mut self) {
        if let Some(t) = self.ticker.take() {
            t.store(true, Ordering::Relaxed);
        }
    }

    /// No device: mix by the wall clock into nothing.
    fn start_ticker(&mut self) {
        let stop = Arc::new(AtomicBool::new(false));
        let (mixer, stop2) = (self.mixer.clone(), stop.clone());
        let _ = std::thread::Builder::new()
            .name("lantern-audio-clock".into())
            .spawn(move || {
                let start = Instant::now();
                let mut done: u64 = 0;
                let mut buf = Vec::new();
                while !stop2.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(10));
                    let due = (start.elapsed().as_secs_f64() * f64::from(mixer.rate())) as u64;
                    let n = due.saturating_sub(done) as usize;
                    if n > 0 {
                        mixer.mix(n, &mut buf);
                        done += n as u64;
                    }
                }
            });
        self.ticker = Some(stop);
    }

    fn open(&mut self, cfg: &AudioCfg) -> Result<String, String> {
        let dev = find_device(&cfg.device).ok_or_else(|| {
            if cfg.device.is_empty() {
                "No audio output found".to_string()
            } else {
                format!("\"{}\" not found", cfg.device)
            }
        })?;
        let name = dev.name().unwrap_or_default();
        let def = dev
            .default_output_config()
            .map_err(|e| format!("{name}: {e}"))?;
        let rate = if cfg.sample_rate > 0 {
            cfg.sample_rate
        } else {
            def.sample_rate().0
        };
        // The config with the most channels at that rate (so channels 3/4
        // and up can be reached), preferring float samples.
        let range = dev
            .supported_output_configs()
            .map_err(|e| format!("{name}: {e}"))?
            .filter(|c| c.min_sample_rate().0 <= rate && rate <= c.max_sample_rate().0)
            .max_by_key(|c| (c.channels(), c.sample_format() == cpal::SampleFormat::F32))
            .ok_or_else(|| format!("{name} does not play {rate} Hz"))?;
        let config = range.with_sample_rate(cpal::SampleRate(rate));
        let channels = config.channels();
        let first = cfg.first_channel.min(channels.saturating_sub(1));
        let wanted = if cfg.mono { 1 } else { 2 };
        if first + wanted > channels {
            return Err(format!(
                "{name} has {channels} channels – channel {} is not there",
                cfg.first_channel + wanted
            ));
        }
        self.mixer.rate.store(rate, Ordering::Relaxed);
        let route = Route {
            channels: usize::from(channels),
            first: usize::from(first),
            mono: cfg.mono,
        };
        let sc: cpal::StreamConfig = config.clone().into();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&dev, &sc, self.mixer.clone(), route),
            cpal::SampleFormat::I16 => build::<i16>(&dev, &sc, self.mixer.clone(), route),
            cpal::SampleFormat::I32 => build::<i32>(&dev, &sc, self.mixer.clone(), route),
            cpal::SampleFormat::U16 => build::<u16>(&dev, &sc, self.mixer.clone(), route),
            f => return Err(format!("{name}: sample format {f} is not supported")),
        }
        .map_err(|e| format!("{name}: {e}"))?;
        stream.play().map_err(|e| format!("{name}: {e}"))?;
        self.stream = Some(stream);
        let chans = if cfg.mono {
            format!("channel {}", first + 1)
        } else {
            format!("channels {}–{}", first + 1, first + 2)
        };
        Ok(format!("{name} · {chans} · {rate} Hz"))
    }

    /// Two seconds of pink noise on the chosen channels (the same on both).
    pub fn test_tone(&self) {
        let rate = self.mixer.rate();
        let v = Voice::new(1.0);
        v.push(&pink_noise(rate, 2.0));
        v.eof.store(true, Ordering::Relaxed);
        self.mixer.add(v);
    }
}

/// Pink noise (equal energy per octave), about -18 dBFS RMS, interleaved
/// stereo with the same signal on both sides; short fades at the ends.
pub fn pink_noise(rate: u32, secs: f32) -> Vec<f32> {
    let n = (rate as f32 * secs) as usize;
    let mut seed: u32 = 0x1234_5678;
    let mut white = move || {
        // xorshift: fast, the same every time.
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    // Paul Kellet's filter: white noise to pink within ±0.05 dB.
    let mut b = [0.0f32; 7];
    let mut out = Vec::with_capacity(n * 2);
    let fade = (rate as f32 * 0.02).max(1.0);
    for i in 0..n {
        let w = white();
        b[0] = 0.99886 * b[0] + w * 0.0555179;
        b[1] = 0.99332 * b[1] + w * 0.0750759;
        b[2] = 0.96900 * b[2] + w * 0.153_852;
        b[3] = 0.86650 * b[3] + w * 0.3104856;
        b[4] = 0.55000 * b[4] + w * 0.5329522;
        b[5] = -0.7616 * b[5] - w * 0.0168980;
        let pink = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
        b[6] = w * 0.115926;
        let env = (i as f32 / fade).min((n - i) as f32 / fade).min(1.0);
        // The filter's sum runs at about 1.8 RMS; 0.07 makes about -18 dBFS.
        let x = pink * 0.07 * env;
        out.push(x);
        out.push(x);
    }
    out
}

#[derive(Clone, Copy)]
struct Route {
    channels: usize,
    first: usize,
    mono: bool,
}

fn build<T>(
    dev: &cpal::Device,
    sc: &cpal::StreamConfig,
    mixer: Arc<Mixer>,
    route: Route,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let mut mix = Vec::new();
    dev.build_output_stream(
        sc,
        move |out: &mut [T], info: &cpal::OutputCallbackInfo| {
            let ts = info.timestamp();
            if let Some(d) = ts.playback.duration_since(&ts.callback) {
                mixer.set_latency(d);
            }
            let frames = out.len() / route.channels.max(1);
            mixer.mix(frames, &mut mix);
            for (f, frame) in out.chunks_mut(route.channels).enumerate() {
                for s in frame.iter_mut() {
                    *s = T::from_sample(0.0);
                }
                let (l, r) = (mix[f * 2], mix[f * 2 + 1]);
                if route.mono {
                    frame[route.first] = T::from_sample(((l + r) * 0.5).clamp(-1.0, 1.0));
                } else {
                    frame[route.first] = T::from_sample(l.clamp(-1.0, 1.0));
                    frame[route.first + 1] = T::from_sample(r.clamp(-1.0, 1.0));
                }
            }
        },
        |e| eprintln!("audio out: {e}"),
        None,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn mixes_ramps_and_counts_played_frames() {
        let m = Mixer::new(100);
        let v = Voice::new(1.0);
        v.push(&[0.5; 40]); // 20 frames
        m.add(v.clone());
        m.set_tap(true);
        let mut out = Vec::new();
        m.mix(10, &mut out);
        assert_eq!(out.len(), 20);
        assert!((out[0] - 0.5).abs() < 1e-6);
        assert_eq!(v.played(), 10);
        assert!((m.take_peak()[0] - 0.5).abs() < 1e-6);
        assert_eq!(m.take_tap().len(), 20);
        // Fade out over 0.1 s = 10 frames: silent at the end, then gone.
        v.stop(0.1, 100);
        m.mix(10, &mut out);
        assert!(out[0] > 0.4 && out[18].abs() < 0.06, "{out:?}");
        m.mix(1, &mut out);
        assert!(v.is_gone());
        assert!(m.voices.lock().unwrap().is_empty());
    }

    #[test]
    fn pink_noise_is_pink_and_not_too_loud() {
        let rate = 48_000;
        let s = pink_noise(rate, 2.0);
        assert_eq!(s.len(), 2 * 96_000);
        let left: Vec<f32> = s.iter().step_by(2).copied().collect();
        let rms = (left.iter().map(|x| x * x).sum::<f32>() / left.len() as f32).sqrt();
        let db = 20.0 * rms.log10();
        assert!((-22.0..-14.0).contains(&db), "{db} dBFS");
        assert!(left.iter().all(|x| x.abs() < 1.0), "no clipping");
        assert!(left[0].abs() < 1e-3, "fades in");
        // Pink: a lot more energy low than high. Energy of the signal vs.
        // its first difference (a high-pass) – white noise gives ~2x, pink far less.
        let e = left.iter().map(|x| x * x).sum::<f32>();
        let d = left.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>();
        assert!(d / e < 0.5, "high share {}", d / e);
    }

    #[test]
    fn delay_counts_down_within_a_buffer() {
        let m = Mixer::new(1000);
        m.set_latency(Duration::from_millis(20));
        let mut out = Vec::new();
        m.mix(10, &mut out); // 10 ms buffer
        let d = m.delay();
        assert!((0.025..=0.030).contains(&d), "{d}");
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(m.delay(), 0.0, "all heard by now");
    }

    #[test]
    fn clock_runs_on_after_the_sound_ended() {
        let m = Mixer::new(100);
        let v = Voice::new(1.0);
        v.push(&[0.1; 4]);
        v.eof.store(true, Ordering::Relaxed);
        m.add(v.clone());
        let mut out = Vec::new();
        m.mix(10, &mut out);
        assert_eq!(v.played(), 2);
        assert!(v.is_gone());
    }
}
