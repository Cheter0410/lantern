//! The start screen (new show, open, migrate, recent shows) over a banner,
//! and the setup wizard a new show runs through – output, sound, timecode –
//! before the main window opens.

use super::*;
use crate::showfolder::{self, Old, Placement, Plan};

/// What the window shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum Screen {
    #[default]
    Start,
    Wizard(Step),
    Main,
}

/// The steps of the setup wizard, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Step {
    Output,
    Audio,
    Timecode,
    Ready,
}

const STEPS: [(Step, &str); 4] = [
    (Step::Output, "Output"),
    (Step::Audio, "Audio"),
    (Step::Timecode, "Timecode"),
    (Step::Ready, "Ready"),
];

impl Step {
    fn index(self) -> usize {
        STEPS.iter().position(|(s, _)| *s == self).unwrap_or(0)
    }
    fn next(self) -> Option<Step> {
        STEPS.get(self.index() + 1).map(|(s, _)| *s)
    }
    fn back(self) -> Option<Step> {
        self.index().checked_sub(1).map(|i| STEPS[i].0)
    }
}

/// The start screen's form.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum Form {
    #[default]
    None,
    New,
    Migrate,
}

#[derive(Default)]
pub(super) struct StartState {
    pub(super) form: Form,
    pub(super) plan: Plan,
    /// The older show to migrate.
    pub(super) old: Option<Old>,
    pub(super) error: Option<String>,
    banner: Option<TextureHandle>,
}

impl StartState {
    fn fresh_plan(&mut self) {
        self.plan = Plan {
            location: showfolder::default_location(),
            ..Plan::default()
        };
        self.old = None;
        self.error = None;
    }
}

/// Most shows kept in "Recent".
const RECENT: usize = 8;

/// A card on the start screen: dark glass, a stroke that lights up on hover.
const GLASS: Color32 = Color32::from_rgba_premultiplied(16, 18, 23, 228);

impl App {
    /// Remember a show for the start screen.
    pub(super) fn remember(&mut self, file: &Path) {
        let file = file.to_path_buf();
        self.settings.recent.retain(|p| p != &file);
        self.settings.recent.insert(0, file.clone());
        self.settings.recent.truncate(RECENT);
        self.settings.last = Some(file);
        self.settings.save();
    }

    /// Shows for "Recent", newest first.
    fn recent(&self) -> Vec<PathBuf> {
        let mut r = self.settings.recent.clone();
        if let Some(l) = &self.settings.last {
            if !r.contains(l) {
                r.insert(0, l.clone());
            }
        }
        r.truncate(RECENT);
        r
    }

    /// Open a show (or presentation) and go to the main window.
    pub(super) fn open_main(&mut self, path: &Path, ctx: &egui::Context) {
        self.load_error = None;
        self.open(path, ctx);
        if self.load_error.is_none() {
            self.screen = Screen::Main;
        } else {
            self.start.error = self.load_error.take();
        }
    }

    /// The start screen again (from the main window).
    pub(super) fn back_to_start(&mut self) {
        self.autosave();
        self.start.form = Form::None;
        self.start.error = None;
        self.screen = Screen::Start;
    }

    /// Create the show of the form and start the wizard.
    pub(super) fn create_show(&mut self, ctx: &egui::Context) {
        // Outputs and inputs of the show before are a good start.
        let show = Show {
            output: self.show.output.clone(),
            audio: self.show.audio.clone(),
            timecode: self.show.timecode.clone(),
            osc_port: self.show.osc_port,
            ..Show::default()
        };
        match self.start.plan.create(show) {
            Ok(file) => {
                self.load_error = None;
                self.open(&file, ctx);
                self.start.form = Form::None;
                self.screen = Screen::Wizard(Step::Output);
            }
            Err(e) => self.start.error = Some(e),
        }
    }

    /// Migrate the older show of the form and open it.
    pub(super) fn migrate_show(&mut self, ctx: &egui::Context) {
        let Some(old) = self.start.old.clone() else {
            self.start.error = Some("Choose the older show first.".into());
            return;
        };
        let fallback = Show {
            output: self.show.output.clone(),
            audio: self.show.audio.clone(),
            timecode: self.show.timecode.clone(),
            osc_port: self.show.osc_port,
            ..Show::default()
        };
        match showfolder::migrate(&old, &self.start.plan, fallback) {
            Ok(file) => {
                self.start.form = Form::None;
                self.open_main(&file, ctx);
                if self.screen == Screen::Main {
                    self.note(format!("Migrated to {}", file.display()));
                }
            }
            Err(e) => self.start.error = Some(e),
        }
    }

    pub(super) fn choose_old(&mut self, p: &Path) {
        match showfolder::find_old(p) {
            Ok(old) => {
                self.start.plan.presentation = old.presentation.clone();
                let stem = match &old.show {
                    Some(f) => showfolder::name_from(f),
                    None => showfolder::name_from(&old.presentation),
                };
                self.start.plan.name = stem;
                self.start.old = Some(old);
                self.start.error = None;
            }
            Err(e) => {
                self.start.old = None;
                self.start.error = Some(e);
            }
        }
    }

    // ------------------------------------------------------------ background

    fn backdrop(&mut self, ui: &mut egui::Ui, dim: f32) -> Rect {
        let rect = ui.max_rect();
        let tex = self.start.banner.get_or_insert_with(|| {
            ui.ctx().load_texture(
                "banner",
                banner_image(1280, 720),
                egui::TextureOptions::LINEAR,
            )
        });
        // Cover: fill the window, crop what is too much.
        let [tw, th] = tex.size().map(|v| v as f32);
        let scale = (rect.width() / tw).max(rect.height() / th);
        let size = Vec2::new(tw * scale, th * scale);
        // Keep the light at the right; crop from the left.
        let min = pos2(rect.right() - size.x, rect.center().y - size.y / 2.0);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let p = ui.painter();
        p.rect_filled(rect, 0.0, BG);
        p.image(tex.id(), Rect::from_min_size(min, size), uv, Color32::WHITE);
        if dim > 0.0 {
            p.rect_filled(rect, 0.0, Color32::from_black_alpha((dim * 255.0) as u8));
        }
        rect
    }

    // ------------------------------------------------------------ start screen

    pub(super) fn start_screen(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let rect = self.backdrop(ui, 0.0);
        let pad = (rect.width() * 0.06).clamp(36.0, 96.0);
        let inner = rect.shrink2(Vec2::new(pad, pad * 0.8));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::top_down(Align::Min)),
        );
        let ui = &mut child;

        // Title.
        ui.add_space((inner.height() * 0.06).min(40.0));
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
            lantern_mark(ui.painter(), r);
            ui.add_space(6.0);
            ui.label(RichText::new("Lantern").size(46.0).strong().color(TEXT));
        });
        ui.add_space(2.0);
        ui.label(
            RichText::new("Presentation playback for the stage")
                .size(18.0)
                .color(Color32::from_rgb(196, 200, 210)),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "Version {}  ·  PowerPoint, Keynote, PDF  ·  Display, NDI, Timecode",
                env!("CARGO_PKG_VERSION")
            ))
            .size(12.5)
            .color(MUTED),
        );
        ui.add_space((inner.height() * 0.07).clamp(18.0, 56.0));

        if let Some(e) = self.start.error.clone() {
            if self.start.form == Form::None {
                banner(ui, DANGER, |ui| {
                    ui.label(RichText::new(e).color(DANGER));
                });
            }
        }

        match self.start.form {
            Form::None => self.start_home(ui, &ctx),
            Form::New => self.new_form(ui, &ctx),
            Form::Migrate => self.migrate_form(ui, &ctx),
        }

        // Footer.
        let foot = pos2(inner.left(), rect.bottom() - pad * 0.45);
        ui.painter().text(
            foot,
            egui::Align2::LEFT_CENTER,
            "Drop a show file or a presentation anywhere to open it",
            egui::FontId::proportional(12.5),
            MUTED,
        );
    }

    fn start_home(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let gap = 16.0;
        let w = ((ui.available_width() - 2.0 * gap) / 3.0).clamp(200.0, 300.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            if action_card(
                ui,
                w,
                Icon::New,
                "New show",
                "A show folder with your presentation, then set up output, sound and timecode.",
                true,
            )
            .clicked()
            {
                self.start.fresh_plan();
                self.start.form = Form::New;
            }
            if action_card(
                ui,
                w,
                Icon::Open,
                "Open show",
                "A .lantern show file – or a presentation to play right away.",
                false,
            )
            .clicked()
            {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Lantern shows", &["lantern"])
                    .add_filter("Presentations", showfolder::PRESENTATIONS)
                    .pick_file()
                {
                    self.open_main(&p, ctx);
                }
            }
            if action_card(
                ui,
                w,
                Icon::Migrate,
                "Migrate older show",
                "A show from Lantern 0.3 or earlier becomes a show folder; its settings come along.",
                false,
            )
            .clicked()
            {
                self.start.fresh_plan();
                self.start.form = Form::Migrate;
            }
        });

        // Recent shows.
        let recent = self.recent();
        if recent.is_empty() {
            return;
        }
        ui.add_space(30.0);
        let width = 3.0 * w + 2.0 * gap;
        ui.label(
            RichText::new("RECENT SHOWS")
                .size(11.5)
                .strong()
                .color(MUTED),
        );
        ui.add_space(6.0);
        let mut open = None;
        Frame::new()
            .fill(GLASS)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(6))
            .show(ui, |ui| {
                ui.set_width(width - 12.0);
                let rows = ((ui.available_height() - 30.0) / 44.0).floor().max(1.0) as usize;
                for p in recent.iter().take(rows) {
                    if recent_row(ui, p).clicked() {
                        open = Some(p.clone());
                    }
                }
            });
        if let Some(p) = open {
            if p.exists() {
                self.open_main(&p, ctx);
            } else {
                self.start.error = Some(format!("Not there any more: {}", p.display()));
                self.settings.recent.retain(|r| r != &p);
                if self.settings.last.as_ref() == Some(&p) {
                    self.settings.last = None;
                }
                self.settings.save();
            }
        }
    }

    /// The form around a new or migrated show: name, place, copy or move.
    fn plan_fields(&mut self, ui: &mut egui::Ui, presentation: bool) {
        let plan = &mut self.start.plan;
        egui::Grid::new("plan")
            .num_columns(2)
            .spacing([16.0, 12.0])
            .min_col_width(110.0)
            .show(ui, |ui| {
                if presentation {
                    ui.label(RichText::new("Presentation").color(MUTED));
                    ui.horizontal(|ui| {
                        path_field(ui, &plan.presentation, "No presentation chosen");
                        if ui.button("Choose…").clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .add_filter("Presentations", showfolder::PRESENTATIONS)
                                .pick_file()
                            {
                                if plan.name.trim().is_empty() {
                                    plan.name = showfolder::name_from(&p);
                                }
                                plan.presentation = p;
                            }
                        }
                    });
                    ui.end_row();
                }
                ui.label(RichText::new("Show name").color(MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut plan.name)
                        .hint_text("e.g. Annual Gala 2026")
                        .desired_width(360.0),
                );
                ui.end_row();
                ui.label(RichText::new("Location").color(MUTED));
                ui.horizontal(|ui| {
                    path_field(ui, &plan.location, "No place chosen");
                    if ui.button("Choose…").clicked() {
                        let mut d = rfd::FileDialog::new();
                        if plan.location.is_dir() {
                            d = d.set_directory(&plan.location);
                        }
                        if let Some(p) = d.pick_folder() {
                            plan.location = p;
                        }
                    }
                });
                ui.end_row();
                ui.label(RichText::new("Presentation file").color(MUTED));
                segmented(
                    ui,
                    &mut plan.placement,
                    &[
                        (Placement::Copy, "Copy into the show folder"),
                        (Placement::Move, "Move into the show folder"),
                    ],
                );
                ui.end_row();
            });
        ui.add_space(10.0);
        let what = match plan.placement {
            Placement::Copy => "The original stays where it is.",
            Placement::Move => "The original is moved – nothing stays behind.",
        };
        if !showfolder::clean_name(&plan.name).is_empty() && !plan.location.as_os_str().is_empty() {
            let pres = plan
                .presentation
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "presentation".into());
            ui.label(
                RichText::new(format!(
                    "Creates  {}{}  with  {}  and  {}.  {what}",
                    plan.folder().display(),
                    std::path::MAIN_SEPARATOR,
                    plan.show_file()
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    pres
                ))
                .size(12.5)
                .color(MUTED),
            );
        }
    }

    fn form_buttons(&mut self, ui: &mut egui::Ui, ok: &str) -> bool {
        let problem = self.start.plan.problem();
        let mut go = false;
        ui.add_space(14.0);
        if let Some(e) = &self.start.error {
            ui.label(RichText::new(e).color(DANGER));
            ui.add_space(6.0);
        }
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new("Cancel").size(15.0))
                        .corner_radius(8)
                        .min_size(Vec2::new(110.0, 38.0)),
                )
                .clicked()
            {
                self.start.form = Form::None;
                self.start.error = None;
            }
            ui.add_space(6.0);
            let r = ui.add_enabled(
                problem.is_none(),
                egui::Button::new(RichText::new(ok).size(15.0).strong().color(Color32::BLACK))
                    .fill(AMBER)
                    .corner_radius(8)
                    .min_size(Vec2::new(200.0, 38.0)),
            );
            go = r.clicked();
            if let Some(p) = &problem {
                ui.add_space(8.0);
                ui.label(RichText::new(p).color(MUTED));
            }
        });
        go
    }

    fn new_form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        form_card(ui, |ui| {
            ui.label(RichText::new("New show").size(24.0).strong());
            ui.label(
                RichText::new(
                    "Lantern makes a folder for the show and puts the presentation in. \
                     Copy, back up or take the folder to another computer as one piece.",
                )
                .color(MUTED),
            );
            ui.add_space(16.0);
            self.plan_fields(ui, true);
            if self.form_buttons(ui, "Create and set up") {
                self.create_show(ctx);
            }
        });
    }

    fn migrate_form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        form_card(ui, |ui| {
            ui.label(RichText::new("Migrate an older show").size(24.0).strong());
            ui.label(
                RichText::new(
                    "Older shows keep a .lantern file next to the presentation. Migrating makes \
                     a show folder of them; slide settings, outputs, sound and timecode come along.",
                )
                .color(MUTED),
            );
            ui.add_space(16.0);
            egui::Grid::new("old")
                .num_columns(2)
                .spacing([16.0, 12.0])
                .min_col_width(110.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("Older show").color(MUTED));
                    ui.horizontal(|ui| {
                        let shown = self
                            .start
                            .old
                            .as_ref()
                            .map(|o| o.show.clone().unwrap_or(o.presentation.clone()))
                            .unwrap_or_default();
                        path_field(ui, &shown, "No show chosen");
                        if ui.button("Choose…").clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .add_filter("Lantern shows and presentations", &{
                                    let mut v = vec!["lantern"];
                                    v.extend_from_slice(showfolder::PRESENTATIONS);
                                    v
                                })
                                .pick_file()
                            {
                                self.choose_old(&p);
                            }
                        }
                    });
                    ui.end_row();
                });
            if let Some(o) = &self.start.old {
                ui.add_space(4.0);
                let text = match &o.show {
                    Some(_) => match Show::load(o.show.as_deref().unwrap_or(Path::new(""))) {
                        Ok(s) => format!(
                            "Found: settings for {} slides · timecode {} · {}",
                            s.slides.len(),
                            match s.timecode.source {
                                TcSource::Off => "off",
                                TcSource::Ltc => "LTC",
                                TcSource::Midi => "MTC (MIDI)",
                                TcSource::RtpMidi => "MTC (network)",
                            },
                            if s.output.ndi { "NDI on" } else { "NDI off" }
                        ),
                        Err(e) => e,
                    },
                    None => "No show file next to it – starts with the settings of the last show."
                        .into(),
                };
                ui.label(RichText::new(text).size(12.5).color(OK));
            }
            ui.add_space(12.0);
            ui.add_enabled_ui(self.start.old.is_some(), |ui| self.plan_fields(ui, false));
            if self.form_buttons(ui, "Migrate and open") {
                self.migrate_show(ctx);
            }
        });
    }

    // ------------------------------------------------------------ wizard

    pub(super) fn wizard(&mut self, ui: &mut egui::Ui, step: Step) {
        let rect = self.backdrop(ui, 0.55);
        let size = Vec2::new(
            (rect.width() - 80.0).clamp(560.0, 860.0),
            (rect.height() - 80.0).clamp(460.0, 680.0),
        );
        let card = Rect::from_center_size(rect.center(), size);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(card)
                .layout(Layout::top_down(Align::Min)),
        );
        let ui = &mut child;
        Frame::new()
            .fill(Color32::from_rgba_premultiplied(18, 20, 25, 246))
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::symmetric(32, 26))
            .show(ui, |ui| {
                ui.set_min_size(card.size() - Vec2::new(64.0, 52.0));
                let name = self
                    .show_file
                    .as_ref()
                    .and_then(|f| f.file_stem())
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("SETUP").size(11.5).strong().color(AMBER));
                    ui.label(RichText::new(&name).size(11.5).strong().color(MUTED));
                });
                ui.add_space(10.0);
                steps_bar(ui, step);
                ui.add_space(18.0);
                let (title, about) = match step {
                    Step::Output => (
                        "Where does the picture go?",
                        "Choose the display for the slides – projector, LED processor or a \
                         second screen – and/or send them over the network with NDI. Turn on the \
                         test picture to check the real screen: cropping, colours, sharpness and \
                         dropped frames.",
                    ),
                    Step::Audio => (
                        "Where does the sound go?",
                        "Videos in the presentation play their sound here. Choose the device and \
                         the channels, then press Test tone and listen on the PA.",
                    ),
                    Step::Timecode => (
                        "Does the show follow timecode?",
                        "Slides can follow timecode from the sound desk or show control: LTC on \
                         an audio input, MTC from a MIDI input (an IAC bus on this Mac) or MTC \
                         over the network. Leave it Off to run the show with GO alone.",
                    ),
                    Step::Ready => (
                        "Ready for the show",
                        "This is how the show is set up. Everything can be changed later under \
                         Settings.",
                    ),
                };
                ui.label(RichText::new(title).size(24.0).strong());
                ui.add_space(4.0);
                ui.label(RichText::new(about).color(MUTED));
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);
                let footer = 56.0;
                egui::ScrollArea::vertical()
                    .max_height((ui.available_height() - footer).max(80.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| match step {
                        Step::Output => self.settings_output(ui),
                        Step::Audio => self.settings_audio(ui),
                        Step::Timecode => {
                            self.settings_timecode(ui);
                            self.wizard_tc_status(ui);
                        }
                        Step::Ready => self.wizard_summary(ui),
                    });
                ui.add_space(10.0);
                self.wizard_footer(ui, step);
            });
    }

    fn wizard_tc_status(&self, ui: &mut egui::Ui) {
        if self.show.timecode.source == TcSource::Off {
            return;
        }
        ui.add_space(14.0);
        let (text, color) = match self.tc.reading() {
            Some(r) => (
                format!("Receiving {}", show::format_tc(r.secs, r.fps)),
                if r.running { OK } else { TC },
            ),
            None => (
                "Nothing received yet – start the timecode to check".into(),
                MUTED,
            ),
        };
        ui.horizontal(|ui| {
            status(ui, &text, color);
        })
        .response
        .on_hover_text(self.tc.status());
    }

    fn wizard_summary(&mut self, ui: &mut egui::Ui) {
        let o = &self.show.output;
        let display = match o.display {
            None => "Off".to_string(),
            Some(i) => match self.displays.get(i) {
                Some(d) => format!("Display {} – {} ({}×{})", i + 1, d.name, d.px[0], d.px[1]),
                None => format!("Display {} (not connected)", i + 1),
            },
        };
        let ndi = if o.ndi {
            format!("On, as \"{}\"", self.ndi_name())
        } else {
            "Off".into()
        };
        let fps = if o.fps_auto {
            format!("Automatic ({} fps)", self.output_fps())
        } else {
            format!("{} fps", o.fps)
        };
        let a = &self.show.audio;
        let audio = if !a.enabled {
            "Off".to_string()
        } else if a.device.is_empty() {
            "System default".into()
        } else {
            a.device.clone()
        };
        let t = &self.show.timecode;
        let tc = match t.source {
            TcSource::Off => "Off – the show runs with GO".to_string(),
            TcSource::Ltc => format!(
                "LTC on {} channel {}",
                if t.ltc_device.is_empty() {
                    "the default input"
                } else {
                    &t.ltc_device
                },
                t.ltc_channel + 1
            ),
            TcSource::Midi => format!(
                "MTC from {}",
                if t.midi_port.is_empty() {
                    "the first MIDI input"
                } else {
                    &t.midi_port
                }
            ),
            TcSource::RtpMidi => format!("MTC over the network, session \"{}\"", t.rtp_name),
        };
        let pres = if let Some(l) = &self.loading {
            let s = l.status.lock().map(|s| s.clone()).unwrap_or_default();
            (s, MUTED)
        } else if let Some(e) = &self.load_error {
            (e.clone(), DANGER)
        } else if self.deck.is_some() {
            (format!("{} slides ready", self.slides.len()), OK)
        } else {
            ("Not loaded".into(), MUTED)
        };
        let folder = self
            .show_file
            .as_ref()
            .and_then(|f| f.parent())
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        egui::Grid::new("summary")
            .num_columns(2)
            .spacing([22.0, 10.0])
            .show(ui, |ui| {
                for (k, v) in [
                    ("Show folder", folder),
                    ("Display", display),
                    ("NDI", ndi),
                    ("Frame rate", fps),
                    ("Sound", audio),
                    ("Timecode", tc),
                ] {
                    ui.label(RichText::new(k).color(MUTED));
                    ui.label(RichText::new(v).strong());
                    ui.end_row();
                }
                ui.label(RichText::new("Presentation").color(MUTED));
                ui.horizontal(|ui| {
                    if self.loading.is_some() {
                        ui.spinner();
                    }
                    ui.label(RichText::new(pres.0).strong().color(pres.1));
                });
                ui.end_row();
            });
        if self.test.on {
            ui.add_space(12.0);
            ui.label(
                RichText::new("The test picture is turned off when the show opens.").color(AMBER),
            );
        }
    }

    fn wizard_footer(&mut self, ui: &mut egui::Ui, step: Step) {
        let big = |text: &str, primary: bool| {
            let b = egui::Button::new(
                RichText::new(text.to_string())
                    .size(15.0)
                    .strong()
                    .color(if primary { Color32::BLACK } else { TEXT }),
            )
            .corner_radius(8)
            .min_size(Vec2::new(130.0, 38.0));
            if primary {
                b.fill(AMBER)
            } else {
                b
            }
        };
        let w = ui.available_width();
        ui.allocate_ui_with_layout(
            Vec2::new(w, 38.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                if step != Step::Ready
                    && ui
                        .add(
                            egui::Button::new(RichText::new("Skip setup").color(MUTED))
                                .frame(false),
                        )
                        .on_hover_text("Open the show now; set it up later under Settings")
                        .clicked()
                {
                    self.finish_wizard();
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    match step.next() {
                        Some(n) => {
                            if ui.add(big("Next", true)).clicked() {
                                self.screen = Screen::Wizard(n);
                            }
                        }
                        None => {
                            if ui.add(big("Open show", true)).clicked() {
                                self.finish_wizard();
                            }
                        }
                    }
                    if let Some(b) = step.back() {
                        if ui.add(big("Back", false)).clicked() {
                            self.screen = Screen::Wizard(b);
                        }
                    }
                });
            },
        );
    }

    pub(super) fn finish_wizard(&mut self) {
        self.test.on = false;
        self.autosave();
        self.screen = Screen::Main;
    }
}

// ---------------------------------------------------------------- widgets

#[derive(Clone, Copy)]
enum Icon {
    New,
    Open,
    Migrate,
}

fn action_card(
    ui: &mut egui::Ui,
    w: f32,
    icon: Icon,
    title: &str,
    about: &str,
    primary: bool,
) -> egui::Response {
    let h = 168.0;
    let (rect, r) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    let hot = r.hovered();
    let p = ui.painter();
    let stroke = if primary {
        AMBER.gamma_multiply(if hot { 1.0 } else { 0.7 })
    } else if hot {
        Color32::from_rgb(90, 96, 110)
    } else {
        LINE
    };
    p.rect(
        rect,
        CornerRadius::same(14),
        if hot {
            Color32::from_rgba_premultiplied(26, 28, 34, 236)
        } else {
            GLASS
        },
        Stroke::new(if primary { 1.5 } else { 1.0 }, stroke),
        egui::StrokeKind::Inside,
    );
    // Icon tile.
    let tile = Rect::from_min_size(rect.min + Vec2::new(20.0, 20.0), Vec2::splat(40.0));
    p.rect_filled(
        tile,
        CornerRadius::same(10),
        if primary {
            AMBER
        } else {
            Color32::from_rgb(40, 44, 54)
        },
    );
    let ink = if primary { Color32::BLACK } else { TEXT };
    draw_icon(p, tile.shrink(11.0), icon, ink);
    p.text(
        rect.min + Vec2::new(20.0, 78.0),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(18.0),
        TEXT,
    );
    let galley = p.layout(
        about.to_string(),
        egui::FontId::proportional(13.0),
        MUTED,
        w - 40.0,
    );
    p.galley(rect.min + Vec2::new(20.0, 106.0), galley, MUTED);
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn draw_icon(p: &egui::Painter, r: Rect, icon: Icon, ink: Color32) {
    let s = Stroke::new(2.2, ink);
    match icon {
        Icon::New => {
            p.line_segment([r.center_top(), r.center_bottom()], s);
            p.line_segment([r.left_center(), r.right_center()], s);
        }
        Icon::Open => {
            // A folder.
            let body = Rect::from_min_max(r.min + Vec2::new(0.0, 4.0), r.max);
            p.rect_stroke(body, CornerRadius::same(2), s, egui::StrokeKind::Middle);
            p.line_segment(
                [r.min + Vec2::new(0.0, 4.0), r.min + Vec2::new(6.0, 0.0)],
                s,
            );
            p.line_segment(
                [r.min + Vec2::new(6.0, 0.0), r.min + Vec2::new(10.0, 4.0)],
                s,
            );
        }
        Icon::Migrate => {
            // An arrow into a box.
            let y = r.center().y;
            p.line_segment([pos2(r.left(), y), pos2(r.right() - 2.0, y)], s);
            p.line_segment(
                [pos2(r.right() - 7.0, y - 5.0), pos2(r.right() - 2.0, y)],
                s,
            );
            p.line_segment(
                [pos2(r.right() - 7.0, y + 5.0), pos2(r.right() - 2.0, y)],
                s,
            );
            p.line_segment([r.left_top(), r.left_top() + Vec2::new(6.0, 0.0)], s);
            p.line_segment([r.left_top(), r.left_bottom() - Vec2::new(0.0, 0.0)], s);
            p.line_segment([r.left_bottom(), r.left_bottom() + Vec2::new(6.0, 0.0)], s);
        }
    }
}

/// The Lantern mark: a warm light in a frame.
fn lantern_mark(p: &egui::Painter, r: Rect) {
    let c = r.center();
    for (rad, a) in [(22.0, 0.10), (15.0, 0.18), (9.0, 0.45)] {
        p.circle_filled(c, rad, AMBER.gamma_multiply(a));
    }
    p.circle_filled(c, 5.0, Color32::from_rgb(255, 236, 190));
    let s = Stroke::new(1.6, AMBER);
    let body = Rect::from_center_size(c + Vec2::new(0.0, 2.0), Vec2::new(22.0, 26.0));
    p.rect_stroke(body, CornerRadius::same(5), s, egui::StrokeKind::Middle);
    p.line_segment(
        [
            body.center_top() + Vec2::new(-5.0, -4.0),
            body.center_top() + Vec2::new(5.0, -4.0),
        ],
        s,
    );
}

fn recent_row(ui: &mut egui::Ui, p: &Path) -> egui::Response {
    let w = ui.available_width();
    let (rect, r) = ui.allocate_exact_size(Vec2::new(w, 44.0), Sense::click());
    let name = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    let there = p.exists();
    let painter = ui.painter();
    if r.hovered() {
        painter.rect_filled(rect, CornerRadius::same(8), Color32::from_rgb(34, 37, 45));
    }
    let dot = pos2(rect.left() + 18.0, rect.center().y);
    painter.circle_filled(dot, 4.0, if there { AMBER } else { LINE });
    painter.text(
        pos2(rect.left() + 34.0, rect.center().y - 8.0),
        egui::Align2::LEFT_CENTER,
        &name,
        egui::FontId::proportional(15.0),
        if there { TEXT } else { MUTED },
    );
    let place = p
        .parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let place = if there {
        place
    } else {
        format!("Missing – {place}")
    };
    painter.text(
        pos2(rect.left() + 34.0, rect.center().y + 9.0),
        egui::Align2::LEFT_CENTER,
        place,
        egui::FontId::proportional(12.0),
        MUTED,
    );
    let when = std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .map(|t| {
            chrono::DateTime::<chrono::Local>::from(t)
                .format("%d %b %Y, %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    painter.text(
        pos2(rect.right() - 14.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        when,
        egui::FontId::proportional(12.0),
        MUTED,
    );
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn path_field(ui: &mut egui::Ui, p: &Path, empty: &str) {
    let (text, color) = if p.as_os_str().is_empty() {
        (empty.to_string(), MUTED)
    } else {
        (p.display().to_string(), TEXT)
    };
    Frame::new()
        .fill(BG)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_width(300.0);
            ui.add(egui::Label::new(RichText::new(text).color(color)).truncate());
        })
        .response
        .on_hover_text(p.display().to_string());
}

fn form_card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    Frame::new()
        .fill(Color32::from_rgba_premultiplied(18, 20, 25, 242))
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(28, 24))
        .show(ui, |ui| {
            ui.set_max_width(760.0);
            add(ui);
        });
}

/// 1 Output — 2 Audio — 3 Timecode — 4 Ready; done steps ticked.
fn steps_bar(ui: &mut egui::Ui, now: Step) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::hover());
    let p = ui.painter();
    let n = STEPS.len();
    let seg = w / n as f32;
    let y = rect.top() + 12.0;
    for (i, (s, label)) in STEPS.iter().enumerate() {
        let x = rect.left() + seg * i as f32 + 12.0;
        let c = pos2(x, y);
        let done = i < now.index();
        let here = *s == now;
        if done {
            p.circle_filled(c, 11.0, AMBER.gamma_multiply(0.85));
            let s = Stroke::new(2.0, Color32::BLACK);
            p.line_segment([c + Vec2::new(-5.0, 0.0), c + Vec2::new(-1.5, 4.0)], s);
            p.line_segment([c + Vec2::new(-1.5, 4.0), c + Vec2::new(5.0, -4.0)], s);
        } else if here {
            p.circle_filled(c, 11.0, AMBER);
            p.text(
                c,
                egui::Align2::CENTER_CENTER,
                (i + 1).to_string(),
                egui::FontId::proportional(13.0),
                Color32::BLACK,
            );
        } else {
            p.circle_stroke(c, 10.0, Stroke::new(1.5, LINE));
            p.text(
                c,
                egui::Align2::CENTER_CENTER,
                (i + 1).to_string(),
                egui::FontId::proportional(13.0),
                MUTED,
            );
        }
        let text = p.text(
            pos2(x + 18.0, y),
            egui::Align2::LEFT_CENTER,
            *label,
            egui::FontId::proportional(14.0),
            if here || done { TEXT } else { MUTED },
        );
        // The line to the next step, from after the label.
        if i + 1 < n {
            let (a, b) = (text.right() + 12.0, x + seg - 16.0);
            if b > a {
                p.line_segment(
                    [pos2(a, y), pos2(b, y)],
                    Stroke::new(
                        2.0,
                        if done {
                            AMBER.gamma_multiply(0.7)
                        } else {
                            LINE
                        },
                    ),
                );
            }
        }
    }
}

// ---------------------------------------------------------------- banner

/// The start screen's banner: a dark stage, beams of light from above, a
/// warm lantern glow on the right and haze. Made here, so no picture file.
pub(super) fn banner_image(w: usize, h: usize) -> egui::ColorImage {
    fn lin(c: [f32; 3]) -> [f32; 3] {
        c.map(|v| (v / 255.0).powf(2.2))
    }
    fn hash(x: u32, y: u32) -> f32 {
        let mut n = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263);
        n = (n ^ (n >> 13)).wrapping_mul(1_274_126_177);
        ((n ^ (n >> 16)) & 0xffff) as f32 / 65535.0
    }
    fn smooth(a: f32, b: f32, x: f32) -> f32 {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
    let top = lin([18.0, 20.0, 32.0]);
    let bottom = lin([4.0, 4.0, 7.0]);
    let amber = lin([255.0, 168.0, 64.0]);
    let warm = lin([255.0, 214.0, 150.0]);
    let cool = lin([90.0, 130.0, 255.0]);
    let aspect = w as f32 / h as f32;
    // Beams: origin x (0..1 of width), angle (radians from straight down),
    // spread, colour, strength.
    let beams = [
        (0.58, 0.32, 0.075, warm, 0.55),
        (0.80, -0.10, 0.06, amber, 0.65),
        (0.95, -0.42, 0.08, warm, 0.40),
        (0.40, 0.55, 0.05, cool, 0.30),
        (0.70, 0.12, 0.035, cool, 0.25),
    ];
    let glow = (0.79 * aspect, 0.60);
    let mut pixels = Vec::with_capacity(w * h);
    for py in 0..h {
        let v = py as f32 / h as f32;
        for px in 0..w {
            let u = px as f32 / w as f32;
            let base = smooth(0.0, 1.0, v);
            let mut c = [0.0f32; 3];
            for k in 0..3 {
                c[k] = top[k] + (bottom[k] - top[k]) * base;
            }
            // Haze: slow waves, so the beams look like they pass smoke.
            let haze = 0.65
                + 0.2 * (u * 7.0 + v * 3.0).sin() * (v * 5.0 - u * 2.0).cos()
                + 0.15 * (u * 17.0 - v * 11.0).sin();
            let mut beam_sum = 0.0;
            for (ox, ang, spread, col, k) in beams {
                let dx = (u - ox) * aspect;
                let dy = v + 0.08;
                let a = dx.atan2(dy) - ang;
                let fall = (-(a / spread).powi(2)).exp();
                let len = (dx * dx + dy * dy).sqrt();
                let fade = (-len * 1.1).exp() * smooth(0.0, 0.12, dy);
                let b = fall * fade * k * haze;
                beam_sum += b;
                for i in 0..3 {
                    c[i] += col[i] * b * 0.5;
                }
            }
            // The lantern: a soft warm glow with a bright core.
            let (gx, gy) = (u * aspect - glow.0, v - glow.1);
            let r2 = gx * gx + gy * gy;
            let g =
                (-r2 / 0.0009).exp() * 1.6 + (-r2 / 0.012).exp() * 0.5 + (-r2 / 0.15).exp() * 0.16;
            for i in 0..3 {
                c[i] += amber[i] * g + warm[i] * (-r2 / 0.0003).exp() * 1.2;
            }
            // Floor: a faint reflection below the horizon.
            let floor = smooth(0.80, 0.86, v);
            if floor > 0.0 {
                let ry = (2.0 * 0.83 - v) - glow.1;
                let rr = gx * gx * 0.4 + ry * ry * 4.0;
                let refl = (-rr / 0.01).exp() * 0.12 * floor;
                for i in 0..3 {
                    c[i] = c[i] * (1.0 - 0.25 * floor) + amber[i] * refl;
                }
            }
            // Dust in the light.
            let d = hash(px as u32, py as u32);
            if d > 0.9985 {
                let s = (d - 0.9985) / 0.0015 * beam_sum * 2.2;
                for i in 0..3 {
                    c[i] += warm[i] * s;
                }
            }
            // Darker at the left (for the text) and at the edges.
            let left = 0.45 + 0.55 * smooth(0.12, 0.62, u);
            let (cx, cy) = (u - 0.6, v - 0.5);
            let vig = 1.0 - 0.6 * (cx * cx * 1.2 + cy * cy * 1.6).min(1.0);
            let dither = (hash(px as u32 + 7, py as u32 + 13) - 0.5) / 255.0;
            let rgb = c.map(|x| {
                let x = x * left * vig;
                let t = 1.0 - (-x * 1.6).exp();
                ((t.powf(1.0 / 2.2) + dither).clamp(0.0, 1.0) * 255.0) as u8
            });
            pixels.push(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
        }
    }
    egui::ColorImage::new([w, h], pixels)
}
