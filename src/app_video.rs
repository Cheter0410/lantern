//! Videos on slides in the window: preparing them while the presentation
//! loads, starting (by itself or with GO), what happens at the end, fading
//! the sound, and the Audio settings and the slide's Video settings.

use super::*;
use crate::audio;
use crate::media::{self, Clip, Playing};
use crate::pptx::{MediaSource, VideoInfo};
use crate::show::{VideoEnd, VideoStart};

/// A video drawn over its slide: texture and where it sits in the output
/// picture (pixels of an output of size `of`).
#[derive(Clone)]
pub(super) struct Overlay {
    pub tex: TextureHandle,
    pub image: Image,
    pub at: (u32, u32),
    pub size: (u32, u32),
    pub of: (u32, u32),
}

/// The video of the slide on the output.
pub(super) struct VideoNow {
    pub slide: usize,
    pub playing: Option<Playing>,
    /// Waits for GO.
    pub armed: bool,
    pub error: Option<String>,
    pub overlay: Option<Overlay>,
    /// Gain the sound is going to (follows blackout and the volume).
    pub gain: f32,
    /// The end was handled (next slide given once).
    pub end_done: bool,
    /// Where the timeline is being dragged to (seconds), until let go.
    pub scrub: Option<f64>,
}

/// How a slide's video plays: from the slide settings, else the presentation.
pub(super) struct Mode {
    pub auto: bool,
    pub end: VideoEnd,
    pub volume: f32,
}

pub(super) fn mode(info: &VideoInfo, cfg: Option<&show::VideoCfg>) -> Mode {
    let d = show::VideoCfg::default();
    let cfg = cfg.unwrap_or(&d);
    Mode {
        auto: match cfg.start {
            VideoStart::FromFile => info.auto,
            VideoStart::Auto => true,
            VideoStart::Go => false,
        },
        end: match cfg.end {
            VideoEnd::FromFile if info.looped => VideoEnd::Loop,
            VideoEnd::FromFile => VideoEnd::Hold,
            e => e,
        },
        volume: cfg.volume.unwrap_or(info.volume).clamp(0.0, 1.0),
    }
}

#[derive(Clone, Copy)]
enum Icon {
    Play,
    Pause,
    Restart,
}

/// Transport symbols, drawn (the fonts have no such glyphs).
fn icon(ui: &egui::Ui, which: Icon, c: egui::Pos2, color: Color32) {
    let p = ui.painter();
    let tri = |x: f32, w: f32, h: f32, right: bool| {
        let pts = if right {
            vec![pos2(x, c.y - h), pos2(x + w, c.y), pos2(x, c.y + h)]
        } else {
            vec![pos2(x + w, c.y - h), pos2(x, c.y), pos2(x + w, c.y + h)]
        };
        egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE)
    };
    match which {
        Icon::Play => {
            p.add(tri(c.x - 5.0, 11.0, 6.5, true));
        }
        Icon::Pause => {
            for dx in [-4.5, 1.5] {
                p.rect_filled(
                    Rect::from_min_size(pos2(c.x + dx, c.y - 6.5), Vec2::new(3.5, 13.0)),
                    1.0,
                    color,
                );
            }
        }
        Icon::Restart => {
            p.rect_filled(
                Rect::from_min_size(pos2(c.x - 7.0, c.y - 6.0), Vec2::new(2.5, 12.0)),
                0.5,
                color,
            );
            p.add(tri(c.x - 4.0, 10.0, 6.0, false));
        }
    }
}

/// Copy the videos out of the presentation and check them with FFmpeg
/// (in the loading thread). Keyed by slide (visible slides only).
pub(super) fn prepare_clips(
    path: &Path,
    info: &pptx::Info,
    status: &dyn Fn(String),
) -> Vec<(usize, Result<Clip, String>)> {
    let videos: Vec<(usize, VideoInfo)> = info
        .visible()
        .into_iter()
        .enumerate()
        .filter_map(|(i, s)| s.video.map(|v| (i, v)))
        .collect();
    if videos.is_empty() {
        return Vec::new();
    }
    let Some(ff) = media::ffmpeg() else {
        return videos
            .into_iter()
            .map(|(i, _)| {
                (
                    i,
                    Err("FFmpeg is missing – the slide shows the first frame".into()),
                )
            })
            .collect();
    };
    let key = office::fingerprint(path).unwrap_or_else(|_| "unknown".into());
    let dir = office::cache_dir().join("media").join(key);
    let total = videos.len();
    videos
        .into_iter()
        .enumerate()
        .map(|(n, (i, v))| {
            status(format!("Preparing videos… {}/{total}", n + 1));
            let r = (|| {
                let file = match &v.source {
                    MediaSource::Embedded(part) => {
                        let name = part.rsplit('/').next().unwrap_or("media");
                        let f = dir.join(name);
                        if !f.is_file() {
                            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                            pptx::extract(path, part, &f)?;
                        }
                        f
                    }
                    MediaSource::Linked(l) => {
                        let p = Path::new(l);
                        let f = if p.is_absolute() {
                            p.to_path_buf()
                        } else {
                            path.parent().unwrap_or(Path::new(".")).join(p)
                        };
                        if !f.is_file() {
                            return Err(format!("Linked video not found: {}", f.display()));
                        }
                        f
                    }
                };
                let probe = media::probe(&ff, &file)?;
                Ok(Clip { file, probe })
            })();
            (i, r)
        })
        .collect()
}

impl App {
    /// The video info of a slide (from the presentation).
    pub(super) fn video_info(&self, i: usize) -> Option<&VideoInfo> {
        self.slides.get(i).and_then(|s| s.video.as_ref())
    }

    pub(super) fn video_mode(&self, i: usize) -> Option<Mode> {
        let info = self.video_info(i)?;
        Some(mode(info, self.show.slides.get(i).map(|c| &c.video)))
    }

    /// GO starts the live slide's video when it waits for GO.
    pub(super) fn go_starts_video(&self) -> bool {
        self.video
            .as_ref()
            .is_some_and(|v| v.armed && Some(v.slide) == self.player.current)
    }

    /// Every frame: follow the slide on the output, play, fade, end.
    pub(super) fn update_video(&mut self, ctx: &egui::Context) {
        self.audio.apply(&self.show.audio);
        self.audio.mixer.set_tap(self.ndi.is_some());
        let fade = self.show.audio.fade.max(0.0);
        self.fading.retain(|p| !p.silent());
        let cur = self.player.current;

        // Slide changed: fade the old video's sound, keep its last frame for
        // the picture transition.
        if self.video.as_ref().is_some_and(|v| Some(v.slide) != cur) {
            if let Some(old) = self.video.take() {
                if let Some(p) = old.playing {
                    p.fade_out(fade);
                    self.fading.push(p);
                }
                self.prev_video = old.overlay;
            }
        }
        if self.player.mix() >= 1.0 {
            self.prev_video = None;
        }
        if self.video.is_none() {
            if let Some(i) = cur.filter(|i| self.clips.contains_key(i)) {
                let auto = self.video_mode(i).is_some_and(|m| m.auto);
                self.video = Some(VideoNow {
                    slide: i,
                    playing: None,
                    armed: !auto,
                    error: None,
                    overlay: None,
                    gain: 0.0,
                    end_done: false,
                    scrub: None,
                });
                if auto {
                    self.start_video();
                }
            }
        }

        let Some(i) = self.video.as_ref().map(|v| v.slide) else {
            return;
        };
        let Some(m) = self.video_mode(i) else { return };
        let target = if self.player.blackout { 0.0 } else { m.volume };
        let mut ended = false;
        let (at, size, of) = self.video_place(i);
        if let Some(v) = self.video.as_mut() {
            if let Some(p) = v.playing.as_mut() {
                if (v.gain - target).abs() > 1e-4 {
                    p.set_gain(target, fade);
                    v.gain = target;
                }
                if p.update() {
                    if let Some(f) = p.frame() {
                        let img = egui::ColorImage::from_rgba_unmultiplied(
                            [f.w as usize, f.h as usize],
                            &f.rgba,
                        );
                        match v.overlay.as_mut() {
                            Some(o) => {
                                o.tex.set(img, egui::TextureOptions::LINEAR);
                                o.image = f.clone();
                            }
                            None => {
                                let tex = ctx.load_texture(
                                    format!("video-{i}"),
                                    img,
                                    egui::TextureOptions::LINEAR,
                                );
                                v.overlay = Some(Overlay {
                                    tex,
                                    image: f.clone(),
                                    at,
                                    size,
                                    of,
                                });
                            }
                        }
                    }
                }
                ended = p.ended();
                ctx.request_repaint_after(Duration::from_millis(8));
            }
        }
        if ended && m.end == VideoEnd::Next {
            if let Some(v) = self.video.as_mut().filter(|v| !v.end_done) {
                v.end_done = true;
                self.go();
            }
        }
    }

    /// Pixel position and size of slide `i`'s video in the output picture.
    pub(super) fn video_place(&self, i: usize) -> ((u32, u32), (u32, u32), (u32, u32)) {
        let (w, h) = self.output_size();
        let aspect = self.deck.as_ref().map_or(16.0 / 9.0, |d| d.aspect);
        let rect = self.video_info(i).map_or([0.0, 0.0, 1.0, 1.0], |v| v.rect);
        let (at, size) = media::place(rect, aspect, w, h);
        (at, size, (w, h))
    }

    /// Start the live slide's video.
    pub(super) fn start_video(&mut self) {
        let Some(i) = self.video.as_ref().map(|v| v.slide) else {
            return;
        };
        let Some(m) = self.video_mode(i) else { return };
        let target = if self.player.blackout { 0.0 } else { m.volume };
        let (_, size, _) = self.video_place(i);
        let r = match (self.clips.get(&i), media::ffmpeg()) {
            (Some(Ok(clip)), Some(ff)) => Playing::start(media::Start {
                ffmpeg: &ff,
                clip,
                w: size.0,
                h: size.1,
                fps: self.show.output.fps.max(25),
                looped: m.end == VideoEnd::Loop,
                mixer: &self.audio.mixer,
                gain: target,
                from: 0.0,
                picture: !self.video_info(i).is_some_and(|v| v.audio_only),
            }),
            (Some(Err(e)), _) => Err(e.clone()),
            (_, None) => Err("FFmpeg is missing".into()),
            (None, _) => Err("No video".into()),
        };
        if let Some(v) = self.video.as_mut() {
            v.armed = false;
            v.end_done = false;
            match r {
                Ok(p) => {
                    v.playing = Some(p);
                    v.gain = target;
                    v.error = None;
                }
                Err(e) => v.error = Some(e),
            }
        }
    }

    /// Play, pause, go to a point of the live slide's video or sound.
    pub(super) fn video_command(&mut self, c: osc::VideoCmd) {
        use osc::VideoCmd as C;
        let Some(v) = self.video.as_mut() else { return };
        let Some(p) = v.playing.as_mut() else {
            // Not started yet: anything but pause starts it.
            if !matches!(c, C::Pause) {
                self.start_video();
                if let C::Seek(t) | C::Skip(t) = c {
                    if t > 0.0 {
                        self.video_command(C::Seek(t));
                    }
                }
            }
            return;
        };
        let pos = p.position();
        let at_end = p.ended();
        let r = match c {
            C::Play if at_end => p.seek(0.0),
            C::Play => {
                p.set_paused(false);
                Ok(())
            }
            C::Pause => {
                p.set_paused(true);
                Ok(())
            }
            C::Toggle if at_end => p.seek(0.0).map(|()| p.set_paused(false)),
            C::Toggle => {
                let paused = p.is_paused();
                p.set_paused(!paused);
                Ok(())
            }
            C::Restart => p.seek(0.0),
            C::Seek(t) => p.seek(t),
            C::Skip(d) => p.seek(pos + d),
        };
        if !matches!(c, C::Pause | C::Toggle) || at_end {
            v.end_done = false;
        }
        if let Err(e) = r {
            v.error = Some(e);
        }
    }

    /// The bar under the Live picture while the live slide has a video or
    /// sound: back to the start, play/pause, ±10 s, timeline with times.
    /// Always the same height, with or without a video, so GO and the
    /// buttons below never move.
    pub(super) fn video_transport(&mut self, ui: &mut egui::Ui) {
        let Some(v) = self.video.as_ref() else {
            ui.add_space(8.0);
            Frame::new()
                .fill(PANEL)
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let row = Vec2::new(ui.available_width(), 30.0);
                    ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
                        ui.set_min_height(30.0);
                        ui.label(
                            RichText::new("No video or sound on this slide")
                                .small()
                                .color(Color32::from_gray(90)),
                        );
                    });
                });
            return;
        };
        let slide = v.slide;
        let audio_only = self.video_info(slide).is_some_and(|i| i.audio_only);
        let duration = self
            .clips
            .get(&slide)
            .and_then(|c| c.as_ref().ok())
            .and_then(|c| c.probe.duration)
            .unwrap_or(0.0);
        let (pos, playing, paused, ended) = match &v.playing {
            Some(p) => (
                p.position(),
                !p.is_paused() && !p.ended(),
                p.is_paused(),
                p.ended(),
            ),
            None => (0.0, false, false, false),
        };
        let armed = v.armed;
        let error = v.error.clone();
        let mut cmd: Option<osc::VideoCmd> = None;
        ui.add_space(8.0);
        Frame::new()
            .fill(PANEL)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let row = Vec2::new(ui.available_width(), 30.0);
                ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
                    let h = 30.0;
                    let small = |text: &str, tip: &str, ui: &mut egui::Ui| {
                        ui.add(
                            egui::Button::new(RichText::new(text).size(14.0))
                                .min_size(Vec2::new(44.0, h))
                                .corner_radius(6),
                        )
                        .on_hover_text(tip)
                        .clicked()
                    };
                    ui.label(
                        RichText::new(match (audio_only, error.is_some()) {
                            (true, false) => "AUDIO",
                            (false, false) => "VIDEO",
                            (true, true) => "AUDIO – CAN'T PLAY",
                            (false, true) => "VIDEO – CAN'T PLAY",
                        })
                        .small()
                        .strong()
                        .color(if error.is_some() {
                            AMBER
                        } else if playing {
                            CURRENT
                        } else {
                            MUTED
                        }),
                    )
                    .on_hover_text(error.clone().unwrap_or_default());
                    let r = ui
                        .add(
                            egui::Button::new("")
                                .min_size(Vec2::new(44.0, h))
                                .corner_radius(6),
                        )
                        .on_hover_text("Back to the start");
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, "Back to the start")
                    });
                    icon(ui, Icon::Restart, r.rect.center(), TEXT);
                    if r.clicked() {
                        cmd = Some(osc::VideoCmd::Restart);
                    }
                    if small("\u{2212}10", "10 seconds back", ui) {
                        cmd = Some(osc::VideoCmd::Skip(-10.0));
                    }
                    let (label, tip, fill) = if playing {
                        ("Pause", "Pause (the picture stays) – K", None)
                    } else if armed {
                        ("Play", "Start (like GO) – K", Some(GO))
                    } else if ended {
                        ("Again", "Play again from the start – K", None)
                    } else if paused {
                        ("Resume", "Play on from here – K", Some(GO))
                    } else {
                        ("Play", "Play – K", Some(GO))
                    };
                    let ink = if fill.is_some() { Color32::WHITE } else { TEXT };
                    // Room on the left for the drawn symbol.
                    let mut b = egui::Button::new(
                        RichText::new(format!("     {label}"))
                            .size(14.0)
                            .strong()
                            .color(ink),
                    )
                    .min_size(Vec2::new(116.0, h))
                    .corner_radius(6);
                    if let Some(f) = fill {
                        b = b.fill(f);
                    }
                    let r = ui.add(b).on_hover_text(tip);
                    let text_w = 7.5 * label.len() as f32 + 14.0;
                    let c = pos2(r.rect.center().x - text_w / 2.0 - 2.0, r.rect.center().y);
                    icon(ui, if playing { Icon::Pause } else { Icon::Play }, c, ink);
                    if r.clicked() {
                        cmd = Some(osc::VideoCmd::Toggle);
                    }
                    if small("+10", "10 seconds on", ui) {
                        cmd = Some(osc::VideoCmd::Skip(10.0));
                    }

                    // Timeline: time so far | track | time left.
                    let scrub = self.video.as_ref().and_then(|v| v.scrub);
                    let shown = scrub.unwrap_or(pos);
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(media::fmt_time(shown))
                            .monospace()
                            .color(TEXT),
                    );
                    let right_w = 64.0;
                    let w = (ui.available_width() - right_w - 8.0).max(60.0);
                    let (r, resp) =
                        ui.allocate_exact_size(Vec2::new(w, h), Sense::click_and_drag());
                    let track = Rect::from_center_size(r.center(), Vec2::new(r.width(), 6.0));
                    let p = ui.painter();
                    p.rect_filled(track, 3.0, LINE);
                    let frac = if duration > 0.0 {
                        (shown / duration).clamp(0.0, 1.0) as f32
                    } else {
                        0.0
                    };
                    let done = Rect::from_min_max(
                        track.min,
                        pos2(track.left() + track.width() * frac, track.bottom()),
                    );
                    p.rect_filled(done, 3.0, if playing { CURRENT } else { MUTED });
                    let knob = pos2(done.right(), track.center().y);
                    let hot = resp.hovered() || resp.dragged();
                    p.circle_filled(knob, if hot { 8.0 } else { 6.0 }, TEXT);
                    let at = |x: f32| {
                        (f64::from(((x - track.left()) / track.width()).clamp(0.0, 1.0))) * duration
                    };
                    if let Some(x) = resp.interact_pointer_pos().map(|p| p.x) {
                        if resp.dragged() || resp.is_pointer_button_down_on() {
                            if let Some(v) = self.video.as_mut() {
                                v.scrub = Some(at(x));
                            }
                        }
                    }
                    if resp.drag_stopped() || resp.clicked() {
                        let to = self
                            .video
                            .as_mut()
                            .and_then(|v| v.scrub.take())
                            .or_else(|| resp.interact_pointer_pos().map(|p| at(p.x)));
                        if let Some(t) = to {
                            cmd = Some(osc::VideoCmd::Seek(t));
                        }
                    }
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Slider, duration > 0.0, "Timeline")
                    });
                    let resp = resp.on_hover_text("Click or drag to go to a point");
                    if let Some(hp) = resp.hover_pos() {
                        if duration > 0.0 && !resp.dragged() {
                            p.text(
                                pos2(hp.x, track.top() - 8.0),
                                egui::Align2::CENTER_BOTTOM,
                                media::fmt_time(at(hp.x)),
                                egui::FontId::monospace(11.0),
                                MUTED,
                            );
                        }
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("-{}", media::fmt_time((duration - shown).max(0.0))))
                            .monospace()
                            .color(MUTED),
                    );
                });
            });
        if let Some(c) = cmd {
            self.video_command(c);
        }
    }

    /// The video overlays for drawing: (slide before, live slide).
    pub(super) fn overlays(&self) -> (Option<&Overlay>, Option<&Overlay>) {
        (
            self.prev_video.as_ref(),
            self.video.as_ref().and_then(|v| v.overlay.as_ref()),
        )
    }

    /// Time left of the live video, and whether it plays.
    /// (`true`: started and not at its end, so the time counts down; it may be paused.)
    pub(super) fn video_left(&self) -> Option<(f64, bool)> {
        let v = self.video.as_ref()?;
        match &v.playing {
            Some(p) => Some((p.remaining().unwrap_or(0.0), !p.ended())),
            None => self
                .clips
                .get(&v.slide)
                .and_then(|c| c.as_ref().ok())
                .and_then(|c| c.probe.duration)
                .map(|d| (d, false)),
        }
    }

    /// The live video is paused.
    pub(super) fn video_paused(&self) -> bool {
        self.video
            .as_ref()
            .and_then(|v| v.playing.as_ref())
            .is_some_and(|p| p.is_paused() && !p.ended())
    }

    /// Settings › Audio.
    pub(super) fn settings_audio(&mut self, ui: &mut egui::Ui) {
        if self.audio_devices_at.elapsed() > Duration::from_secs(3) {
            self.audio_devices_at = Instant::now();
            self.audio_devices = audio::devices();
            self.audio_caps = audio::caps(&self.show.audio.device);
        }
        let a = &mut self.show.audio;
        let before = a.clone();
        ui.checkbox(&mut a.enabled, "Play the videos' sound on this computer");
        ui.add_space(6.0);
        ui.add_enabled_ui(a.enabled, |ui| {
            egui::Grid::new("audio")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Device").color(MUTED));
                    let shown = if a.device.is_empty() {
                        "System default".to_string()
                    } else {
                        a.device.clone()
                    };
                    egui::ComboBox::from_id_salt("audiodev")
                        .width(300.0)
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut a.device, String::new(), "System default");
                            for d in &self.audio_devices {
                                ui.selectable_value(&mut a.device, d.clone(), d);
                            }
                        });
                    ui.end_row();

                    let chans = self.audio_caps.as_ref().map_or(2, |c| c.channels).max(1);
                    ui.label(RichText::new("Channels").color(MUTED));
                    ui.horizontal(|ui| {
                        segmented(ui, &mut a.mono, &[(false, "Stereo"), (true, "Mono")]);
                        let step = if a.mono { 1 } else { 2 };
                        let label = |f: u16| {
                            if a.mono {
                                format!("{}", f + 1)
                            } else {
                                format!("{}–{}", f + 1, f + 2)
                            }
                        };
                        let shown = label(a.first_channel);
                        let mut first = a.first_channel;
                        egui::ComboBox::from_id_salt("audioch")
                            .width(90.0)
                            .selected_text(shown)
                            .show_ui(ui, |ui| {
                                let mut f = 0;
                                while f + step <= chans {
                                    ui.selectable_value(&mut first, f, label(f));
                                    f += step;
                                }
                            });
                        a.first_channel = first;
                        ui.label(RichText::new(format!("of {chans}")).small().color(MUTED));
                    });
                    ui.end_row();

                    ui.label(RichText::new("Sample rate").color(MUTED));
                    let rates = self
                        .audio_caps
                        .as_ref()
                        .map(|c| c.rates.clone())
                        .unwrap_or_else(|| audio::RATES.to_vec());
                    let dev_rate = self.audio_caps.as_ref().map_or(0, |c| c.default_rate);
                    let rate_text = |r: u32| {
                        if r == 0 {
                            if dev_rate > 0 {
                                format!("Device setting ({dev_rate} Hz)")
                            } else {
                                "Device setting".to_string()
                            }
                        } else {
                            format!("{r} Hz")
                        }
                    };
                    egui::ComboBox::from_id_salt("audiorate")
                        .width(200.0)
                        .selected_text(rate_text(a.sample_rate))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut a.sample_rate, 0, rate_text(0));
                            for r in rates {
                                ui.selectable_value(&mut a.sample_rate, r, rate_text(r));
                            }
                        });
                    ui.end_row();
                });
        });
        ui.add_space(4.0);
        egui::Grid::new("audio2")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Fade").color(MUTED));
                ui.add(
                    egui::Slider::new(&mut a.fade, 0.0..=5.0)
                        .suffix(" s")
                        .step_by(0.1),
                )
                .on_hover_text("Sound fades out on a slide change, blackout or clear");
                ui.end_row();
            });
        if before.device != a.device {
            self.audio_caps = audio::caps(&a.device);
            // Channels the new device doesn't have: back to the first ones.
            let chans = self.audio_caps.as_ref().map_or(2, |c| c.channels);
            if a.first_channel + if a.mono { 1 } else { 2 } > chans {
                a.first_channel = 0;
            }
        }
        if a.mono != before.mono && !a.mono && a.first_channel % 2 == 1 {
            a.first_channel -= 1;
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let (color, text) = if self.audio.ok {
                (OK, self.audio.status.clone())
            } else if self.show.audio.enabled {
                (DANGER, self.audio.status.clone())
            } else {
                (MUTED, self.audio.status.clone())
            };
            status(ui, &text, color);
        });
        ui.horizontal(|ui| {
            if ui
                .button("Test tone")
                .on_hover_text("One second of 1 kHz on the chosen channels")
                .clicked()
            {
                self.audio.test_tone();
            }
            if ui
                .button("Open again")
                .on_hover_text("After plugging a device in")
                .clicked()
            {
                self.audio.reopen();
                self.audio_devices_at = Instant::now() - Duration::from_secs(10);
            }
            meter(ui, self.meter, Vec2::new(160.0, 14.0));
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new(if self.show.output.ndi {
                "NDI sends the same sound with the picture."
            } else {
                "With NDI on, the sound goes along with the picture."
            })
            .small()
            .color(MUTED),
        );
    }

    /// The Video part of the slide settings.
    pub(super) fn inspector_video(&mut self, ui: &mut egui::Ui, i: usize) {
        let Some(info) = self.video_info(i).cloned() else {
            return;
        };
        let audio_only = self.video_info(i).is_some_and(|v| v.audio_only);
        section(ui, if audio_only { "Audio" } else { "Video" });
        let clip = self.clips.get(&i).cloned();
        match &clip {
            Some(Ok(c)) => {
                let d = c.probe.duration.map(media::fmt_time).unwrap_or_default();
                let sound = if c.probe.audio {
                    "with sound"
                } else {
                    "no sound"
                };
                ui.label(RichText::new(format!("{} · {d} · {sound}", info.name)).color(MUTED));
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e).color(DANGER));
            }
            None => {
                ui.label(RichText::new(&info.name).color(MUTED));
            }
        }
        let Some(cfg) = self.show.slides.get_mut(i) else {
            return;
        };
        let v = &mut cfg.video;
        ui.add_space(4.0);
        ui.label(RichText::new("Start").small().color(MUTED));
        segmented(
            ui,
            &mut v.start,
            &[
                (VideoStart::FromFile, "As in file"),
                (VideoStart::Auto, "Automatic"),
                (VideoStart::Go, "With GO"),
            ],
        );
        if v.start == VideoStart::FromFile {
            ui.label(
                RichText::new(if info.auto {
                    "In the file: starts by itself"
                } else {
                    "In the file: starts with a click (GO)"
                })
                .small()
                .color(MUTED),
            );
        }
        ui.add_space(4.0);
        ui.label(RichText::new("At the end").small().color(MUTED));
        segmented(
            ui,
            &mut v.end,
            &[
                (VideoEnd::FromFile, "As in file"),
                (VideoEnd::Hold, "Hold"),
                (VideoEnd::Loop, "Loop"),
                (VideoEnd::Next, "Next slide"),
            ],
        );
        if v.end == VideoEnd::FromFile {
            ui.label(
                RichText::new(if info.looped {
                    "In the file: loops"
                } else {
                    "In the file: stays on the last frame"
                })
                .small()
                .color(MUTED),
            );
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Volume").small().color(MUTED));
        ui.horizontal(|ui| {
            let mut pct = v.volume.unwrap_or(info.volume) * 100.0;
            if ui
                .add(
                    egui::Slider::new(&mut pct, 0.0..=100.0)
                        .suffix(" %")
                        .step_by(1.0),
                )
                .changed()
            {
                v.volume = Some(pct / 100.0);
            }
            if v.volume.is_some()
                && ui
                    .small_button("As in file")
                    .on_hover_text(format!("{:.0} %", info.volume * 100.0))
                    .clicked()
            {
                v.volume = None;
            }
        });
    }
}

/// A two-bar level meter (left, right).
pub(super) fn meter(ui: &mut egui::Ui, level: [f32; 2], size: Vec2) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    let gap = 2.0;
    let bar_h = (r.height() - gap) / 2.0;
    for (k, l) in level.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(r.left(), r.top() + k as f32 * (bar_h + gap)),
            Vec2::new(r.width(), bar_h),
        );
        p.rect_filled(row, 2.0, CARD);
        // dBFS from -60 to 0 across the bar.
        let db = 20.0 * l.max(1e-6).log10();
        let x = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
        let color = if db > -3.0 {
            DANGER
        } else if db > -12.0 {
            AMBER
        } else {
            OK
        };
        p.rect_filled(
            Rect::from_min_size(row.min, Vec2::new(row.width() * x, bar_h)),
            2.0,
            color,
        );
    }
    resp
}
