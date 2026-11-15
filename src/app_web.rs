//! The web remote in the window: starts the server when it is switched on,
//! hands it the show's state and pictures, and Settings › Network shows the
//! address with a QR code to scan.

use super::*;
use crate::web::{self, Which};

#[derive(Default)]
pub(super) struct WebUi {
    pub(super) server: Option<web::Server>,
    pub(super) error: Option<(u16, String)>,
    at: Option<Instant>,
    /// The QR code for the address (made again when it changes).
    qr: Option<(String, qrcodegen::QrCode)>,
}

fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

impl App {
    pub(super) fn update_web(&mut self, ctx: &egui::Context) {
        while let Ok(c) = self.web_rx.try_recv() {
            self.command_from(logui::Origin::Web, c);
        }
        let cfg = self.show.web.clone();
        if !cfg.enabled {
            self.web.server = None;
            self.web.error = None;
            return;
        }
        let running = self.web.server.as_ref().map(|s| s.port);
        // Port 0 (tests) takes any free port: keep that one.
        let wanted = running.filter(|_| cfg.port == 0).unwrap_or(cfg.port);
        if running != Some(wanted) && self.web.error.as_ref().is_none_or(|(p, _)| *p != cfg.port) {
            self.web.server = None;
            let c = ctx.clone();
            match web::Server::start(cfg.port, self.web_tx.clone(), move || c.request_repaint()) {
                Ok(s) => {
                    self.web.server = Some(s);
                    self.web.error = None;
                }
                Err(e) => self.web.error = Some((cfg.port, e)),
            }
        }
        if self
            .web
            .at
            .is_some_and(|t| t.elapsed() < Duration::from_millis(100))
        {
            return;
        }
        self.web.at = Some(Instant::now());
        let Some(server) = &self.web.server else {
            return;
        };
        server.set_pin(cfg.pin.trim());
        let next = self.next_index();
        let scene = self.stage_scene();
        let pic = |p: Option<usize>| p.and_then(|p| self.fulls.get(&p)).map(|f| &f.image);
        let id = |p: Option<usize>| {
            p.and_then(|p| self.fulls.get(&p))
                .map(|f| Arc::as_ptr(&f.image.rgba) as u64)
        };
        let cur = self.player.current;
        server.set_pic(Which::Now, id(cur).unwrap_or(0), pic(cur));
        server.set_pic(Which::Next, id(next).unwrap_or(0), pic(next));
        let starts_video = self.go_starts_video();
        let next_cfg = next.and_then(|n| self.show.slides.get(n));
        let state = serde_json::json!({
            "show": self.show_file.as_ref().and_then(|f| f.file_stem())
                .map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            "clock": clock(),
            "live": self.live(),
            "slide": cur.map(|c| c + 1),
            "total": self.slides.len(),
            "next": next.map(|n| n + 1),
            "next_title": next.and_then(|n| self.slides.get(n)).map(|s| s.title.clone()),
            "next_how": next.map(|_| match next_cfg.map(|c| c.trigger).unwrap_or_default() {
                Trigger::Go => "comes with GO".to_string(),
                Trigger::Timecode => format!(
                    "comes at TC {}",
                    next_cfg.map(|c| c.tc.as_str()).unwrap_or("")
                ),
            }),
            "pic_now": id(cur),
            "pic_next": id(next),
            "can_go": next.is_some() || starts_video,
            "go_what": match next {
                _ if starts_video => "Play video".to_string(),
                Some(n) => format!("Slide {}", n + 1),
                None => "End".to_string(),
            },
            "blackout": self.player.blackout,
            "video": self.video.is_some(),
            "video_playing": self.video_left().is_some_and(|(_, p)| p) && !self.video_paused(),
            "notes": cur.and_then(|c| self.slides.get(c)).map(|s| s.notes.clone())
                .unwrap_or_default(),
            "timer": scene_timer(&scene).0,
            "timer_color": hex(scene_timer(&scene).1),
            "timer_running": self.stage.timer.running(),
            "message": self.stage.message.as_ref().map(|m| m.text.clone()),
        });
        server.set_state(state.to_string());
    }

    /// Addresses to type or scan.
    fn web_url(&self) -> Option<String> {
        let port = self.web.server.as_ref()?.port;
        Some(format!(
            "http://{}:{port}",
            local_ip().unwrap_or_else(|| "localhost".into())
        ))
    }

    /// Settings › Network: the web remote.
    pub(super) fn settings_web(&mut self, ui: &mut egui::Ui) {
        section(ui, "Web remote (phone, tablet)");
        let w = &mut self.show.web;
        ui.checkbox(
            &mut w.enabled,
            "On – control the show from a browser in the same network",
        );
        ui.add_enabled_ui(w.enabled, |ui| {
            egui::Grid::new("web")
                .num_columns(2)
                .spacing([14.0, 8.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Port").color(MUTED));
                    ui.add(egui::DragValue::new(&mut w.port).range(1024..=65000));
                    ui.end_row();
                    ui.label(RichText::new("PIN").color(MUTED));
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut w.pin)
                                .hint_text("none")
                                .char_limit(12)
                                .desired_width(90.0),
                        );
                        if r.changed() {
                            w.pin.retain(|c| c.is_ascii_alphanumeric());
                        }
                        ui.label(
                            RichText::new(if w.pin.trim().is_empty() {
                                "Anyone in the network can press GO."
                            } else {
                                "Asked for once on each device."
                            })
                            .small()
                            .color(if w.pin.trim().is_empty() {
                                AMBER
                            } else {
                                MUTED
                            }),
                        );
                    });
                    ui.end_row();
                });
        });
        if !self.show.web.enabled {
            return;
        }
        if let Some((_, e)) = &self.web.error {
            ui.label(RichText::new(e).color(DANGER));
            return;
        }
        let Some(url) = self.web_url() else { return };
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let qr = match &self.web.qr {
                Some((u, q)) if *u == url => q.clone(),
                _ => {
                    let Ok(q) = qrcodegen::QrCode::encode_text(&url, qrcodegen::QrCodeEcc::Medium)
                    else {
                        return;
                    };
                    self.web.qr = Some((url.clone(), q.clone()));
                    q
                }
            };
            let n = qr.size();
            let side = 132.0;
            let (r, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
            let p = ui.painter();
            p.rect_filled(r, 6.0, Color32::WHITE);
            let quiet = 3.0;
            let cell = side / (n as f32 + 2.0 * quiet);
            for y in 0..n {
                for x in 0..n {
                    if qr.get_module(x, y) {
                        let min = r.min + Vec2::new(x as f32 + quiet, y as f32 + quiet) * cell;
                        p.rect_filled(
                            Rect::from_min_size(min, Vec2::splat(cell + 0.2)),
                            0.0,
                            Color32::BLACK,
                        );
                    }
                }
            }
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("Scan with the phone, or open:").color(MUTED));
                ui.label(RichText::new(&url).monospace().size(16.0).strong());
                ui.add_space(4.0);
                let n = self.web.server.as_ref().map_or(0, |s| s.clients());
                let (text, color) = match n {
                    0 => ("No device connected".to_string(), MUTED),
                    1 => ("1 device connected".to_string(), OK),
                    n => (format!("{n} devices connected"), OK),
                };
                status(ui, &text, color);
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "GO+, GO−, Blackout, the slide now and next, notes, the countdown and \
                         stage messages. Works in Live too.",
                    )
                    .small()
                    .color(MUTED),
                );
            });
        });
    }
}

fn scene_timer(s: &stagewin::StageScene) -> (String, Color32) {
    s.timer_parts()
}
