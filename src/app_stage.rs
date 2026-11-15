//! The stage display: a screen for the speaker or stage manager on its own
//! display – time, slide and countdown across the top, a message from the
//! operator, the slide now and the next one, and the notes in large type.

use super::*;
use crate::osc::StageCmd;
use crate::stage::{fmt_countdown, Message, Timer};

pub(super) struct StageNow {
    pub(super) timer: Timer,
    pub(super) message: Option<Message>,
    /// The message being typed.
    pub(super) draft: String,
    started: Option<Instant>,
    /// Tests: show the stage display in the window instead.
    #[cfg(test)]
    pub(super) preview: bool,
}

impl Default for StageNow {
    fn default() -> Self {
        StageNow {
            timer: Timer::new(crate::show::StageCfg::default().timer_minutes),
            message: None,
            draft: String::new(),
            started: None,
            #[cfg(test)]
            preview: false,
        }
    }
}

/// Everything the stage display shows, taken out of the app for one frame
/// (the stage window draws it in its own closure).
pub(super) struct StageScene {
    cur: Option<TextureHandle>,
    cur_video: Option<video::Overlay>,
    next: Option<TextureHandle>,
    blackout: bool,
    aspect: f32,
    slide: String,
    on_slide: String,
    next_label: String,
    next_title: String,
    notes: String,
    clock: String,
    timecode: Option<String>,
    timer: String,
    timer_color: Color32,
    timer_state: &'static str,
    message: Option<(String, f32)>,
    show_next: bool,
    show_notes: bool,
    notes_size: f32,
}

impl StageScene {
    /// The countdown as text and its colour.
    pub(super) fn timer_parts(&self) -> (String, Color32) {
        (self.timer.clone(), self.timer_color)
    }
}

const STAGE_BG: Color32 = Color32::from_rgb(6, 7, 9);

impl App {
    pub(super) fn stage_command(&mut self, c: StageCmd) {
        let minutes = self.show.stage.timer_minutes;
        let s = &mut self.stage;
        match c {
            StageCmd::Message(t) => s.message = Message::new(&t),
            StageCmd::ClearMessage => s.message = None,
            StageCmd::TimerStart => s.timer.start(),
            StageCmd::TimerPause => s.timer.pause(),
            StageCmd::TimerToggle => s.timer.toggle(),
            StageCmd::TimerReset(m) => {
                if let Some(m) = m {
                    self.show.stage.timer_minutes = m.clamp(1, 600);
                }
                s.timer.reset(m.unwrap_or(minutes));
            }
        }
    }

    pub(super) fn stage_scene(&self) -> StageScene {
        let cfg = &self.show.stage;
        let pic = |p: Option<usize>| {
            self.tex(p)
                .or_else(|| p.and_then(|p| self.thumbs.get(&p)))
                .cloned()
        };
        let total = self.slides.len();
        let next = self.next_index();
        let (w, h) = self.output_size();
        let left = self.stage.timer.remaining();
        let (timer_color, timer_state) = if self.stage.timer.untouched() {
            (MUTED, "ready")
        } else if left <= 0.0 {
            (DANGER, "over time")
        } else if left <= f64::from(cfg.warn_minutes) * 60.0 {
            (
                AMBER,
                if self.stage.timer.running() {
                    "left"
                } else {
                    "paused"
                },
            )
        } else {
            (
                TEXT,
                if self.stage.timer.running() {
                    "left"
                } else {
                    "paused"
                },
            )
        };
        StageScene {
            cur: pic(self.player.current),
            cur_video: self.overlays().1.cloned(),
            next: pic(next),
            blackout: self.player.blackout,
            aspect: w as f32 / h.max(1) as f32,
            slide: match self.player.current {
                Some(c) => format!("{} / {total}", c + 1),
                None => format!("– / {total}"),
            },
            on_slide: match self.player.current {
                Some(_) => {
                    let s = self.player.since.elapsed().as_secs();
                    format!("on slide {}:{:02}", s / 60, s % 60)
                }
                None => "not started".into(),
            },
            next_label: match next {
                Some(n) => format!("Slide {}", n + 1),
                None => "End".into(),
            },
            next_title: next
                .and_then(|n| self.slides.get(n))
                .map(|s| s.title.clone())
                .unwrap_or_default(),
            notes: self
                .player
                .current
                .and_then(|c| self.slides.get(c))
                .map(|s| s.notes.clone())
                .unwrap_or_default(),
            clock: clock(),
            timecode: (self.show.timecode.source != TcSource::Off).then(|| {
                self.tc
                    .reading()
                    .map_or("--:--:--:--".into(), |r| show::format_tc(r.secs, r.fps))
            }),
            timer: fmt_countdown(left),
            timer_color,
            timer_state,
            message: self.stage.message.as_ref().map(|m| {
                let t = m.at.elapsed().as_secs_f32();
                let pulse = if m.flashing() {
                    0.5 + 0.5 * (t * std::f32::consts::TAU * 1.2).cos()
                } else {
                    0.0
                };
                (m.text.clone(), pulse)
            }),
            show_next: cfg.next,
            show_notes: cfg.notes,
            notes_size: cfg.notes_size,
        }
    }

    /// The stage display on its display (when one is chosen and there).
    pub(super) fn stage_window(&mut self, ctx: &egui::Context) {
        // A countdown never touched follows the length in the settings.
        if self.stage.timer.untouched()
            && self.stage.timer.length != f64::from(self.show.stage.timer_minutes) * 60.0
        {
            self.stage.timer.reset(self.show.stage.timer_minutes);
        }
        let Some(m) = self.show.stage.display else {
            self.stage.started = None;
            return;
        };
        let Some(d) = self.displays.get(m).cloned() else {
            return;
        };
        let since = *self.stage.started.get_or_insert_with(Instant::now);
        if since.elapsed() < Duration::from_millis(300) {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        let zoom = ctx.zoom_factor().max(0.1);
        let borderless =
            cfg!(target_os = "macos") || std::env::var_os("LANTERN_BORDERLESS").is_some();
        let mut b = egui::ViewportBuilder::default().with_title("Lantern Stage");
        if borderless {
            b = b
                .with_decorations(false)
                .with_resizable(false)
                .with_position([d.pos[0] / zoom, d.pos[1] / zoom])
                .with_inner_size([d.size[0] / zoom, d.size[1] / zoom])
                .with_window_level(egui::WindowLevel::AlwaysOnTop);
        } else {
            b = b.with_fullscreen(true).with_monitor(m);
        }
        let scene = self.stage_scene();
        let mut closed = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of(("lantern-stage", m)),
            b,
            |ui, _| {
                let rect = ui.ctx().content_rect();
                paint_stage(ui, rect, &scene);
                ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                if ui.ctx().input(|i| i.viewport().close_requested()) {
                    closed = true;
                }
            },
        );
        if borderless {
            crate::platform::raise_output_windows(
                "Lantern Stage",
                [d.pos[0], d.pos[1], d.size[0], d.size[1]],
            );
        }
        if closed {
            self.show.stage.display = None;
        }
        // The clocks move every second; a message flashes.
        let flashing = self.stage.message.as_ref().is_some_and(Message::flashing);
        ctx.request_repaint_after(Duration::from_millis(if flashing { 33 } else { 250 }));
    }

    /// Under the notes, while the stage display is on: the message to the
    /// stage and the countdown. Works in Live too (it runs the show).
    pub(super) fn stage_controls(&mut self, ui: &mut egui::Ui) {
        let minutes = self.show.stage.timer_minutes;
        Frame::new()
            .fill(PANEL)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("STAGE").small().strong().color(MUTED));
                    ui.add_space(6.0);
                    let right = 300.0;
                    let field = (ui.available_width() - right - 150.0).max(120.0);
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.stage.draft)
                            .hint_text("Message to the stage…")
                            .desired_width(field),
                    );
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if ui
                        .add_enabled(
                            !self.stage.draft.trim().is_empty(),
                            egui::Button::new("Send"),
                        )
                        .on_hover_text("Shows on the stage display until cleared (Enter)")
                        .clicked()
                        || enter
                    {
                        if let Some(m) = Message::new(&self.stage.draft) {
                            self.stage.message = Some(m);
                            self.stage.draft.clear();
                        }
                    }
                    if self.stage.message.is_some()
                        && ui
                            .button(RichText::new("Clear message").color(AMBER))
                            .on_hover_text(
                                self.stage
                                    .message
                                    .as_ref()
                                    .map(|m| format!("On stage: \"{}\"", m.text))
                                    .unwrap_or_default(),
                            )
                            .clicked()
                    {
                        self.stage.message = None;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .button("Reset")
                            .on_hover_text(format!("Back to {minutes}:00"))
                            .clicked()
                        {
                            self.stage.timer.reset(minutes);
                        }
                        let running = self.stage.timer.running();
                        if ui
                            .button(if running { "Pause" } else { "Start" })
                            .on_hover_text("The countdown on the stage display")
                            .clicked()
                        {
                            self.stage.timer.toggle();
                        }
                        let s = self.stage_scene();
                        ui.label(
                            RichText::new(&s.timer)
                                .monospace()
                                .size(17.0)
                                .strong()
                                .color(if s.timer_color == TEXT {
                                    TEXT
                                } else {
                                    s.timer_color
                                }),
                        );
                        ui.label(RichText::new("COUNTDOWN").small().strong().color(MUTED));
                    });
                });
            });
    }

    /// Settings › Stage.
    pub(super) fn settings_stage(&mut self, ui: &mut egui::Ui) {
        let label = |d: Option<usize>, ds: &[Disp]| match d {
            None => "Off".to_string(),
            Some(i) => match ds.get(i) {
                Some(x) => format!("{} – {} ({}×{})", i + 1, x.name, x.px[0], x.px[1]),
                None => format!("{} (not connected)", i + 1),
            },
        };
        let output = self.show.output.display;
        let c = &mut self.show.stage;
        egui::Grid::new("stage")
            .num_columns(2)
            .spacing([14.0, 10.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Display").color(MUTED));
                egui::ComboBox::from_id_salt("stagedisp")
                    .width(320.0)
                    .selected_text(label(c.display, &self.displays))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut c.display, None, "Off");
                        for i in 0..self.displays.len() {
                            let mut text = label(Some(i), &self.displays);
                            if output == Some(i) {
                                text.push_str(" · the output");
                            }
                            ui.selectable_value(&mut c.display, Some(i), text);
                        }
                    });
                ui.end_row();
                ui.label(RichText::new("Show").color(MUTED));
                ui.horizontal(|ui| {
                    ui.checkbox(&mut c.next, "Next slide");
                    ui.checkbox(&mut c.notes, "Notes");
                });
                ui.end_row();
                ui.label(RichText::new("Notes size").color(MUTED));
                ui.add(egui::Slider::new(&mut c.notes_size, 24.0..=90.0).suffix(" pt"));
                ui.end_row();
                ui.label(RichText::new("Countdown").color(MUTED));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut c.timer_minutes)
                            .range(1..=600)
                            .suffix(" min"),
                    );
                    ui.label(RichText::new("amber from").color(MUTED));
                    ui.add(
                        egui::DragValue::new(&mut c.warn_minutes)
                            .range(0..=120)
                            .suffix(" min"),
                    );
                    ui.label(RichText::new("left").color(MUTED));
                });
                ui.end_row();
            });
        ui.add_space(10.0);
        let w = ui.available_width().min(470.0);
        let (r, _) = ui.allocate_exact_size(Vec2::new(w, w * 9.0 / 16.0), Sense::hover());
        let scene = self.stage_scene();
        paint_stage(ui, r, &scene);
        ui.painter()
            .rect_stroke(r, 4.0, Stroke::new(1.0, LINE), egui::StrokeKind::Outside);
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "For the speaker or stage manager. Messages and the countdown are set under the \
                 notes while it is on, and over OSC: /lantern/stage/message \"text\", \
                 /lantern/stage/clear, /lantern/timer/start, /timer/pause, /timer/toggle, \
                 /timer/reset [minutes].",
            )
            .small()
            .color(MUTED),
        );
    }
}

/// Draw the stage display into `rect` (sizes follow its height: the same
/// picture full screen and as the small preview).
pub(super) fn paint_stage(ui: &egui::Ui, rect: Rect, s: &StageScene) {
    let p = ui.painter().with_clip_rect(rect);
    let k = rect.height() / 1080.0;
    let pad = 28.0 * k;
    let gap = 20.0 * k;
    p.rect_filled(rect, 0.0, STAGE_BG);
    let inner = rect.shrink(pad);
    let mono = |size: f32| egui::FontId::monospace(size * k);
    let prop = |size: f32| egui::FontId::proportional(size * k);
    let fits = |text: &str, font: egui::FontId, w: f32| {
        // Smaller until it fits the width.
        let mut f = font;
        loop {
            let g = p.layout_no_wrap(text.to_string(), f.clone(), TEXT);
            if g.size().x <= w || f.size < 8.0 {
                return f;
            }
            f.size *= 0.9;
        }
    };

    // Top: time, slide, countdown (and timecode when it is on).
    let top_h = 150.0 * k;
    let mut cells: Vec<(&str, String, Color32, String)> = vec![
        ("TIME", s.clock.clone(), TEXT, String::new()),
        ("SLIDE", s.slide.clone(), CURRENT, s.on_slide.clone()),
    ];
    if let Some(tc) = &s.timecode {
        cells.push(("TIMECODE", tc.clone(), TC, String::new()));
    }
    cells.push((
        "COUNTDOWN",
        s.timer.clone(),
        s.timer_color,
        s.timer_state.to_string(),
    ));
    let n = cells.len() as f32;
    let cw = (inner.width() - gap * (n - 1.0)) / n;
    for (i, (cap, value, color, sub)) in cells.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(inner.left() + i as f32 * (cw + gap), inner.top()),
            Vec2::new(cw, top_h),
        );
        p.rect_filled(r, 12.0 * k, CLOCK_BG);
        p.text(
            r.left_top() + Vec2::new(20.0 * k, 16.0 * k),
            egui::Align2::LEFT_TOP,
            *cap,
            prop(20.0),
            MUTED,
        );
        if !sub.is_empty() {
            p.text(
                r.right_top() + Vec2::new(-20.0 * k, 16.0 * k),
                egui::Align2::RIGHT_TOP,
                sub,
                prop(20.0),
                MUTED,
            );
        }
        let font = fits(value, mono(80.0), cw - 40.0 * k);
        p.text(
            pos2(r.center().x, r.bottom() - 58.0 * k),
            egui::Align2::CENTER_CENTER,
            value,
            font,
            *color,
        );
    }
    let mut y = inner.top() + top_h + gap;

    // A message from the operator: across, big, flashing at first.
    if let Some((text, pulse)) = &s.message {
        let font = prop(60.0);
        let g = p.layout(text.clone(), font, Color32::BLACK, inner.width() - 60.0 * k);
        let h = (g.size().y + 44.0 * k).min(320.0 * k);
        let r = Rect::from_min_size(pos2(inner.left(), y), Vec2::new(inner.width(), h));
        let fill = AMBER.lerp_to_gamma(Color32::WHITE, pulse * 0.6);
        p.rect_filled(r, 14.0 * k, fill);
        p.with_clip_rect(r.intersect(rect)).galley(
            pos2(r.left() + 30.0 * k, r.top() + 22.0 * k),
            g,
            Color32::BLACK,
        );
        y += h + gap;
    }

    // The slide now and the next one.
    let rest = inner.bottom() - y;
    let label_h = 34.0 * k;
    let title_h = if s.show_next && !s.next_title.is_empty() {
        48.0 * k
    } else {
        0.0
    };
    let pics_h = (if s.show_notes { rest * 0.56 } else { rest } - label_h - title_h).max(20.0);
    // Next is 0.6 of now; both as large as the height and width allow, next
    // right beside now.
    const NEXT_SCALE: f32 = 0.6;
    let now_w = if s.show_next {
        ((inner.width() - gap) / (1.0 + NEXT_SCALE)).min(pics_h * s.aspect)
    } else {
        inner.width().min(pics_h * s.aspect)
    }
    .max(10.0);
    let next_w = now_w * NEXT_SCALE;
    let fit = |w: f32| Vec2::new(w, w / s.aspect);
    let now_size = fit(now_w);
    // Centred when there is room to spare.
    let used = if s.show_next {
        now_w + gap + next_w
    } else {
        now_w
    };
    let x0 = inner.left() + ((inner.width() - used) / 2.0).max(0.0);
    let now = Rect::from_min_size(pos2(x0, y + label_h), now_size);
    p.text(
        pos2(now.left(), y),
        egui::Align2::LEFT_TOP,
        "NOW",
        prop(24.0),
        CURRENT,
    );
    paint_output(
        &p,
        now,
        None,
        s.cur.as_ref(),
        1.0,
        if s.blackout { 1.0 } else { 0.0 },
        None,
        s.cur_video.as_ref(),
    );
    p.rect_stroke(
        now,
        4.0 * k,
        Stroke::new(3.0 * k, if s.blackout { DANGER } else { CURRENT }),
        egui::StrokeKind::Outside,
    );
    if s.blackout {
        p.text(
            now.center(),
            egui::Align2::CENTER_CENTER,
            "BLACKOUT",
            prop(56.0),
            DANGER,
        );
    }
    if s.show_next {
        let x = now.right() + gap;
        let size = fit(next_w);
        let next = Rect::from_min_size(pos2(x, y + label_h), size);
        p.text(pos2(x, y), egui::Align2::LEFT_TOP, "NEXT", prop(24.0), NEXT);
        p.text(
            pos2(x + 80.0 * k, y + 2.0 * k),
            egui::Align2::LEFT_TOP,
            &s.next_label,
            prop(22.0),
            MUTED,
        );
        paint_output(&p, next, None, s.next.as_ref(), 1.0, 0.0, None, None);
        p.rect_stroke(
            next,
            4.0 * k,
            Stroke::new(2.0 * k, NEXT),
            egui::StrokeKind::Outside,
        );
        if s.next.is_none() {
            p.text(
                next.center(),
                egui::Align2::CENTER_CENTER,
                &s.next_label,
                prop(40.0),
                MUTED,
            );
        }
        if !s.next_title.is_empty() {
            let g = p.layout(s.next_title.clone(), prop(30.0), TEXT, next_w);
            p.galley(pos2(x, next.bottom() + 12.0 * k), g, TEXT);
        }
    }
    y = now.bottom() + title_h + gap;

    // Notes, as large as fits.
    if s.show_notes {
        let r = Rect::from_min_max(pos2(inner.left(), y), inner.max);
        if r.height() > 40.0 * k {
            p.rect_filled(r, 12.0 * k, Color32::from_rgb(16, 18, 22));
            p.text(
                r.left_top() + Vec2::new(22.0 * k, 16.0 * k),
                egui::Align2::LEFT_TOP,
                "NOTES",
                prop(20.0),
                MUTED,
            );
            let area = Rect::from_min_max(
                r.left_top() + Vec2::new(22.0 * k, 50.0 * k),
                r.right_bottom() - Vec2::new(22.0 * k, 14.0 * k),
            );
            if s.notes.trim().is_empty() {
                p.text(
                    area.left_top(),
                    egui::Align2::LEFT_TOP,
                    "No notes on this slide",
                    prop(30.0),
                    Color32::from_gray(90),
                );
            } else {
                let mut size = s.notes_size;
                let g = loop {
                    let g = p.layout(s.notes.clone(), prop(size), TEXT, area.width());
                    if g.size().y <= area.height() || size <= 20.0 {
                        break g;
                    }
                    size *= 0.92;
                };
                p.with_clip_rect(area.intersect(rect))
                    .galley(area.left_top(), g, TEXT);
            }
        }
    }
}
