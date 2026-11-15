//! The show log in the window: watches what changes (the slide on the output,
//! blackout, the video, the stage, problems) and writes it down with who did
//! it, and a panel on the right shows it with a field for notes.

use super::*;
use crate::showlog::{self, Kind, Log};

/// Why the slide changed (set just before, read when the change is logged).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Why {
    Go,
    Back,
    Clicked,
    Timecode,
    Jump,
    First,
    Reset,
}

/// Where a command came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Origin {
    #[default]
    Here,
    Osc,
    Web,
}

impl Origin {
    fn says(self) -> &'static str {
        match self {
            Origin::Here => "",
            Origin::Osc => " via OSC",
            Origin::Web => " via web remote",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VideoState {
    Ready,
    Playing,
    Paused,
    Ended,
}

/// What was seen last time (a change of it is logged).
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    slide: Option<usize>,
    step: usize,
    blackout: bool,
    video: Option<(usize, VideoState)>,
    timer_running: bool,
    timer_untouched: bool,
    message: Option<String>,
    live: bool,
    issues: Vec<(&'static str, String)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Filter {
    #[default]
    All,
    Cues,
    Problems,
    Notes,
}

impl Filter {
    fn shows(self, k: Kind) -> bool {
        match self {
            Filter::All => true,
            Filter::Cues => matches!(k, Kind::Cue | Kind::Blackout | Kind::Video),
            Filter::Problems => matches!(k, Kind::Problem | Kind::Fixed),
            Filter::Notes => k == Kind::Note,
        }
    }
}

#[derive(Default)]
pub(super) struct LogUi {
    pub(super) log: Log,
    pub(super) open: bool,
    pub(super) filter: Filter,
    pub(super) draft: String,
    /// The show file the log is for.
    for_file: Option<PathBuf>,
    seen: Option<Seen>,
    pub(super) why: Option<Why>,
    pub(super) origin: Origin,
}

impl App {
    /// The slide changes, and why (for the log).
    pub(super) fn goto_by(&mut self, slide: Option<usize>, why: Why) {
        self.log.why = Some(why);
        self.goto(slide);
    }

    /// A command from OSC or the web remote.
    pub(super) fn command_from(&mut self, origin: Origin, c: Command) {
        self.log.origin = origin;
        self.command(c);
    }

    fn timecode_now(&self) -> Option<String> {
        if self.show.timecode.source == TcSource::Off {
            return None;
        }
        self.tc
            .reading()
            .filter(|r| r.running)
            .map(|r| show::format_tc(r.secs, r.fps))
    }

    pub(super) fn log_entry(&mut self, kind: Kind, text: impl Into<String>) {
        let tc = self.timecode_now();
        self.log.log.push(clock(), tc, kind, text);
    }

    fn seen_now(&self) -> Seen {
        let video = self.video.as_ref().map(|v| {
            let state = match &v.playing {
                None => VideoState::Ready,
                Some(p) if p.ended() => VideoState::Ended,
                Some(p) if p.is_paused() => VideoState::Paused,
                Some(_) => VideoState::Playing,
            };
            (v.slide, state)
        });
        Seen {
            slide: self.player.current,
            step: self.player.step,
            blackout: self.player.blackout,
            video,
            timer_running: self.stage.timer.running(),
            timer_untouched: self.stage.timer.untouched(),
            message: self.stage.message.as_ref().map(|m| m.text.clone()),
            live: self.live(),
            issues: self
                .health
                .monitor
                .issues
                .iter()
                .map(|i| (i.key, i.title.clone()))
                .collect(),
        }
    }

    fn slide_name(&self, i: usize) -> String {
        match self.slides.get(i).map(|s| s.title.trim()) {
            Some(t) if !t.is_empty() => format!("Slide {} · {t}", i + 1),
            _ => format!("Slide {}", i + 1),
        }
    }

    /// Log what changed since the last frame (call once at the end of a frame).
    pub(super) fn update_log(&mut self) {
        // Another show: its own file, and start looking afresh.
        if self.log.for_file != self.show_file {
            self.log.for_file = self.show_file.clone();
            let date = chrono::Local::now().format("%Y-%m-%d").to_string();
            let file = self
                .show_file
                .as_deref()
                .and_then(|f| showlog::file_for(f, &date));
            self.log.log.set_file(file);
            self.log.seen = None;
            if let Some(name) = self
                .show_file
                .as_ref()
                .and_then(|f| f.file_stem())
                .map(|s| s.to_string_lossy().to_string())
            {
                self.log_entry(Kind::Show, format!("Show \"{name}\" opened"));
            }
        }
        let now = self.seen_now();
        let why = self.log.why.take();
        let origin = std::mem::take(&mut self.log.origin);
        let Some(was) = self.log.seen.replace(now.clone()) else {
            // The first look: problems already there are worth a line too.
            for (_, title) in &now.issues {
                self.log_entry(Kind::Problem, title.clone());
            }
            return;
        };
        if was == now {
            return;
        }
        let by = origin.says();
        if now.live != was.live {
            self.log_entry(
                Kind::Show,
                if now.live {
                    "Live mode on"
                } else {
                    "Edit mode"
                },
            );
        }
        if now.slide != was.slide || now.step != was.step {
            let how = match why {
                Some(Why::Go) => format!(" – GO{by}"),
                Some(Why::Back) => format!(" – GO−{by}"),
                Some(Why::Clicked) => format!(" – clicked{by}"),
                Some(Why::Timecode) => " – on timecode".to_string(),
                Some(Why::Jump) => format!(" – jump{by}"),
                Some(Why::First) => format!(" – first slide{by}"),
                Some(Why::Reset) | None => by.to_string(),
            };
            let text = match now.slide {
                Some(i) if self.steps_of(i) > 0 => format!(
                    "{} · build {}/{}{how}",
                    self.slide_name(i),
                    now.step,
                    self.steps_of(i)
                ),
                Some(i) => format!("{}{how}", self.slide_name(i)),
                None => format!("Reset – nothing on the output{by}"),
            };
            self.log_entry(Kind::Cue, text);
        }
        if now.blackout != was.blackout {
            let text = if now.blackout {
                format!("Blackout on{by}")
            } else {
                format!("Blackout off{by}")
            };
            self.log_entry(Kind::Blackout, text);
        }
        if let Some((slide, state)) = now.video {
            let before = was.video.filter(|(s, _)| *s == slide).map(|(_, s)| s);
            if before != Some(state) {
                let left = self
                    .video_left()
                    .map(|(l, _)| format!(", {} left", crate::stage::fmt_countdown(l)))
                    .unwrap_or_default();
                let text = match (before, state) {
                    (_, VideoState::Ready) => None,
                    (Some(VideoState::Paused), VideoState::Playing) => {
                        Some(format!("Video on slide {} goes on{left}{by}", slide + 1))
                    }
                    (_, VideoState::Playing) => {
                        Some(format!("Video on slide {} started{left}{by}", slide + 1))
                    }
                    (_, VideoState::Paused) => {
                        Some(format!("Video on slide {} paused{left}{by}", slide + 1))
                    }
                    (_, VideoState::Ended) => Some(format!("Video on slide {} ended", slide + 1)),
                };
                if let Some(t) = text {
                    self.log_entry(Kind::Video, t);
                }
            }
        }
        if now.message != was.message {
            let text = match &now.message {
                Some(m) => format!("Stage message: \"{m}\"{by}"),
                None => format!("Stage message cleared{by}"),
            };
            self.log_entry(Kind::Stage, text);
        }
        let left = crate::stage::fmt_countdown(self.stage.timer.remaining());
        if now.timer_running != was.timer_running {
            let text = if now.timer_running {
                format!("Countdown started, {left} left{by}")
            } else {
                format!("Countdown paused at {left}{by}")
            };
            self.log_entry(Kind::Stage, text);
        } else if now.timer_untouched && !was.timer_untouched {
            self.log_entry(Kind::Stage, format!("Countdown reset to {left}{by}"));
        }
        for (key, title) in &now.issues {
            if !was.issues.iter().any(|(k, _)| k == key) {
                self.log_entry(Kind::Problem, title.clone());
            }
        }
        for (key, title) in &was.issues {
            if !now.issues.iter().any(|(k, _)| k == key) {
                self.log_entry(Kind::Fixed, format!("Fixed: {title}"));
            }
        }
    }

    /// Top bar: open or close the log.
    pub(super) fn log_button(&mut self, ui: &mut egui::Ui) {
        let b =
            egui::Button::new(RichText::new("Log").color(if self.log.open { AMBER } else { TEXT }))
                .fill(if self.log.open { CARD } else { PANEL })
                .stroke(Stroke::new(1.0, LINE))
                .corner_radius(CornerRadius::same(6));
        if ui
            .add(b)
            .on_hover_text("Show log: every cue, blackout, video, problem and note (Ctrl/Cmd+L)")
            .clicked()
        {
            self.log.open = !self.log.open;
        }
    }

    /// The log panel on the right.
    pub(super) fn log_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Show log").size(16.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(flat_button("Close"))
                    .on_hover_text("Close")
                    .clicked()
                {
                    self.log.open = false;
                }
                if let Some(f) = self.log.log.file().map(Path::to_path_buf) {
                    if ui
                        .add(flat_button("File"))
                        .on_hover_text(format!("Open {}", f.display()))
                        .clicked()
                    {
                        if f.exists() {
                            office::open_url(&f.to_string_lossy());
                        } else {
                            self.note("Nothing written yet");
                        }
                    }
                }
                if ui
                    .add(flat_button("Copy"))
                    .on_hover_text("Copy the whole log as text")
                    .clicked()
                {
                    ui.ctx().copy_text(self.log.log.text());
                    self.note("Show log copied");
                }
            });
        });
        let cues = self.log.log.count(Kind::Cue);
        let problems = self.log.log.count(Kind::Problem);
        ui.label(
            RichText::new(match problems {
                0 => format!("{cues} cues · no problems"),
                1 => format!("{cues} cues · 1 problem"),
                n => format!("{cues} cues · {n} problems"),
            })
            .small()
            .color(MUTED),
        );
        ui.add_space(6.0);
        segmented(
            ui,
            &mut self.log.filter,
            &[
                (Filter::All, "All"),
                (Filter::Cues, "Cues"),
                (Filter::Problems, "Problems"),
                (Filter::Notes, "Notes"),
            ],
        );
        ui.add_space(6.0);
        if let Some(e) = &self.log.log.error {
            ui.label(RichText::new(e).small().color(DANGER));
        }
        // The note field stays at the bottom; the list fills the rest.
        let note_h = 64.0;
        let list_h = (ui.available_height() - note_h).max(80.0);
        Frame::new()
            .fill(CLOCK_BG)
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .max_height(list_h - 16.0)
                    .min_scrolled_height(list_h - 16.0)
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let filter = self.log.filter;
                        let mut any = false;
                        for e in self.log.log.entries.iter().filter(|e| filter.shows(e.kind)) {
                            any = true;
                            log_row(ui, e);
                        }
                        if !any {
                            ui.label(RichText::new("Nothing yet.").color(MUTED));
                        }
                    });
            });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let w = ui.available_width() - 56.0;
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.log.draft)
                    .hint_text("Note for the log…")
                    .desired_width(w),
            );
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let add = ui
                .add_enabled(!self.log.draft.trim().is_empty(), egui::Button::new("Add"))
                .clicked();
            if (enter || add) && !self.log.draft.trim().is_empty() {
                let text = std::mem::take(&mut self.log.draft).trim().to_string();
                self.log_entry(Kind::Note, text);
                if enter {
                    r.request_focus();
                }
            }
        });
    }
}

fn kind_color(k: Kind) -> Color32 {
    match k {
        Kind::Show => MUTED,
        Kind::Cue => CURRENT,
        Kind::Blackout => DANGER,
        Kind::Video => NEXT,
        Kind::Stage => AMBER,
        Kind::Problem => DANGER,
        Kind::Fixed => OK,
        Kind::Note => TEXT,
    }
}

fn log_row(ui: &mut egui::Ui, e: &showlog::Entry) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new(&e.at).monospace().size(12.0).color(MUTED));
        let c = kind_color(e.kind);
        let (r, _) = ui.allocate_exact_size(Vec2::new(4.0, 16.0), Sense::hover());
        ui.painter().rect_filled(r, 2.0, c);
        ui.vertical(|ui| {
            let text = RichText::new(&e.text).size(13.0);
            let text = match e.kind {
                Kind::Problem => text.color(DANGER),
                Kind::Note => text.strong(),
                Kind::Show => text.color(MUTED),
                _ => text.color(TEXT),
            };
            ui.add(egui::Label::new(text).wrap());
            if let Some(tc) = &e.tc {
                ui.label(
                    RichText::new(format!("TC {tc}"))
                        .monospace()
                        .size(11.0)
                        .color(TC),
                );
            }
        });
    });
    ui.add_space(2.0);
}
