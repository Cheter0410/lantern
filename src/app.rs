//! The Lantern window: slide list, live / next, GO, notes, slide settings,
//! and the outputs (full-screen display, NDI).

use crate::deck::{self, Deck, Image, Kind};
use crate::office::{self, InstallState};
use crate::osc::{self, Command};
use crate::pptx::{self, SlideInfo, Transition};
use crate::show::{self, Chase, Player, Resolution, Show, TcSource, TransitionCfg, Trigger};
use crate::{ndi, net, timecode};
use egui::{
    pos2, Align, Color32, CornerRadius, Frame, Key, Layout, Margin, Rect, RichText, Sense, Stroke,
    TextureHandle, Vec2,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

const BG: Color32 = Color32::from_rgb(12, 13, 16);
const PANEL: Color32 = Color32::from_rgb(20, 22, 27);
const CARD: Color32 = Color32::from_rgb(28, 31, 38);
const LINE: Color32 = Color32::from_rgb(44, 48, 58);
const TEXT: Color32 = Color32::from_rgb(232, 235, 239);
const MUTED: Color32 = Color32::from_rgb(138, 144, 158);
// Colour meaning as in Show Master: next = green, on air / running = blue,
// Live mode = red, Edit mode = blue, error = red.
/// Good / connected (status dots).
const OK: Color32 = Color32::from_rgb(34, 197, 94);
/// The next slide.
const NEXT: Color32 = Color32::from_rgb(34, 197, 94);
const NEXT_BG: Color32 = Color32::from_rgb(20, 60, 36);
/// The slide on the output.
const CURRENT: Color32 = Color32::from_rgb(59, 130, 246);
const CURRENT_BG: Color32 = Color32::from_rgb(23, 42, 82);
const GO: Color32 = Color32::from_rgb(22, 163, 74);
const MODE_LIVE: Color32 = Color32::from_rgb(220, 38, 38);
const MODE_EDIT: Color32 = Color32::from_rgb(37, 99, 235);
const TC: Color32 = Color32::from_rgb(96, 165, 250);
const DANGER: Color32 = Color32::from_rgb(239, 68, 68);
const AMBER: Color32 = Color32::from_rgb(255, 196, 92);
const HOVER: Color32 = Color32::from_rgb(26, 29, 35);
/// Background of the clock cells.
const CLOCK_BG: Color32 = Color32::from_rgb(10, 11, 14);

/// App settings kept between starts.
#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct Settings {
    last: Option<PathBuf>,
    /// Live mode: the show runs, nothing about it can be changed by accident.
    live: bool,
}

fn settings_file() -> Option<PathBuf> {
    // Tests keep their settings away from the real ones.
    if let Some(d) = std::env::var_os("LANTERN_CONFIG_DIR") {
        return Some(PathBuf::from(d).join("settings.json"));
    }
    Some(dirs::config_dir()?.join("lantern").join("settings.json"))
}

impl Settings {
    fn load() -> Self {
        settings_file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }
    fn save(&self) {
        let Some(f) = settings_file() else { return };
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(f, t);
        }
    }
}

/// The show file belonging to a presentation: `talk.pptx` -> `talk.lantern`.
pub fn show_file_for(p: &Path) -> PathBuf {
    p.with_extension("lantern")
}

#[derive(Clone)]
struct Disp {
    name: String,
    pos: [f32; 2],
    size: [f32; 2],
    px: [u32; 2],
    here: bool,
}

type Loaded = Result<(PathBuf, Option<pptx::Info>, Vec<u8>), String>;

struct Loading {
    rx: Receiver<Loaded>,
    status: Arc<Mutex<String>>,
    started: Instant,
}

struct Full {
    image: Image,
    tex: TextureHandle,
}

pub struct App {
    settings: Settings,
    show: Show,
    show_file: Option<PathBuf>,
    saved: Option<Show>,
    slides: Vec<SlideInfo>,
    deck: Option<Deck>,
    loading: Option<Loading>,
    load_error: Option<String>,
    mtime: Option<SystemTime>,
    changed_on_disk: bool,
    last_mtime_check: Instant,
    thumbs: HashMap<usize, TextureHandle>,
    fulls: HashMap<usize, Full>,
    full_size: (u32, u32),
    player: Player,
    chase: Chase,
    tc: timecode::Input,
    osc: Option<osc::Listener>,
    osc_error: Option<(u16, String)>,
    osc_tx: Sender<Command>,
    osc_rx: Receiver<Command>,
    ndi: Option<ndi::Sender>,
    ndi_error: Option<String>,
    selected: Option<usize>,
    install: InstallState,
    office_found: Option<PathBuf>,
    office_checked: Instant,
    displays: Vec<Disp>,
    displays_at: Instant,
    settings_open: bool,
    settings_tab: SettingsTab,
    net: net::Net,
    output_started: Option<Instant>,
    message: Option<(String, Instant)>,
    scroll_to_live: bool,
}

impl App {
    pub fn new(ctx: &egui::Context, open: Option<PathBuf>) -> Self {
        style(ctx);
        let (osc_tx, osc_rx) = channel();
        let settings = Settings::load();
        let mut app = App {
            settings,
            show: Show::default(),
            show_file: None,
            saved: None,
            slides: Vec::new(),
            deck: None,
            loading: None,
            load_error: None,
            mtime: None,
            changed_on_disk: false,
            last_mtime_check: Instant::now(),
            thumbs: HashMap::new(),
            fulls: HashMap::new(),
            full_size: (0, 0),
            player: Player::default(),
            chase: Chase::default(),
            tc: timecode::Input::default(),
            osc: None,
            osc_error: None,
            osc_tx,
            osc_rx,
            ndi: None,
            ndi_error: None,
            selected: None,
            install: Arc::default(),
            office_found: office::find(),
            office_checked: Instant::now(),
            displays: Vec::new(),
            displays_at: Instant::now() - Duration::from_secs(10),
            settings_open: false,
            settings_tab: SettingsTab::default(),
            net: net::Net::default(),
            output_started: None,
            message: None,
            scroll_to_live: false,
        };
        let start = open.or_else(|| app.settings.last.clone().filter(|p| p.exists()));
        if let Some(p) = start {
            app.open(&p, ctx);
        }
        app
    }

    fn note(&mut self, m: impl Into<String>) {
        self.message = Some((m.into(), Instant::now()));
    }

    fn live(&self) -> bool {
        self.settings.live
    }

    /// Edit or Live. Live closes the slide settings and keeps the show as it is.
    fn set_live(&mut self, on: bool) {
        if self.settings.live == on {
            return;
        }
        self.settings.live = on;
        self.settings.save();
        if on {
            self.selected = None;
            self.note("Live – Änderungen gesperrt");
        } else {
            self.note("Bearbeiten");
        }
    }

    fn locked(&mut self) {
        self.note("Im Live-Modus gesperrt – zum Ändern auf „Bearbeiten“ (Strg/Cmd+E)");
    }

    // ---------------------------------------------------------------- files

    /// Open a presentation or a show file.
    fn open(&mut self, path: &Path, ctx: &egui::Context) {
        let is_show = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("lantern"));
        let (show, file) = if is_show {
            match Show::load(path) {
                Ok(s) => (s, path.to_path_buf()),
                Err(e) => {
                    self.load_error = Some(e);
                    return;
                }
            }
        } else {
            let f = show_file_for(path);
            let mut s = Show::load(&f).unwrap_or_else(|_| Show {
                // Outputs and inputs carry over from the show before.
                output: self.show.output.clone(),
                timecode: self.show.timecode.clone(),
                osc_port: self.show.osc_port,
                ..Show::default()
            });
            s.presentation = path.to_path_buf();
            (s, f)
        };
        self.show = show;
        self.saved = Some(self.show.clone());
        self.show_file = Some(file.clone());
        self.settings.last = Some(file);
        self.settings.save();
        self.player = Player::default();
        self.chase.reset();
        self.selected = None;
        self.load(ctx);
    }

    /// (Re)load the presentation of the show in the background.
    fn load(&mut self, ctx: &egui::Context) {
        let path = self.show.presentation.clone();
        self.mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        self.changed_on_disk = false;
        self.load_error = None;
        let status = Arc::new(Mutex::new("Lese die Präsentation…".to_string()));
        let (tx, rx) = channel();
        let (st, c) = (status.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = (|| {
                let is_pdf = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
                let is_pptx = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("pptx"));
                let info = if is_pptx {
                    Some(pptx::read(&path)?)
                } else {
                    None
                };
                let pdf = if is_pdf {
                    path.clone()
                } else {
                    office::to_pdf(&path, &|s| {
                        if let Ok(mut g) = st.lock() {
                            *g = s;
                        }
                        c.request_repaint();
                    })?
                };
                let bytes = std::fs::read(&pdf).map_err(|e| format!("{}: {e}", pdf.display()))?;
                Ok((path, info, bytes))
            })();
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.loading = Some(Loading {
            rx,
            status,
            started: Instant::now(),
        });
    }

    fn finish_load(&mut self, r: Loaded) {
        self.loading = None;
        let (path, info, bytes) = match r {
            Ok(v) => v,
            Err(e) => {
                self.load_error = Some(e);
                return;
            }
        };
        let deck = match Deck::open(bytes) {
            Ok(d) => d,
            Err(e) => {
                self.load_error = Some(e);
                return;
            }
        };
        let visible = info.as_ref().map(|i| i.visible()).unwrap_or_default();
        self.slides = if visible.len() == deck.pages {
            visible
        } else {
            (0..deck.pages)
                .map(|i| SlideInfo {
                    number: i + 1,
                    title: String::new(),
                    notes: String::new(),
                    hidden: false,
                    transition: Transition::Cut,
                })
                .collect()
        };
        self.show.match_slides(&self.slides);
        if self.saved.as_ref().is_some_and(|s| s.slides.is_empty()) {
            self.saved = Some(self.show.clone());
        }
        let tw = 320u32;
        let th = (tw as f32 / deck.aspect).round() as u32;
        for p in 0..deck.pages {
            deck.request(p, tw, th, Kind::Thumb);
        }
        self.thumbs.clear();
        self.fulls.clear();
        if let Some(c) = self.player.current {
            if c >= deck.pages {
                self.player.current = None;
            }
        }
        self.player.previous = None;
        self.deck = Some(deck);
        self.note(format!(
            "{} geladen – {} Folien",
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            self.slides.len()
        ));
    }

    fn pick_file(&mut self, ctx: &egui::Context) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter(
                "Präsentationen",
                &["pptx", "ppt", "odp", "key", "pdf", "lantern"],
            )
            .pick_file()
        {
            self.open(&p, ctx);
        }
    }

    fn autosave(&mut self) {
        if self.saved.as_ref() == Some(&self.show) {
            return;
        }
        if let Some(f) = &self.show_file {
            match self.show.save(f) {
                Ok(()) => self.saved = Some(self.show.clone()),
                Err(e) => self.note(format!("Speichern fehlgeschlagen: {e}")),
            }
        }
    }

    // ---------------------------------------------------------------- playback

    fn output_size(&self) -> (u32, u32) {
        if let Some(s) = self.show.output.resolution.size() {
            return s;
        }
        self.show
            .output
            .display
            .and_then(|d| self.displays.get(d))
            .map(|d| (d.px[0].max(320), d.px[1].max(240)))
            .unwrap_or((1920, 1080))
    }

    fn transition_of(&self, i: usize) -> Transition {
        match self.show.slides.get(i).map(|s| s.transition) {
            Some(TransitionCfg::Cut) => Transition::Cut,
            Some(TransitionCfg::Fade(s)) => Transition::Fade(s),
            _ => self
                .slides
                .get(i)
                .map(|s| s.transition)
                .unwrap_or(Transition::Cut),
        }
    }

    fn goto(&mut self, slide: Option<usize>) {
        let n = self.slides.len();
        let slide = slide.filter(|s| *s < n);
        let t = slide
            .map(|s| self.transition_of(s))
            .unwrap_or(Transition::Fade(0.5));
        self.player.show(slide, t);
        self.scroll_to_live = true;
    }

    fn next_index(&self) -> Option<usize> {
        let n = self.slides.len();
        match self.player.current {
            None => (n > 0).then_some(0),
            Some(c) => (c + 1 < n).then_some(c + 1),
        }
    }

    fn go(&mut self) {
        if let Some(n) = self.next_index() {
            self.goto(Some(n));
        }
    }

    fn back(&mut self) {
        if let Some(c) = self.player.current {
            if c > 0 {
                self.goto(Some(c - 1));
            }
        }
    }

    fn command(&mut self, c: Command) {
        match c {
            Command::Go => self.go(),
            Command::Back => self.back(),
            Command::Slide(i) => self.goto(Some(i)),
            Command::First => self.goto(Some(0)),
            Command::Clear => self.goto(None),
            Command::Blackout(v) => {
                let on = v.unwrap_or(!self.player.blackout);
                self.player.set_blackout(on);
            }
        }
    }

    /// Ask for the pictures needed soon (live, previous, next) and forget others.
    fn want_pictures(&mut self) {
        let size = self.output_size();
        if size != self.full_size {
            self.fulls.clear();
            self.full_size = size;
        }
        let Some(deck) = &self.deck else { return };
        let mut want: Vec<usize> = Vec::new();
        want.extend(self.player.current);
        want.extend(self.player.previous);
        want.extend(self.next_index());
        if let Some(c) = self.player.current {
            if c > 0 {
                want.push(c - 1);
            }
            want.push(c + 2);
        }
        want.extend(self.selected);
        want.retain(|p| *p < deck.pages);
        for p in &want {
            if !self.fulls.contains_key(p) {
                deck.request(*p, size.0, size.1, Kind::Full);
            }
        }
        if self.fulls.len() > 8 {
            self.fulls.retain(|p, _| want.contains(p));
        }
    }

    fn receive_pictures(&mut self, ctx: &egui::Context) {
        let Some(deck) = &self.deck else { return };
        for r in deck.poll() {
            let ci = egui::ColorImage::from_rgba_unmultiplied(
                [r.image.w as usize, r.image.h as usize],
                &r.image.rgba,
            );
            match r.kind {
                Kind::Thumb => {
                    let t = ctx.load_texture(
                        format!("thumb-{}", r.page),
                        ci,
                        egui::TextureOptions::LINEAR,
                    );
                    self.thumbs.insert(r.page, t);
                }
                Kind::Full => {
                    if (r.image.w, r.image.h) != self.full_size {
                        continue;
                    }
                    let tex = ctx.load_texture(
                        format!("full-{}", r.page),
                        ci,
                        egui::TextureOptions::LINEAR,
                    );
                    self.fulls.insert(
                        r.page,
                        Full {
                            image: r.image,
                            tex,
                        },
                    );
                }
            }
        }
    }

    fn follow_timecode(&mut self) {
        let Some(r) = self.tc.reading() else { return };
        let keys: Vec<(usize, f64)> = self
            .show
            .slides
            .iter()
            .enumerate()
            .filter(|(_, s)| s.trigger == Trigger::Timecode)
            .filter_map(|(i, s)| Some((i, show::parse_tc(&s.tc, r.fps.round())?)))
            .collect();
        if let Some(i) = self.chase.update(r.secs, &keys) {
            if Some(i) != self.player.current {
                self.goto(Some(i));
            }
        }
    }

    fn update_ndi(&mut self) {
        let o = &self.show.output;
        if !o.ndi {
            self.ndi = None;
            self.ndi_error = None;
            return;
        }
        let size = self.output_size();
        let name = if o.ndi_name.trim().is_empty() {
            "Lantern".to_string()
        } else {
            o.ndi_name.trim().to_string()
        };
        let fresh = self
            .ndi
            .as_ref()
            .is_some_and(|s| s.name == name && s.size == size && s.fps == o.fps);
        if !fresh {
            self.ndi = None;
            match ndi::Sender::start(&name, size.0, size.1, o.fps) {
                Ok(s) => {
                    self.ndi = Some(s);
                    self.ndi_error = None;
                }
                Err(e) => {
                    if self.ndi_error.is_none() {
                        self.ndi_error = Some(e);
                    }
                    return;
                }
            }
        }
        let pic = |p: Option<usize>| p.and_then(|p| self.fulls.get(&p)).map(|f| f.image.clone());
        let p = &self.player;
        let scene = ndi::Scene {
            previous: pic(p.previous),
            current: pic(p.current),
            since: p.since,
            fade: p.fade,
            black_from: p.black_level(),
            black_to: p.black_level(),
            black_since: Instant::now(),
            black_fade: show::BLACKOUT_FADE,
        };
        if let Some(s) = &self.ndi {
            s.set(scene);
        }
    }

    fn update_osc(&mut self, ctx: &egui::Context) {
        let port = self.show.osc_port;
        if self.osc.as_ref().map(|l| l.port) != Some(port)
            && self.osc_error.as_ref().is_none_or(|(p, _)| *p != port)
        {
            self.osc = None;
            let c = ctx.clone();
            match osc::Listener::start(port, self.osc_tx.clone(), move || c.request_repaint()) {
                Ok(l) => {
                    self.osc = Some(l);
                    self.osc_error = None;
                }
                Err(e) => self.osc_error = Some((port, e)),
            }
        }
        while let Ok(c) = self.osc_rx.try_recv() {
            self.command(c);
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (go, back, black, first, open, settings, reload, escape, mode) = ctx.input(|i| {
            let any = |ks: &[Key]| ks.iter().any(|k| i.key_pressed(*k));
            (
                any(&[
                    Key::Space,
                    Key::ArrowRight,
                    Key::ArrowDown,
                    Key::PageDown,
                    Key::Enter,
                    Key::N,
                ]),
                any(&[
                    Key::ArrowLeft,
                    Key::ArrowUp,
                    Key::PageUp,
                    Key::Backspace,
                    Key::P,
                ]),
                any(&[Key::B, Key::Period]),
                any(&[Key::Home]),
                i.modifiers.command && i.key_pressed(Key::O),
                i.modifiers.command && i.key_pressed(Key::Comma),
                i.modifiers.command && i.key_pressed(Key::R),
                i.key_pressed(Key::Escape),
                i.modifiers.command && i.key_pressed(Key::E),
            )
        });
        if mode {
            self.set_live(!self.live());
        }
        if (open || reload) && self.live() {
            self.locked();
        } else if reload && self.deck.is_some() {
            self.load(ctx);
        }
        if escape {
            if self.settings_open {
                self.settings_open = false;
            } else {
                self.selected = None;
            }
        }
        if go {
            self.go();
        }
        if back {
            self.back();
        }
        if black {
            let on = !self.player.blackout;
            self.player.set_blackout(on);
        }
        if first {
            self.goto(Some(0));
        }
        if open && !self.live() {
            self.pick_file(ctx);
        }
        if settings {
            self.settings_open = !self.settings_open;
        }
    }

    fn tex(&self, p: Option<usize>) -> Option<&TextureHandle> {
        p.and_then(|p| self.fulls.get(&p)).map(|f| &f.tex)
    }

    // ---------------------------------------------------------------- output window

    fn output_window(&mut self, ctx: &egui::Context) {
        let Some(m) = self.show.output.display else {
            self.output_started = None;
            return;
        };
        let Some(d) = self.displays.get(m).cloned() else {
            return;
        };
        // Give the display a moment after switching on (fullscreen on macOS).
        let since = *self.output_started.get_or_insert_with(Instant::now);
        if since.elapsed() < Duration::from_millis(300) {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        let zoom = ctx.zoom_factor().max(0.1);
        let borderless =
            cfg!(target_os = "macos") || std::env::var_os("LANTERN_BORDERLESS").is_some();
        let mut b = egui::ViewportBuilder::default().with_title("Lantern Output");
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
        let (prev, cur) = (
            self.tex(self.player.previous).cloned(),
            self.tex(self.player.current).cloned(),
        );
        let (mix, black) = (self.player.mix(), self.player.black_level());
        let mut closed = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of(("lantern-output", m)),
            b,
            |ui, _| {
                let rect = ui.ctx().content_rect();
                let painter = ui.ctx().layer_painter(egui::LayerId::background());
                paint_output(&painter, rect, prev.as_ref(), cur.as_ref(), mix, black);
                ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                if ui.ctx().input(|i| i.viewport().close_requested()) {
                    closed = true;
                }
            },
        );
        if borderless {
            crate::platform::raise_output_windows(
                "Lantern Output",
                [d.pos[0], d.pos[1], d.size[0], d.size[1]],
            );
        }
        if closed {
            self.show.output.display = None;
        }
    }

    // ---------------------------------------------------------------- UI parts

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new("Lantern").size(18.0).strong().color(AMBER));
            ui.add_space(10.0);
            if self.live() {
                ui.label(
                    RichText::new(" LIVE ")
                        .strong()
                        .size(15.0)
                        .color(Color32::WHITE)
                        .background_color(MODE_LIVE),
                );
                ui.add_space(6.0);
            }
            if self.deck.is_some() || self.loading.is_some() {
                let name = self
                    .show
                    .presentation
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                ui.add(egui::Label::new(RichText::new(name).strong()).truncate());
                if !self.slides.is_empty() {
                    ui.label(RichText::new(format!("{} Folien", self.slides.len())).color(MUTED));
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(flat_button("Einstellungen"))
                    .on_hover_text("Ausgänge, Timecode, Netzwerk (Strg/Cmd+,)")
                    .clicked()
                {
                    self.settings_open = !self.settings_open;
                }
                if !self.live()
                    && ui
                        .add(flat_button("Öffnen"))
                        .on_hover_text("PowerPoint, Keynote, OpenDocument oder PDF (Strg/Cmd+O)")
                        .clicked()
                {
                    self.pick_file(&ctx);
                }
                ui.add_space(4.0);
                let mut live = self.live();
                mode_switch(ui, &mut live);
                self.set_live(live);
                ui.add_space(12.0);
                if !self.live() {
                    ui.label(RichText::new(clock()).monospace().size(15.0).color(MUTED));
                }
                ui.add_space(8.0);
                // Outputs and timecode, as quiet status dots.
                if self.show.output.ndi {
                    let ok = self.ndi.is_some();
                    status(ui, "NDI", if ok { OK } else { DANGER }).on_hover_text(
                        self.ndi_error
                            .clone()
                            .unwrap_or_else(|| format!("Sendet als „{}“", self.ndi_name())),
                    );
                }
                match self.show.output.display {
                    Some(d) if d < self.displays.len() => {
                        status(ui, &format!("Bildschirm {}", d + 1), OK);
                    }
                    Some(d) => {
                        status(ui, &format!("Bildschirm {} fehlt", d + 1), DANGER);
                    }
                    None if !self.show.output.ndi => {
                        let r = status(ui, "Kein Ausgang", MUTED)
                            .on_hover_text("Klicken: einen Bildschirm oder NDI als Ausgang wählen");
                        if r.clicked() {
                            self.settings_open = true;
                            self.settings_tab = SettingsTab::Output;
                        }
                    }
                    None => {}
                }
                if self.show.timecode.source != TcSource::Off && !self.live() {
                    let (text, color) = match self.tc.reading() {
                        None => ("--:--:--:--".to_string(), MUTED),
                        Some(r) => (
                            show::format_tc(r.secs, r.fps),
                            if r.running { TC } else { MUTED },
                        ),
                    };
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(text)
                            .monospace()
                            .size(15.0)
                            .strong()
                            .color(color),
                    )
                    .on_hover_text(self.tc.status());
                }
            });
        });
    }

    fn ndi_name(&self) -> String {
        let n = self.show.output.ndi_name.trim();
        if n.is_empty() {
            "Lantern".into()
        } else {
            n.into()
        }
    }

    fn slide_list(&mut self, ui: &mut egui::Ui) {
        let next = self.next_index();
        let mut clicked: Option<usize> = None;
        let mut double: Option<usize> = None;
        let scroll = std::mem::take(&mut self.scroll_to_live);
        let aspect = self.deck.as_ref().map(|d| d.aspect).unwrap_or(16.0 / 9.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (i, s) in self.slides.iter().enumerate() {
                    let live = self.player.current == Some(i);
                    let is_next = next == Some(i);
                    let sel = self.selected == Some(i);
                    let tw = 96.0;
                    let th = tw / aspect;
                    let (rect, resp) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), th + 14.0),
                        Sense::click(),
                    );
                    let p = ui.painter();
                    let fill = if live {
                        CURRENT_BG
                    } else if is_next {
                        NEXT_BG
                    } else if sel {
                        CARD
                    } else if resp.hovered() {
                        HOVER
                    } else {
                        Color32::TRANSPARENT
                    };
                    p.rect_filled(rect, 6.0, fill);
                    let accent = if live {
                        Some(CURRENT)
                    } else if is_next {
                        Some(NEXT)
                    } else {
                        None
                    };
                    if let Some(c) = accent {
                        p.rect_filled(
                            Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
                            CornerRadius::same(2),
                            c,
                        );
                    }
                    if sel {
                        p.rect_stroke(rect, 6.0, Stroke::new(1.0, AMBER), egui::StrokeKind::Inside);
                    }
                    p.text(
                        rect.left_top() + Vec2::new(12.0, 8.0),
                        egui::Align2::LEFT_TOP,
                        format!("{}", i + 1),
                        egui::FontId::monospace(12.0),
                        accent.unwrap_or(MUTED),
                    );
                    let thumb = Rect::from_min_size(
                        rect.left_top() + Vec2::new(36.0, 7.0),
                        Vec2::new(tw, th),
                    );
                    match self.thumbs.get(&i) {
                        Some(t) => {
                            p.image(t.id(), thumb, full_uv(), Color32::WHITE);
                        }
                        None => {
                            p.rect_filled(thumb, 2.0, Color32::BLACK);
                        }
                    }
                    p.rect_stroke(
                        thumb,
                        2.0,
                        Stroke::new(1.0, LINE),
                        egui::StrokeKind::Outside,
                    );
                    // Title (two lines at most) and one quiet line about the trigger.
                    let tx = thumb.right() + 10.0;
                    let width = (rect.right() - tx - 8.0).max(20.0);
                    let title = if s.title.is_empty() {
                        format!("Folie {}", s.number)
                    } else {
                        s.title.clone()
                    };
                    let mut job = egui::text::LayoutJob::single_section(
                        title,
                        egui::TextFormat::simple(egui::FontId::proportional(14.0), TEXT),
                    );
                    job.wrap = egui::text::TextWrapping {
                        max_width: width,
                        max_rows: 2,
                        break_anywhere: false,
                        overflow_character: Some('…'),
                    };
                    let g = p.layout_job(job);
                    let gh = g.rect.height();
                    p.galley(pos2(tx, rect.top() + 7.0), g, TEXT);
                    let cfg = self.show.slides.get(i);
                    let (meta, mc) = match cfg.map(|c| c.trigger) {
                        Some(Trigger::Timecode) => {
                            let tc = cfg.map(|c| c.tc.as_str()).unwrap_or("");
                            (
                                format!("TC {}", if tc.is_empty() { "fehlt" } else { tc }),
                                TC,
                            )
                        }
                        _ => ("GO".to_string(), MUTED),
                    };
                    let mut x = tx;
                    let y = rect.top() + 11.0 + gh;
                    let r = p.text(
                        pos2(x, y),
                        egui::Align2::LEFT_TOP,
                        meta,
                        egui::FontId::proportional(11.5),
                        mc,
                    );
                    x = r.right() + 8.0;
                    if !s.notes.is_empty() {
                        p.text(
                            pos2(x, y),
                            egui::Align2::LEFT_TOP,
                            "Notiz",
                            egui::FontId::proportional(11.5),
                            MUTED,
                        );
                    }
                    let resp = if s.notes.is_empty() {
                        resp
                    } else {
                        resp.on_hover_text(&s.notes)
                    };
                    if scroll && live {
                        resp.scroll_to_me(Some(Align::Center));
                    }
                    if resp.double_clicked() {
                        double = Some(i);
                    } else if resp.clicked() {
                        clicked = Some(i);
                    }
                }
            });
        if let Some(i) = clicked.filter(|_| !self.live()) {
            // Clicking the selected slide again closes its settings.
            self.selected = if self.selected == Some(i) {
                None
            } else {
                Some(i)
            };
        }
        if let Some(i) = double {
            self.goto(Some(i));
        }
    }

    fn center(&mut self, ui: &mut egui::Ui) {
        if self.live() {
            self.clock_strip(ui);
            ui.add_space(14.0);
        }
        let next = self.next_index();
        let avail = ui.available_size();
        let aspect = {
            let (w, h) = self.output_size();
            w as f32 / h as f32
        };
        let gap = 16.0;
        // Live big, next smaller beside it; together at most about half the height.
        let max_h = (avail.y * 0.5).max(120.0);
        let live_w = ((avail.x - gap) * 0.64).min(max_h * aspect).max(160.0);
        let next_w = ((avail.x - gap) * 0.36).min(live_w * 0.62).max(100.0);
        let total = self.slides.len();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(live_w);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("LIVE").small().strong().color(CURRENT));
                    let what = match self.player.current {
                        Some(c) => format!("Folie {} von {total}", c + 1),
                        None => "Schwarz".into(),
                    };
                    ui.label(RichText::new(what).small().color(MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.player.current.is_some() {
                            let s = self.player.since.elapsed().as_secs();
                            ui.label(
                                RichText::new(format!("{}:{:02}", s / 60, s % 60))
                                    .small()
                                    .monospace()
                                    .color(MUTED),
                            )
                            .on_hover_text("Zeit auf dieser Folie");
                        }
                    });
                });
                let (r, _) =
                    ui.allocate_exact_size(Vec2::new(live_w, live_w / aspect), Sense::hover());
                paint_output(
                    ui.painter(),
                    r,
                    self.tex(self.player.previous),
                    self.tex(self.player.current),
                    self.player.mix(),
                    self.player.black_level(),
                );
                ui.painter().rect_stroke(
                    r,
                    4.0,
                    Stroke::new(
                        2.0,
                        if self.player.blackout {
                            DANGER
                        } else {
                            CURRENT
                        },
                    ),
                    egui::StrokeKind::Outside,
                );
                if self.player.blackout {
                    ui.painter().text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        "BLACKOUT",
                        egui::FontId::proportional(24.0),
                        DANGER,
                    );
                }
            });
            ui.add_space(gap);
            ui.vertical(|ui| {
                ui.set_width(next_w);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("NEXT").small().strong().color(NEXT));
                    if let Some(n) = next {
                        ui.label(
                            RichText::new(format!("Folie {}", n + 1))
                                .small()
                                .color(MUTED),
                        );
                    }
                });
                let (r, _) =
                    ui.allocate_exact_size(Vec2::new(next_w, next_w / aspect), Sense::hover());
                paint_output(ui.painter(), r, None, self.tex(next), 1.0, 0.0);
                ui.painter()
                    .rect_stroke(r, 4.0, Stroke::new(1.0, NEXT), egui::StrokeKind::Outside);
                if next.is_none() {
                    ui.painter().text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        "Ende",
                        egui::FontId::proportional(18.0),
                        MUTED,
                    );
                }
                if let Some(n) = next {
                    ui.add_space(4.0);
                    let title = self
                        .slides
                        .get(n)
                        .map(|s| s.title.clone())
                        .unwrap_or_default();
                    if !title.is_empty() {
                        ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
                    }
                    let cfg = self.show.slides.get(n);
                    let t = match cfg.map(|c| c.trigger).unwrap_or_default() {
                        Trigger::Go => "kommt mit GO".to_string(),
                        Trigger::Timecode => {
                            format!("kommt bei TC {}", cfg.map(|s| s.tc.as_str()).unwrap_or(""))
                        }
                    };
                    ui.label(RichText::new(t).small().color(MUTED));
                }
            });
        });
        ui.add_space(14.0);
        // Transport.
        ui.horizontal(|ui| {
            let h = 54.0;
            let (r, go) = ui.allocate_exact_size(
                Vec2::new((live_w * 0.42).clamp(180.0, 320.0), h),
                if next.is_some() {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            let fill = match (next.is_some(), go.hovered(), go.is_pointer_button_down_on()) {
                (false, ..) => CARD,
                (true, _, true) => Color32::from_rgb(21, 128, 61),
                (true, true, _) => Color32::from_rgb(28, 180, 84),
                _ => GO,
            };
            let ink = if next.is_some() {
                Color32::WHITE
            } else {
                MUTED
            };
            ui.painter().rect_filled(r, 8.0, fill);
            ui.painter().text(
                r.left_center() + Vec2::new(18.0, 0.0),
                egui::Align2::LEFT_CENTER,
                "GO",
                egui::FontId::proportional(26.0),
                ink,
            );
            ui.painter().text(
                r.right_center() - Vec2::new(16.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                match next {
                    Some(n) => format!("Folie {}", n + 1),
                    None => "Ende".into(),
                },
                egui::FontId::proportional(14.0),
                ink,
            );
            if go
                .on_hover_text("Leertaste, Pfeil rechts, Bild ab oder Klicker")
                .clicked()
            {
                self.go();
            }
            if ui
                .add(tool_button("Zurück", None, h))
                .on_hover_text("Pfeil links, Bild auf")
                .clicked()
            {
                self.back();
            }
            let bo = self.player.blackout;
            if ui
                .add(tool_button("Blackout", bo.then_some(DANGER), h))
                .on_hover_text("Taste B")
                .clicked()
            {
                self.player.set_blackout(!bo);
            }
            if ui
                .add(tool_button("Leeren", None, h))
                .on_hover_text("Zurück auf Schwarz vor der ersten Folie")
                .clicked()
            {
                self.goto(None);
            }
        });
        ui.add_space(14.0);
        // Speaker notes of the live slide.
        let notes = self
            .player
            .current
            .and_then(|c| self.slides.get(c))
            .map(|s| s.notes.as_str())
            .unwrap_or("");
        Frame::new()
            .fill(PANEL)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height((ui.available_height() - 28.0).max(40.0));
                ui.label(RichText::new("NOTIZEN").small().strong().color(MUTED));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if notes.is_empty() {
                        ui.label(RichText::new("Keine Notizen auf dieser Folie").color(MUTED));
                    } else {
                        ui.label(RichText::new(notes).size(19.0));
                    }
                });
            });
    }

    /// Live: big clocks across the top (time of day, timecode, time on the
    /// slide, slide count), drawn like Show Master's clock strip.
    fn clock_strip(&mut self, ui: &mut egui::Ui) {
        let h = (ui.available_height() * 0.13).clamp(78.0, 112.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
        let gap = 8.0;
        // Timecode needs the most room.
        let weights = [1.0, 1.4, 0.8, 0.8];
        let total: f32 = weights.iter().sum();
        let avail = rect.width() - gap * (weights.len() as f32 - 1.0);
        let mut x = rect.left();
        let cells: Vec<Rect> = weights
            .iter()
            .map(|w| {
                let width = avail * w / total;
                let r = Rect::from_min_size(pos2(x, rect.top()), Vec2::new(width, h));
                x += width + gap;
                r
            })
            .collect();
        // One digit size for all of them (calmer than four sizes).
        let digits_h = (h - 30.0) * 0.95;
        let fit = |i: usize, chars: usize| fit_digits(chars, cells[i].width() - 24.0, digits_h);
        let size = fit(0, 8).min(fit(1, 11)).min(fit(2, 5)).min(fit(3, 7));

        clock_cell(ui, cells[0], "Uhrzeit", &clock(), TEXT, None, size);

        let src = self.show.timecode.source;
        let reading = self.tc.reading();
        let (tc, tc_color, accent) = match (src, reading) {
            (TcSource::Off, _) => ("--:--:--:--".to_string(), MUTED, None),
            (_, None) => ("--:--:--:--".to_string(), MUTED, None),
            (_, Some(r)) => (
                show::format_tc(r.secs, r.fps),
                if r.running { TEXT } else { MUTED },
                r.running.then_some(CURRENT),
            ),
        };
        let free = clock_cell(ui, cells[1], "Timecode", &tc, tc_color, accent, size);
        let (badge_text, badge_color, on) = match (src, reading) {
            (TcSource::Off, _) => ("AUS".to_string(), MUTED, false),
            (_, None) => ("KEIN SIGNAL".to_string(), DANGER, true),
            (s, Some(r)) => (
                format!(
                    "{} {}{}",
                    if s == TcSource::Ltc {
                        "LTC"
                    } else {
                        "RTP-MIDI"
                    },
                    r.fps.round(),
                    if r.running { "" } else { " STEHT" }
                ),
                if r.running { OK } else { AMBER },
                r.running,
            ),
        };
        badge(
            ui,
            free.right(),
            free.center().y,
            &badge_text,
            on,
            badge_color,
        )
        .on_hover_text(self.tc.status());

        let on_slide = self.player.current.map(|_| {
            let s = self.player.since.elapsed().as_secs();
            format!("{}:{:02}", s / 60, s % 60)
        });
        clock_cell(
            ui,
            cells[2],
            "Auf der Folie",
            on_slide.as_deref().unwrap_or("-:--"),
            if on_slide.is_some() { TEXT } else { MUTED },
            None,
            size,
        );
        let count = match self.player.current {
            Some(c) => format!("{}/{}", c + 1, self.slides.len()),
            None => format!("-/{}", self.slides.len()),
        };
        clock_cell(
            ui,
            cells[3],
            "Folie",
            &count,
            if self.player.current.is_some() {
                TEXT
            } else {
                MUTED
            },
            self.player.current.map(|_| CURRENT),
            size,
        );
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.selected.filter(|i| *i < self.show.slides.len()) else {
            return;
        };
        let reading = self.tc.reading();
        let file_t = self
            .slides
            .get(i)
            .map(|s| s.transition)
            .unwrap_or(Transition::Cut);
        let title = self
            .slides
            .get(i)
            .map(|s| s.title.clone())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Folie {}", i + 1))
                    .size(17.0)
                    .strong(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(flat_button("Schließen"))
                    .on_hover_text("Esc")
                    .clicked()
                {
                    self.selected = None;
                }
            });
        });
        if !title.is_empty() {
            ui.add(egui::Label::new(RichText::new(title).color(MUTED)).truncate());
        }
        ui.add_space(6.0);
        if let Some(t) = self.thumbs.get(&i) {
            let w = ui.available_width();
            let aspect = self.deck.as_ref().map(|d| d.aspect).unwrap_or(16.0 / 9.0);
            ui.add(
                egui::Image::new(t)
                    .fit_to_exact_size(Vec2::new(w, w / aspect))
                    .corner_radius(4),
            );
        }
        ui.add_space(6.0);
        if ui
            .add(tool_button("Jetzt zeigen", None, 32.0))
            .on_hover_text("Auch: Doppelklick in der Liste")
            .clicked()
        {
            self.goto(Some(i));
        }
        let cfg = &mut self.show.slides[i];
        section(ui, "Auslösen");
        segmented(
            ui,
            &mut cfg.trigger,
            &[(Trigger::Go, "GO"), (Trigger::Timecode, "Timecode")],
        );
        match cfg.trigger {
            Trigger::Go => {
                ui.label(
                    RichText::new("Mit GO-Knopf, Taste, Klicker oder OSC.")
                        .small()
                        .color(MUTED),
                );
            }
            Trigger::Timecode => {
                ui.horizontal(|ui| {
                    let fps = reading.map(|r| r.fps.round()).unwrap_or(30.0);
                    let ok = cfg.tc.is_empty() || show::parse_tc(&cfg.tc, fps).is_some();
                    ui.add(
                        egui::TextEdit::singleline(&mut cfg.tc)
                            .hint_text("HH:MM:SS:FF")
                            .font(egui::TextStyle::Monospace)
                            .desired_width(118.0)
                            .text_color(if ok { TEXT } else { DANGER }),
                    );
                    if let Some(r) = reading {
                        if ui
                            .button("Aktueller TC")
                            .on_hover_text("Den gerade laufenden Timecode übernehmen")
                            .clicked()
                        {
                            cfg.tc = show::format_tc(r.secs, r.fps);
                        }
                    }
                });
                if self.show.timecode.source == TcSource::Off {
                    ui.label(
                        RichText::new("Der Timecode-Eingang ist aus (Einstellungen › Timecode).")
                            .small()
                            .color(AMBER),
                    );
                }
            }
        }
        section(ui, "Übergang");
        let mut kind = match cfg.transition {
            TransitionCfg::FromFile => 0,
            TransitionCfg::Cut => 1,
            TransitionCfg::Fade(_) => 2,
        };
        segmented(
            ui,
            &mut kind,
            &[(0, "Wie Datei"), (1, "Schnitt"), (2, "Blende")],
        );
        let mut secs = match cfg.transition {
            TransitionCfg::Fade(s) => s,
            _ => 0.7,
        };
        match kind {
            0 => {
                ui.label(
                    RichText::new(match file_t {
                        Transition::Cut => "In der Datei: Schnitt".to_string(),
                        Transition::Fade(s) => format!("In der Datei: Blende {s:.1} s"),
                    })
                    .small()
                    .color(MUTED),
                );
            }
            2 => {
                ui.add(
                    egui::Slider::new(&mut secs, 0.1..=5.0)
                        .suffix(" s")
                        .step_by(0.1),
                );
            }
            _ => {}
        }
        cfg.transition = match kind {
            1 => TransitionCfg::Cut,
            2 => TransitionCfg::Fade(secs),
            _ => TransitionCfg::FromFile,
        };
        let notes = self
            .slides
            .get(i)
            .map(|s| s.notes.clone())
            .unwrap_or_default();
        if !notes.is_empty() {
            section(ui, "Notizen");
            ui.label(RichText::new(notes).color(MUTED));
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("Einstellungen")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_size([520.0, 460.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                Frame::window(&ctx.global_style())
                    .fill(PANEL)
                    .inner_margin(Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (t, label) in [
                        (SettingsTab::Output, "Ausgang"),
                        (SettingsTab::Timecode, "Timecode"),
                        (SettingsTab::Network, "Netzwerk"),
                        (SettingsTab::Program, "Programm"),
                    ] {
                        let on = self.settings_tab == t;
                        if ui
                            .add(
                                egui::Button::new(RichText::new(label).color(if on {
                                    TEXT
                                } else {
                                    MUTED
                                }))
                                .fill(if on { CARD } else { Color32::TRANSPARENT })
                                .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            self.settings_tab = t;
                        }
                    }
                });
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
                let live = self.live();
                if live {
                    ui.label(
                        RichText::new(
                            "Live: nur ansehen. Zum Ändern auf „Bearbeiten“ wechseln (Strg/Cmd+E).",
                        )
                        .color(MODE_LIVE),
                    );
                    ui.add_space(6.0);
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_enabled_ui(!live, |ui| match self.settings_tab {
                            SettingsTab::Output => self.settings_output(ui),
                            SettingsTab::Timecode => self.settings_timecode(ui),
                            SettingsTab::Network => self.settings_network(ui),
                            SettingsTab::Program => self.settings_program(ui),
                        });
                    });
            });
        self.settings_open = open;
    }

    fn settings_output(&mut self, ui: &mut egui::Ui) {
        let o = &mut self.show.output;
        let label = |d: Option<usize>, ds: &[Disp]| match d {
            None => "Aus".to_string(),
            Some(i) => match ds.get(i) {
                Some(x) => format!(
                    "{} – {} ({}×{}){}",
                    i + 1,
                    x.name,
                    x.px[0],
                    x.px[1],
                    if x.here { " · hier ist Lantern" } else { "" }
                ),
                None => format!("{} (nicht angeschlossen)", i + 1),
            },
        };
        egui::Grid::new("out")
            .num_columns(2)
            .spacing([14.0, 10.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Bildschirm").color(MUTED));
                egui::ComboBox::from_id_salt("display")
                    .width(320.0)
                    .selected_text(label(o.display, &self.displays))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut o.display, None, "Aus");
                        for i in 0..self.displays.len() {
                            ui.selectable_value(
                                &mut o.display,
                                Some(i),
                                label(Some(i), &self.displays),
                            );
                        }
                    });
                ui.end_row();
                ui.label(RichText::new("Auflösung").color(MUTED));
                egui::ComboBox::from_id_salt("res")
                    .width(320.0)
                    .selected_text(o.resolution.label())
                    .show_ui(ui, |ui| {
                        for r in Resolution::ALL {
                            ui.selectable_value(&mut o.resolution, r, r.label());
                        }
                    });
                ui.end_row();
                ui.label(RichText::new("NDI").color(MUTED));
                ui.horizontal(|ui| {
                    ui.checkbox(&mut o.ndi, "senden als");
                    ui.add_enabled(
                        o.ndi,
                        egui::TextEdit::singleline(&mut o.ndi_name)
                            .hint_text("Lantern")
                            .desired_width(160.0),
                    );
                });
                ui.end_row();
                if o.ndi {
                    ui.label(RichText::new("Bilder/s").color(MUTED));
                    segmented(
                        ui,
                        &mut o.fps,
                        &[(25, "25"), (30, "30"), (50, "50"), (60, "60")],
                    );
                    ui.end_row();
                }
            });
        ui.add_space(6.0);
        if self.show.output.ndi {
            if let Some(e) = &self.ndi_error {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(e).color(DANGER));
                    ui.hyperlink_to("NDI Tools laden", ndi::WEBSITE);
                });
            } else if self.ndi.is_some() {
                let (w, h) = self.output_size();
                ui.label(
                    RichText::new(format!("NDI-Quelle „{}“ sendet {w}×{h}", self.ndi_name()))
                        .color(OK),
                );
            }
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new("Der Ausgang zeigt nur die Folien – keine Symbole, kein Mauszeiger.")
                .small()
                .color(MUTED),
        );
    }

    fn settings_timecode(&mut self, ui: &mut egui::Ui) {
        let t = &mut self.show.timecode;
        segmented(
            ui,
            &mut t.source,
            &[
                (TcSource::Off, "Aus"),
                (TcSource::Ltc, "LTC (Audio)"),
                (TcSource::RtpMidi, "MIDI-Timecode (Netzwerk)"),
            ],
        );
        ui.add_space(8.0);
        match t.source {
            TcSource::Off => {
                ui.label(
                    RichText::new(
                        "Ohne Timecode laufen alle Folien mit GO. Folien mit Timecode-Auslöser warten.",
                    )
                    .color(MUTED),
                );
            }
            TcSource::Ltc => {
                egui::Grid::new("ltc")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("Eingang").color(MUTED));
                        let shown = if t.ltc_device.is_empty() {
                            "Standard".to_string()
                        } else {
                            t.ltc_device.clone()
                        };
                        egui::ComboBox::from_id_salt("ltcdev")
                            .width(300.0)
                            .selected_text(shown)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut t.ltc_device, String::new(), "Standard");
                                for d in timecode::audio_inputs() {
                                    ui.selectable_value(&mut t.ltc_device, d.clone(), d);
                                }
                            });
                        ui.end_row();
                        ui.label(RichText::new("Kanal").color(MUTED));
                        let mut ch = t.ltc_channel + 1;
                        ui.add(egui::DragValue::new(&mut ch).range(1..=64));
                        t.ltc_channel = ch - 1;
                        ui.end_row();
                        ui.label(RichText::new("Pegel").color(MUTED));
                        let l = self.tc.level();
                        ui.add(egui::ProgressBar::new(l.min(1.0)).desired_width(220.0));
                        ui.end_row();
                    });
            }
            TcSource::RtpMidi => {
                egui::Grid::new("rtp")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("Session").color(MUTED));
                        ui.text_edit_singleline(&mut t.rtp_name);
                        ui.end_row();
                        ui.label(RichText::new("Port").color(MUTED));
                        ui.add(egui::DragValue::new(&mut t.rtp_port).range(1024..=65000));
                        ui.end_row();
                    });
                ui.add_space(6.0);
                let peers = self.tc.connected();
                if peers.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "Noch nicht verbunden. In Show Master unter MIDI die Session „{}“ verbinden – oder hier eine wählen:",
                            t.rtp_name
                        ))
                        .color(MUTED),
                    );
                } else {
                    ui.label(
                        RichText::new(format!("Verbunden mit {}", peers.join(", "))).color(OK),
                    );
                }
                let mut connect: Option<String> = None;
                for (name, addr) in self.net.rtp_sessions() {
                    if name == t.rtp_name {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        ui.label(&name);
                        ui.label(RichText::new(&addr).small().color(MUTED));
                        if !peers.contains(&name) && ui.small_button("Verbinden").clicked() {
                            connect = Some(addr.clone());
                        }
                    });
                }
                if let Some(a) = connect {
                    if let Err(e) = self.tc.connect(&a) {
                        self.note(e);
                    }
                }
            }
        }
        if self.show.timecode.source != TcSource::Off {
            ui.add_space(8.0);
            ui.label(RichText::new(self.tc.status()).small().color(MUTED));
        }
    }

    fn settings_network(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Show Master").strong());
        ui.label(
            RichText::new(format!(
                "Show Master findet diesen Rechner im Netz als „{}“: Lantern-Cue › Network.",
                self.net.name()
            ))
            .color(MUTED),
        );
        if let Some(ip) = local_ip() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("IP-Adresse").color(MUTED));
                ui.label(RichText::new(ip).monospace());
            });
        }
        if let Some(p) = self.net.problem() {
            ui.label(
                RichText::new(format!(
                    "Netzwerk-Suche gestört: {p} – dann die IP-Adresse eintragen."
                ))
                .small()
                .color(AMBER),
            );
        }
        section(ui, "Fernsteuerung (OSC)");
        ui.horizontal(|ui| {
            ui.label(RichText::new("UDP-Port").color(MUTED));
            ui.add(egui::DragValue::new(&mut self.show.osc_port).range(1024..=65000));
            match (&self.osc, &self.osc_error) {
                (Some(_), _) => {
                    ui.label(RichText::new("empfängt").color(OK));
                }
                (None, Some((_, e))) => {
                    ui.label(RichText::new(e).color(DANGER));
                }
                _ => {}
            }
        });
        ui.add_space(4.0);
        egui::Grid::new("osc_cmds")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for (a, what) in [
                    ("/lantern/go", "nächste Folie"),
                    ("/lantern/back", "vorige Folie"),
                    ("/lantern/slide <n>", "Folie n zeigen"),
                    ("/lantern/first", "erste Folie"),
                    (
                        "/lantern/blackout [0|1]",
                        "Blackout (ohne Wert: umschalten)",
                    ),
                    ("/lantern/clear", "Schwarz vor der ersten Folie"),
                ] {
                    ui.label(RichText::new(a).small().monospace());
                    ui.label(RichText::new(what).small().color(MUTED));
                    ui.end_row();
                }
            });
    }

    fn settings_program(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("LibreOffice").strong());
        match &self.office_found {
            Some(p) => {
                ui.label(
                    RichText::new("Installiert – PowerPoint-Dateien werden damit umgewandelt.")
                        .color(MUTED),
                );
                ui.label(RichText::new(p.display().to_string()).small().color(MUTED));
            }
            None => {
                ui.label(
                    RichText::new(
                        "Nicht installiert – nötig für PowerPoint-Dateien (PDFs gehen auch so).",
                    )
                    .color(AMBER),
                );
            }
        }
        section(ui, "Präsentation");
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.deck.is_some(), egui::Button::new("Neu einlesen"))
                .on_hover_text("Strg/Cmd+R")
                .clicked()
            {
                self.load(&ctx);
            }
            if let Some(f) = &self.show_file {
                ui.label(
                    RichText::new(format!(
                        "Einstellungen gespeichert in {}",
                        f.file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or_default()
                    ))
                    .small()
                    .color(MUTED),
                );
            }
        });
        section(ui, "Tasten");
        egui::Grid::new("keys")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for (k, what) in [
                    ("Leertaste · → · Bild ab · Enter", "GO"),
                    ("← · Bild auf · Rücktaste", "Zurück"),
                    ("B", "Blackout"),
                    ("Pos1", "Erste Folie"),
                    ("Esc", "Folien-Einstellungen schließen"),
                    ("Strg/Cmd+E", "Bearbeiten / Live umschalten"),
                ] {
                    ui.label(RichText::new(k).small());
                    ui.label(RichText::new(what).small().color(MUTED));
                    ui.end_row();
                }
            });
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("Lantern {}", env!("CARGO_PKG_VERSION")))
                .small()
                .color(MUTED),
        );
    }

    /// One line at the top of the centre: LibreOffice missing, file changed,
    /// errors, loading.
    fn banners(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.office_found.is_none() {
            let st = self.install.lock().map(|s| s.clone()).unwrap_or_default();
            banner(ui, AMBER, |ui| {
                ui.label(RichText::new("LibreOffice fehlt").strong().color(AMBER));
                ui.label("– nötig, um PowerPoint-Dateien originalgetreu zu öffnen.");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if st.running {
                        match st.progress {
                            Some(p) => {
                                ui.add(
                                    egui::ProgressBar::new(p)
                                        .desired_width(140.0)
                                        .show_percentage(),
                                );
                            }
                            None => {
                                ui.spinner();
                            }
                        }
                        ui.label(RichText::new(&st.text).small());
                    } else {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Jetzt installieren")
                                        .strong()
                                        .color(Color32::BLACK),
                                )
                                .fill(AMBER),
                            )
                            .clicked()
                        {
                            office::install(self.install.clone());
                        }
                        if ui.link("Download-Seite").clicked() {
                            office::open_url(office::DOWNLOAD_PAGE);
                        }
                        if let Some(e) = &st.error {
                            ui.label(RichText::new(e).small().color(DANGER));
                        } else if !st.text.is_empty() {
                            ui.label(RichText::new(&st.text).small());
                        }
                    }
                });
            });
        }
        if self.changed_on_disk && self.loading.is_none() {
            banner(ui, TC, |ui| {
                ui.label("Die Präsentation wurde geändert.");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.live() {
                        ui.label(
                            RichText::new("Neu einlesen geht im Modus „Bearbeiten“")
                                .small()
                                .color(MUTED),
                        );
                    } else if ui.button("Neu einlesen").clicked() {
                        self.load(&ctx);
                    }
                });
            });
        }
        if let Some(l) = &self.loading {
            if self.deck.is_some() {
                let s = l.status.lock().map(|s| s.clone()).unwrap_or_default();
                banner(ui, MUTED, |ui| {
                    ui.spinner();
                    ui.label(s);
                });
            }
        }
        if let (Some(e), true) = (&self.load_error, self.deck.is_some()) {
            let e = e.clone();
            banner(ui, DANGER, |ui| {
                ui.label(RichText::new(e).color(DANGER));
            });
        }
    }

    /// Short messages, bottom right, without moving anything.
    fn toast(&self, ctx: &egui::Context) {
        let Some((m, at)) = &self.message else { return };
        let age = at.elapsed().as_secs_f32();
        if age > 4.0 {
            return;
        }
        let alpha = ((4.0 - age) / 0.5).clamp(0.0, 1.0);
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -16.0])
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_opacity(alpha);
                Frame::new()
                    .fill(CARD)
                    .stroke(Stroke::new(1.0, LINE))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.label(m);
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.28);
            ui.label(RichText::new("Lantern").size(38.0).strong().color(AMBER));
            ui.label(
                RichText::new("Präsentationen für die Bühne")
                    .size(15.0)
                    .color(MUTED),
            );
            ui.add_space(26.0);
            if let Some(l) = &self.loading {
                ui.spinner();
                let s = l.status.lock().map(|s| s.clone()).unwrap_or_default();
                ui.label(
                    RichText::new(format!("{s}  ({} s)", l.started.elapsed().as_secs()))
                        .color(MUTED),
                );
                return;
            }
            if let Some(e) = &self.load_error {
                ui.label(RichText::new(e).color(DANGER));
                ui.add_space(10.0);
            }
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("Präsentation öffnen")
                            .size(17.0)
                            .strong()
                            .color(Color32::BLACK),
                    )
                    .fill(AMBER)
                    .corner_radius(8)
                    .min_size(Vec2::new(240.0, 46.0)),
                )
                .clicked()
            {
                self.pick_file(&ctx);
            }
            ui.add_space(10.0);
            ui.label(
                RichText::new("oder eine Datei hierher ziehen  ·  PowerPoint, Keynote, PDF")
                    .color(MUTED),
            );
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SettingsTab {
    #[default]
    Output,
    Timecode,
    Network,
    Program,
}

/// The address other computers reach this one at (no packet is sent).
fn local_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then(|| ip.to_string())
}

fn full_uv() -> Rect {
    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
}

/// Draw the output picture: previous slide, the current one faded over it, blackout on top.
fn paint_output(
    p: &egui::Painter,
    rect: Rect,
    prev: Option<&TextureHandle>,
    cur: Option<&TextureHandle>,
    mix: f32,
    black: f32,
) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    p.rect_filled(rect, 0.0, Color32::BLACK);
    let fit = |t: &TextureHandle| {
        let [w, h] = t.size();
        let (s, ox, oy) = deck::fit(
            w as f32,
            h as f32,
            rect.width().round() as u32,
            rect.height().round() as u32,
        );
        Rect::from_min_size(
            rect.min + Vec2::new(ox as f32, oy as f32),
            Vec2::new(w as f32 * s, h as f32 * s),
        )
    };
    if mix < 1.0 {
        if let Some(t) = prev {
            p.image(t.id(), fit(t), uv, Color32::WHITE);
        }
    }
    if let Some(t) = cur {
        let a = (mix.clamp(0.0, 1.0) * 255.0).round() as u8;
        p.image(t.id(), fit(t), uv, Color32::from_white_alpha(a));
    } else if mix < 1.0 && prev.is_some() {
        // Fading to black (cleared).
        p.rect_filled(rect, 0.0, Color32::from_black_alpha((mix * 255.0) as u8));
    }
    if black > 0.0 {
        p.rect_filled(
            rect,
            0.0,
            Color32::from_black_alpha((black.clamp(0.0, 1.0) * 255.0).round() as u8),
        );
    }
}

/// A coloured dot and a label (outputs in the top bar).
fn status(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let font = egui::FontId::proportional(13.0);
    let g = ui.painter().layout_no_wrap(text.to_string(), font, TEXT);
    let size = Vec2::new(g.size().x + 16.0, 22.0);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    ui.painter()
        .circle_filled(pos2(r.left() + 4.0, r.center().y), 3.5, color);
    ui.painter().galley(
        pos2(r.left() + 14.0, r.center().y - g.size().y / 2.0),
        g,
        TEXT,
    );
    resp
}

/// Font size that fits `chars` monospace digits into `width` and `height`.
fn fit_digits(chars: usize, width: f32, height: f32) -> f32 {
    // Monospace glyphs are about 0.6 em wide.
    let by_width = width / (chars.max(1) as f32 * 0.62);
    by_width.min(height).clamp(12.0, 72.0)
}

/// A clock cell: caption on top, large tabular digits below, an optional
/// accent bar on the left. Returns the free part of the caption row.
fn clock_cell(
    ui: &mut egui::Ui,
    rect: Rect,
    caption: &str,
    value: &str,
    color: Color32,
    accent: Option<Color32>,
    size: f32,
) -> Rect {
    let p = ui.painter_at(rect);
    p.rect(
        rect,
        6.0,
        CLOCK_BG,
        Stroke::new(1.0, LINE),
        egui::StrokeKind::Inside,
    );
    if let Some(a) = accent {
        p.rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(4.0, rect.height())),
            CornerRadius {
                nw: 6,
                sw: 6,
                ne: 0,
                se: 0,
            },
            a,
        );
    }
    let pad = 12.0;
    let cap = p.text(
        rect.min + Vec2::new(pad, 7.0),
        egui::Align2::LEFT_TOP,
        caption.to_uppercase(),
        egui::FontId::proportional(12.0),
        accent.unwrap_or(MUTED),
    );
    let digits = Rect::from_min_max(
        pos2(rect.left() + pad, rect.top() + 24.0),
        pos2(rect.right() - pad, rect.bottom() - 4.0),
    );
    p.text(
        digits.left_center(),
        egui::Align2::LEFT_CENTER,
        value,
        egui::FontId::monospace(size),
        color,
    );
    Rect::from_min_max(
        pos2(cap.right() + 8.0, rect.top() + 5.0),
        pos2(rect.right() - 8.0, rect.top() + 21.0),
    )
}

/// A small badge painted leftwards from `right` (e.g. "LTC 25").
fn badge(
    ui: &mut egui::Ui,
    right: f32,
    center_y: f32,
    text: &str,
    on: bool,
    color: Color32,
) -> egui::Response {
    let font = egui::FontId::proportional(11.0);
    let fg = if on { Color32::BLACK } else { color };
    let g = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let size = g.size() + Vec2::new(10.0, 3.0);
    let r = Rect::from_min_size(pos2(right - size.x, center_y - size.y / 2.0), size);
    if on {
        ui.painter().rect_filled(r, 3.0, color);
    } else {
        ui.painter()
            .rect_stroke(r, 3.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    }
    ui.painter().galley(r.min + Vec2::new(5.0, 1.5), g, fg);
    ui.interact(r, ui.scope_id().with(("badge", text)), Sense::hover())
}

/// A quiet button for the top bar and panel headers.
fn flat_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(TEXT))
        .fill(CARD)
        .stroke(Stroke::NONE)
        .corner_radius(6)
}

/// A large transport button; `on` paints it in that colour.
fn tool_button(text: &str, on: Option<Color32>, h: f32) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(text.to_string())
            .size(15.0)
            .color(if on.is_some() { Color32::WHITE } else { TEXT }),
    )
    .fill(on.unwrap_or(CARD))
    .stroke(Stroke::NONE)
    .corner_radius(8)
    .min_size(Vec2::new(104.0, h))
}

/// Bearbeiten | Live. Live is painted green so it is clear the show is locked.
fn mode_switch(ui: &mut egui::Ui, live: &mut bool) {
    // A scope keeps the top bar's centred layout, so the pills line up with
    // the other buttons.
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        // Right-to-left layout: "Live" first so it ends up on the right.
        for (v, label, on_fill, tip) in [
            (
                true,
                "Live",
                MODE_LIVE,
                "Show läuft: Folien-Einstellungen, Öffnen und Einstellungen gesperrt (Strg/Cmd+E)",
            ),
            (
                false,
                "Bearbeiten",
                MODE_EDIT,
                "Folien, Ausgänge und Timecode einrichten (Strg/Cmd+E)",
            ),
        ] {
            let on = *live == v;
            let b = egui::Button::new(RichText::new(label).strong().color(if on {
                Color32::WHITE
            } else {
                MUTED
            }))
            .fill(if on { on_fill } else { CARD })
            .stroke(Stroke::NONE)
            .corner_radius(6);
            if ui.add(b).on_hover_text(tip).clicked() {
                *live = v;
            }
        }
    });
}

/// Side-by-side choices, one of them selected.
fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (v, label) in options {
            let on = *value == *v;
            let b = egui::Button::new(RichText::new(*label).color(if on {
                Color32::BLACK
            } else {
                TEXT
            }))
            .fill(if on { AMBER } else { CARD })
            .stroke(Stroke::NONE)
            .corner_radius(6);
            if ui.add(b).clicked() {
                *value = *v;
            }
        }
    });
}

/// A small heading with some room above.
fn section(ui: &mut egui::Ui, text: &str) {
    ui.add_space(14.0);
    ui.label(
        RichText::new(text.to_uppercase())
            .small()
            .strong()
            .color(MUTED),
    );
    ui.add_space(2.0);
}

/// A slim notice line with a coloured edge.
fn banner(ui: &mut egui::Ui, color: Color32, add: impl FnOnce(&mut egui::Ui)) {
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.6)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(add);
        });
    ui.add_space(8.0);
}

fn clock() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

fn style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.override_text_color = Some(TEXT);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(36, 40, 49);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(48, 53, 64);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(58, 64, 77);
    v.selection.bg_fill = Color32::from_rgb(120, 86, 20);
    // Buttons keep their size on hover and press: nothing moves.
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.expansion = 0.0;
    }
    ctx.set_visuals(v);
    let mut s = (*ctx.global_style()).clone();
    s.spacing.button_padding = Vec2::new(12.0, 6.0);
    s.spacing.item_spacing = Vec2::new(8.0, 6.0);
    // Small print stays readable from a step away.
    if let Some(f) = s.text_styles.get_mut(&egui::TextStyle::Small) {
        f.size = 12.0;
    }
    ctx.set_global_style(s);
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Displays.
        if self.displays_at.elapsed() > Duration::from_secs(2) {
            self.displays_at = Instant::now();
            if let Some(w) = frame.winit_window() {
                let current = w.current_monitor();
                self.displays = w
                    .available_monitors()
                    .map(|m| {
                        let (size, pos, scale) =
                            (m.size(), m.position(), m.scale_factor().max(0.1) as f32);
                        Disp {
                            name: m.name().unwrap_or_else(|| "Bildschirm".into()),
                            pos: [pos.x as f32 / scale, pos.y as f32 / scale],
                            size: [size.width as f32 / scale, size.height as f32 / scale],
                            px: [size.width, size.height],
                            here: current.as_ref() == Some(&m),
                        }
                    })
                    .collect();
            }
        }
        if self.office_found.is_none() && self.office_checked.elapsed() > Duration::from_secs(3) {
            self.office_checked = Instant::now();
            self.office_found = office::find();
        }
        // Loading finished?
        if let Some(r) = self.loading.as_ref().and_then(|l| l.rx.try_recv().ok()) {
            self.finish_load(r);
        }
        // Presentation changed on disk?
        if self.deck.is_some() && self.last_mtime_check.elapsed() > Duration::from_secs(2) {
            self.last_mtime_check = Instant::now();
            let m = std::fs::metadata(&self.show.presentation)
                .and_then(|m| m.modified())
                .ok();
            if m.is_some() && m != self.mtime {
                self.changed_on_disk = true;
            }
        }
        // Dropped files.
        let dropped: Option<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .find(|p| !p.as_os_str().is_empty())
        });
        if let Some(p) = dropped {
            if self.live() {
                self.locked();
            } else {
                self.open(&p, &ctx);
            }
        }
        self.tc.apply(&self.show.timecode.clone());
        self.update_osc(&ctx);
        self.keys(&ctx);
        self.follow_timecode();
        self.receive_pictures(&ctx);
        self.want_pictures();
        self.update_ndi();
        self.autosave();
        self.output_window(&ctx);

        let osc_port = self.osc.as_ref().map(|l| l.port);
        let rtp = self
            .tc
            .rtp_port()
            .map(|p| (self.show.timecode.rtp_name.clone(), p));
        self.net.update(osc_port, rtp);

        egui::Panel::top("top")
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(14, 8))
                    .stroke(if self.live() {
                        Stroke::new(2.0, MODE_LIVE)
                    } else {
                        Stroke::NONE
                    }),
            )
            .exact_size(46.0)
            .show(ui, |ui| self.top_bar(ui));
        if self.deck.is_some() {
            egui::Panel::left("slides")
                .frame(
                    Frame::new()
                        .fill(PANEL)
                        .inner_margin(Margin::symmetric(8, 10)),
                )
                .default_size(300.0)
                .min_size(220.0)
                .resizable(true)
                .show(ui, |ui| self.slide_list(ui));
            if self.selected.is_some() && !self.live() {
                egui::Panel::right("inspector")
                    .frame(Frame::new().fill(PANEL).inner_margin(Margin::same(14)))
                    .default_size(290.0)
                    .min_size(250.0)
                    .resizable(true)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| self.inspector(ui));
                    });
            }
        }
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(Margin::same(18)))
            .show(ui, |ui| {
                self.banners(ui);
                if self.deck.is_some() {
                    self.center(ui);
                } else {
                    self.welcome(ui);
                }
            });
        self.toast(&ctx);
        if self.settings_open {
            self.settings_window(&ctx);
        }
        if self.player.animating() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(
                if self.show.timecode.source == TcSource::Off {
                    250
                } else {
                    33
                },
            ));
        }
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
