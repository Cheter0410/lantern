//! Output health in the window: a look every half second, a badge in the
//! top bar while something is wrong, and the list of problems under it.

use super::*;
use crate::health::{self, Area, Facts, Level, Tc};

#[derive(Default)]
pub(super) struct HealthUi {
    pub(super) monitor: health::Monitor,
    pub(super) open: bool,
    at: Option<Instant>,
    /// Timecode: last seen running, and when a stand began.
    tc_running_at: Option<Instant>,
    tc_first: Option<Instant>,
}

impl App {
    pub(super) fn update_health(&mut self) {
        if self
            .health
            .at
            .is_some_and(|t| t.elapsed() < Duration::from_millis(500))
        {
            return;
        }
        self.health.at = Some(Instant::now());
        let tc = match (self.show.timecode.source, self.tc.reading()) {
            (TcSource::Off, _) => {
                self.health.tc_running_at = None;
                self.health.tc_first = None;
                Tc::Off
            }
            (_, None) => Tc::Never,
            (_, Some(r)) => {
                let first = *self.health.tc_first.get_or_insert_with(Instant::now);
                if r.running {
                    self.health.tc_running_at = Some(Instant::now());
                    Tc::Running
                } else {
                    let from = self.health.tc_running_at.unwrap_or(first);
                    Tc::Stopped {
                        at: r.secs,
                        fps: r.fps,
                        for_secs: from.elapsed().as_secs_f64(),
                    }
                }
            }
        };
        let o = &self.show.output;
        let facts = Facts {
            display: o.display,
            displays: self.displays.len(),
            display_is_here: o
                .display
                .and_then(|d| self.displays.get(d))
                .is_some_and(|d| d.here),
            ndi: o.ndi,
            ndi_error: self.ndi_error.clone(),
            ndi_receivers: self.ndi.as_ref().and_then(|s| s.receivers()),
            tc,
            tc_slides: self
                .show
                .slides
                .iter()
                .any(|s| s.trigger == Trigger::Timecode),
            audio_enabled: self.show.audio.enabled,
            audio_ok: self.audio.ok,
            audio_disconnected: self.audio.disconnected,
            audio_status: self.audio.status.clone(),
            sound_in_show: self
                .clips
                .values()
                .any(|c| c.as_ref().is_ok_and(|c| c.probe.audio)),
            stage_display: self.show.stage.display,
        };
        let fresh = self.health.monitor.update(health::check(&facts));
        if let Some(i) = fresh.iter().max_by_key(|i| i.level) {
            self.note(format!("Problem: {}", i.title));
        }
    }

    /// The badge in the top bar (nothing while all is well).
    pub(super) fn health_badge(&mut self, ui: &mut egui::Ui) {
        let m = &self.health.monitor;
        let Some(worst) = m.worst() else {
            self.health.open = false;
            return;
        };
        let n = m.issues.len();
        let color = match worst {
            Level::Error => DANGER,
            Level::Warn => AMBER,
        };
        let text = if n == 1 {
            m.issues[0].title.clone()
        } else {
            format!("{n} problems")
        };
        let font = egui::FontId::proportional(13.5);
        let g = ui
            .painter()
            .layout_no_wrap(text.clone(), font, Color32::WHITE);
        let w = (g.size().x + 42.0).min(340.0);
        let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::click());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Health: {text}"))
        });
        // A new problem: the badge pulses for a few seconds.
        let flash = m.flashing();
        let pulse = if flash {
            let t = m.new_at.map_or(0.0, |t| t.elapsed().as_secs_f32());
            ui.ctx().request_repaint_after(Duration::from_millis(30));
            0.5 + 0.5 * (t * std::f32::consts::TAU * 1.5).cos()
        } else {
            0.0
        };
        let fill = color.gamma_multiply(0.22 + 0.5 * pulse);
        let p = ui.painter();
        p.rect(
            r,
            CornerRadius::same(7),
            fill,
            Stroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
        warning_sign(p, pos2(r.left() + 16.0, r.center().y), color);
        let clip = Rect::from_min_max(
            pos2(r.left() + 30.0, r.top()),
            pos2(r.right() - 8.0, r.bottom()),
        );
        p.with_clip_rect(clip).galley(
            pos2(r.left() + 30.0, r.center().y - g.size().y / 2.0),
            g,
            Color32::WHITE,
        );
        if resp
            .on_hover_text("Output problems – click for details")
            .clicked()
        {
            self.health.open = !self.health.open;
        }
        if self.health.open {
            self.health_list(ui.ctx(), r);
        }
    }

    fn health_list(&mut self, ctx: &egui::Context, under: Rect) {
        let mut goto: Option<Area> = None;
        let mut close = false;
        let area = egui::Area::new(egui::Id::unique("health-list"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2((under.right() - 420.0).max(8.0), under.bottom() + 8.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, LINE))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(14))
                    .shadow(egui::Shadow {
                        offset: [0, 6],
                        blur: 18,
                        spread: 0,
                        color: Color32::from_black_alpha(140),
                    })
                    .show(ui, |ui| {
                        ui.set_width(420.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("OUTPUT HEALTH").small().strong().color(MUTED));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("Close").clicked() {
                                    close = true;
                                }
                            });
                        });
                        ui.add_space(6.0);
                        for (n, i) in self.health.monitor.issues.iter().enumerate() {
                            if n > 0 {
                                ui.add_space(4.0);
                                ui.separator();
                                ui.add_space(4.0);
                            }
                            let color = match i.level {
                                Level::Error => DANGER,
                                Level::Warn => AMBER,
                            };
                            ui.horizontal(|ui| {
                                let (r, _) =
                                    ui.allocate_exact_size(Vec2::new(18.0, 18.0), Sense::hover());
                                warning_sign(ui.painter(), r.center(), color);
                                ui.label(RichText::new(&i.title).strong().color(color));
                                if let Some(d) = self.health.monitor.since(i.key) {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(RichText::new(since_text(d)).small().color(MUTED));
                                    });
                                }
                            });
                            ui.label(RichText::new(&i.detail).color(TEXT));
                            let tab = match i.area {
                                Area::Output => "Output settings",
                                Area::Audio => "Audio settings",
                                Area::Timecode => "Timecode settings",
                                Area::Stage => "Stage settings",
                            };
                            if ui.small_button(tab).clicked() {
                                goto = Some(i.area);
                            }
                        }
                    });
            });
        if let Some(a) = goto {
            self.settings_open = true;
            self.settings_tab = match a {
                Area::Output => SettingsTab::Output,
                Area::Audio => SettingsTab::Audio,
                Area::Timecode => SettingsTab::Timecode,
                Area::Stage => SettingsTab::Stage,
            };
            close = true;
        }
        // A click elsewhere (not on the badge) closes it.
        let outside = ctx.input(|i| i.pointer.any_click())
            && ctx
                .pointer_interact_pos()
                .is_some_and(|p| !area.response.rect.contains(p) && !under.contains(p));
        if close || outside || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.health.open = false;
        }
    }
}

fn since_text(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("for {s} s")
    } else {
        format!("for {} min", s / 60)
    }
}

/// A small warning triangle with "!" (painted: no font has to have it).
fn warning_sign(p: &egui::Painter, c: egui::Pos2, color: Color32) {
    let (w, h) = (15.0, 13.0);
    let pts = vec![
        pos2(c.x, c.y - h / 2.0),
        pos2(c.x + w / 2.0, c.y + h / 2.0),
        pos2(c.x - w / 2.0, c.y + h / 2.0),
    ];
    p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    p.line_segment(
        [pos2(c.x, c.y - 2.5), pos2(c.x, c.y + 2.0)],
        Stroke::new(1.8, Color32::BLACK),
    );
    p.circle_filled(pos2(c.x, c.y + 4.4), 1.0, Color32::BLACK);
}
