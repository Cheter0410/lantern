//! The Lantern window: slide list, live / next, GO, notes, slide settings,
//! and the outputs (full-screen display, NDI).

use crate::deck::{self, Deck, Image, Kind};
use crate::office::{self, InstallState};
use crate::osc::{self, Command};
use crate::pptx::{self, SlideInfo, Transition};
use crate::show::{self, Chase, Player, Resolution, Show, TcSource, TransitionCfg, Trigger};
use crate::{ndi, timecode};
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
const LIVE: Color32 = Color32::from_rgb(34, 197, 94);
const NEXT: Color32 = Color32::from_rgb(245, 179, 66);
const TC: Color32 = Color32::from_rgb(96, 165, 250);
const DANGER: Color32 = Color32::from_rgb(239, 68, 68);
const AMBER: Color32 = Color32::from_rgb(255, 196, 92);

/// App settings kept between starts.
#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct Settings {
    last: Option<PathBuf>,
}

fn settings_file() -> Option<PathBuf> {
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
        let (go, back, black, first, open, settings) = ctx.input(|i| {
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
            )
        });
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
        if open {
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
            crate::platform::raise_output_windows("Lantern Output");
        }
        if closed {
            self.show.output.display = None;
        }
    }

    // ---------------------------------------------------------------- UI parts

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("Lantern").size(20.0).strong().color(AMBER));
            ui.add_space(8.0);
            let name = self
                .show
                .presentation
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Keine Präsentation".into());
            ui.label(RichText::new(name).color(MUTED));
            if ui
                .button("Öffnen…")
                .on_hover_text("PowerPoint, Keynote, OpenDocument oder PDF (Strg/⌘+O)")
                .clicked()
            {
                self.pick_file(&ctx);
            }
            if self.deck.is_some()
                && ui
                    .button("Neu laden")
                    .on_hover_text("Die Präsentation neu einlesen")
                    .clicked()
            {
                self.load(&ctx);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Einstellungen").clicked() {
                    self.settings_open = !self.settings_open;
                }
                ui.add_space(6.0);
                // Outputs.
                let ndi_ok = self.ndi.is_some();
                if self.show.output.ndi {
                    chip(ui, "NDI", if ndi_ok { LIVE } else { DANGER });
                }
                match self.show.output.display {
                    Some(d) if d < self.displays.len() => {
                        chip(ui, &format!("Bildschirm {}", d + 1), LIVE)
                    }
                    Some(d) => chip(ui, &format!("Bildschirm {} fehlt", d + 1), DANGER),
                    None => chip(ui, "Kein Ausgang", MUTED),
                }
                ui.add_space(10.0);
                // Clock and timecode.
                ui.label(RichText::new(clock()).monospace().size(16.0).color(MUTED));
                ui.add_space(6.0);
                let (text, color) = match (self.show.timecode.source, self.tc.reading()) {
                    (TcSource::Off, _) => ("TC aus".to_string(), MUTED),
                    (_, None) => ("--:--:--:--".to_string(), MUTED),
                    (_, Some(r)) => (
                        show::format_tc(r.secs, r.fps),
                        if r.running { TC } else { MUTED },
                    ),
                };
                ui.label(
                    RichText::new(text)
                        .monospace()
                        .size(18.0)
                        .strong()
                        .color(color),
                )
                .on_hover_text(self.tc.status());
            });
        });
    }

    fn slide_list(&mut self, ui: &mut egui::Ui) {
        let next = self.next_index();
        let mut clicked: Option<usize> = None;
        let mut double: Option<usize> = None;
        let scroll = std::mem::take(&mut self.scroll_to_live);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, s) in self.slides.iter().enumerate() {
                    let live = self.player.current == Some(i);
                    let is_next = next == Some(i);
                    let sel = self.selected == Some(i);
                    let stroke = if live {
                        Stroke::new(2.0, LIVE)
                    } else if is_next {
                        Stroke::new(1.5, NEXT)
                    } else if sel {
                        Stroke::new(1.5, TC)
                    } else {
                        Stroke::new(1.0, LINE)
                    };
                    let resp = Frame::new()
                        .fill(if sel { CARD } else { PANEL })
                        .stroke(stroke)
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::same(6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("{}", i + 1))
                                        .monospace()
                                        .strong()
                                        .color(if live { LIVE } else { MUTED }),
                                );
                                let tw = 112.0;
                                let aspect =
                                    self.deck.as_ref().map(|d| d.aspect).unwrap_or(16.0 / 9.0);
                                let (r, _) = ui.allocate_exact_size(
                                    Vec2::new(tw, tw / aspect),
                                    Sense::hover(),
                                );
                                match self.thumbs.get(&i) {
                                    Some(t) => {
                                        ui.painter().image(
                                            t.id(),
                                            r,
                                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                            Color32::WHITE,
                                        );
                                    }
                                    None => {
                                        ui.painter().rect_filled(r, 2.0, Color32::BLACK);
                                    }
                                }
                                ui.vertical(|ui| {
                                    let title = if s.title.is_empty() {
                                        format!("Folie {}", s.number)
                                    } else {
                                        s.title.clone()
                                    };
                                    ui.add(
                                        egui::Label::new(RichText::new(title).strong()).truncate(),
                                    );
                                    ui.horizontal(|ui| {
                                        let cfg = self.show.slides.get(i);
                                        match cfg.map(|c| c.trigger) {
                                            Some(Trigger::Timecode) => {
                                                let tc = cfg.map(|c| c.tc.as_str()).unwrap_or("");
                                                chip(
                                                    ui,
                                                    &format!(
                                                        "TC {}",
                                                        if tc.is_empty() { "–" } else { tc }
                                                    ),
                                                    TC,
                                                );
                                            }
                                            _ => chip(ui, "GO", MUTED),
                                        }
                                        if live {
                                            chip(ui, "LIVE", LIVE);
                                        } else if is_next {
                                            chip(ui, "NEXT", NEXT);
                                        }
                                        if !s.notes.is_empty() {
                                            ui.label(RichText::new("Notiz").small().color(MUTED))
                                                .on_hover_text(&s.notes);
                                        }
                                    });
                                });
                            });
                        })
                        .response
                        .interact(Sense::click());
                    if scroll && live {
                        resp.scroll_to_me(Some(Align::Center));
                    }
                    if resp.double_clicked() {
                        double = Some(i);
                    } else if resp.clicked() {
                        clicked = Some(i);
                    }
                    ui.add_space(4.0);
                }
            });
        if let Some(i) = clicked {
            self.selected = Some(i);
        }
        if let Some(i) = double {
            self.goto(Some(i));
        }
    }

    fn center(&mut self, ui: &mut egui::Ui) {
        let next = self.next_index();
        let avail = ui.available_size();
        let aspect = {
            let (w, h) = self.output_size();
            w as f32 / h as f32
        };
        // Live big, next smaller beside it.
        let live_w = ((avail.x - 24.0) * 0.64).max(200.0);
        let next_w = ((avail.x - 24.0) * 0.36 - 12.0).max(120.0);
        let max_h = (avail.y * 0.55).max(120.0);
        let live_w = live_w.min(max_h * aspect);
        let next_w = next_w.min(max_h * aspect);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("LIVE").strong().color(LIVE));
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
                ui.painter()
                    .rect_stroke(r, 0.0, Stroke::new(2.0, LIVE), egui::StrokeKind::Outside);
                if self.player.blackout {
                    ui.painter().text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        "BLACKOUT",
                        egui::FontId::proportional(28.0),
                        DANGER,
                    );
                }
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("NEXT").strong().color(NEXT));
                let (r, _) =
                    ui.allocate_exact_size(Vec2::new(next_w, next_w / aspect), Sense::hover());
                paint_output(ui.painter(), r, None, self.tex(next), 1.0, 0.0);
                ui.painter()
                    .rect_stroke(r, 0.0, Stroke::new(1.5, NEXT), egui::StrokeKind::Outside);
                if next.is_none() {
                    ui.painter().text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        "Ende",
                        egui::FontId::proportional(20.0),
                        MUTED,
                    );
                }
                ui.add_space(8.0);
                if let Some(n) = next {
                    let trig = self
                        .show
                        .slides
                        .get(n)
                        .map(|s| s.trigger)
                        .unwrap_or_default();
                    let t = match trig {
                        Trigger::Go => "kommt mit GO".to_string(),
                        Trigger::Timecode => format!(
                            "kommt bei TC {}",
                            self.show.slides.get(n).map(|s| s.tc.as_str()).unwrap_or("")
                        ),
                    };
                    ui.label(RichText::new(format!("Folie {} · {t}", n + 1)).color(MUTED));
                }
                // Time on the current slide.
                if self.player.current.is_some() {
                    let s = self.player.since.elapsed().as_secs();
                    ui.label(
                        RichText::new(format!("Auf dieser Folie seit {}:{:02}", s / 60, s % 60))
                            .color(MUTED),
                    );
                }
            });
        });
        ui.add_space(12.0);
        // Buttons.
        ui.horizontal(|ui| {
            let go_label = match next {
                Some(n) => format!("GO  ·  Folie {}", n + 1),
                None => "GO  (Ende)".into(),
            };
            let go = ui.add_enabled(
                next.is_some(),
                egui::Button::new(
                    RichText::new(go_label)
                        .size(22.0)
                        .strong()
                        .color(Color32::BLACK),
                )
                .fill(LIVE)
                .min_size(Vec2::new(240.0, 56.0)),
            );
            if go
                .on_hover_text("Leertaste, →, Bild ↓ oder Klicker")
                .clicked()
            {
                self.go();
            }
            if ui
                .add(
                    egui::Button::new(RichText::new("Zurück").size(16.0))
                        .min_size(Vec2::new(110.0, 56.0)),
                )
                .on_hover_text("←, Bild ↑")
                .clicked()
            {
                self.back();
            }
            let bo = self.player.blackout;
            let b = egui::Button::new(RichText::new("Blackout").size(16.0).color(if bo {
                Color32::WHITE
            } else {
                TEXT
            }))
            .fill(if bo { DANGER } else { CARD })
            .min_size(Vec2::new(120.0, 56.0));
            if ui.add(b).on_hover_text("B oder . (Taste)").clicked() {
                self.player.set_blackout(!bo);
            }
            if ui
                .add(
                    egui::Button::new(RichText::new("Leeren").size(16.0))
                        .min_size(Vec2::new(100.0, 56.0)),
                )
                .on_hover_text("Zurück auf Schwarz vor der ersten Folie")
                .clicked()
            {
                self.goto(None);
            }
        });
        ui.add_space(12.0);
        // Speaker notes of the live slide.
        Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(ui.available_height().max(40.0) - 24.0);
                ui.label(RichText::new("Notizen").small().color(MUTED));
                let notes = self
                    .player
                    .current
                    .and_then(|c| self.slides.get(c))
                    .map(|s| s.notes.as_str())
                    .unwrap_or("");
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.label(RichText::new(if notes.is_empty() { "—" } else { notes }).size(19.0));
                });
            });
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.selected.filter(|i| *i < self.show.slides.len()) else {
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "Folie anklicken, um sie einzustellen.\nDoppelklick zeigt sie sofort.",
                )
                .color(MUTED),
            );
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
        ui.label(
            RichText::new(format!("Folie {}", i + 1))
                .size(17.0)
                .strong(),
        );
        if !title.is_empty() {
            ui.label(RichText::new(title).color(MUTED));
        }
        if let Some(t) = self.thumbs.get(&i) {
            let w = ui.available_width();
            let aspect = self.deck.as_ref().map(|d| d.aspect).unwrap_or(16.0 / 9.0);
            ui.add(egui::Image::new(t).fit_to_exact_size(Vec2::new(w, w / aspect)));
        }
        if ui.button("Jetzt zeigen").clicked() {
            self.goto(Some(i));
        }
        ui.separator();
        let cfg = &mut self.show.slides[i];
        ui.label(RichText::new("Auslösen").strong());
        ui.radio_value(
            &mut cfg.trigger,
            Trigger::Go,
            "Mit GO (Knopf, Klicker, OSC)",
        );
        ui.radio_value(&mut cfg.trigger, Trigger::Timecode, "Bei Timecode");
        if cfg.trigger == Trigger::Timecode {
            ui.horizontal(|ui| {
                let fps = reading.map(|r| r.fps.round()).unwrap_or(30.0);
                let ok = cfg.tc.is_empty() || show::parse_tc(&cfg.tc, fps).is_some();
                ui.add(
                    egui::TextEdit::singleline(&mut cfg.tc)
                        .hint_text("HH:MM:SS:FF")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(120.0)
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
                    RichText::new("Timecode-Eingang ist aus (Einstellungen).")
                        .small()
                        .color(AMBER),
                );
            }
        }
        ui.separator();
        ui.label(RichText::new("Übergang").strong());
        let file_label = match file_t {
            Transition::Cut => "Wie in der Datei (Schnitt)".to_string(),
            Transition::Fade(s) => format!("Wie in der Datei (Blende {s:.1} s)"),
        };
        let mut kind = match cfg.transition {
            TransitionCfg::FromFile => 0,
            TransitionCfg::Cut => 1,
            TransitionCfg::Fade(_) => 2,
        };
        ui.radio_value(&mut kind, 0, file_label);
        ui.radio_value(&mut kind, 1, "Schnitt");
        ui.radio_value(&mut kind, 2, "Blende");
        let mut secs = match cfg.transition {
            TransitionCfg::Fade(s) => s,
            _ => 0.7,
        };
        if kind == 2 {
            ui.add(
                egui::Slider::new(&mut secs, 0.1..=5.0)
                    .suffix(" s")
                    .step_by(0.1),
            );
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
            ui.separator();
            ui.label(RichText::new("Notizen").strong());
            ui.label(RichText::new(notes).color(MUTED));
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("Einstellungen")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(480.0)
            .default_height(640.0)
            .default_pos(ctx.content_rect().center() - Vec2::new(240.0, 320.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.settings_ui(ui));
            });
        self.settings_open = open;
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Ausgang").size(16.0).strong());
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
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Bildschirm");
                egui::ComboBox::from_id_salt("display")
                    .width(300.0)
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
                ui.label("Auflösung");
                egui::ComboBox::from_id_salt("res")
                    .selected_text(o.resolution.label())
                    .show_ui(ui, |ui| {
                        for r in Resolution::ALL {
                            ui.selectable_value(&mut o.resolution, r, r.label());
                        }
                    });
                ui.end_row();
                ui.label("NDI");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut o.ndi, "senden als");
                    ui.add_enabled(
                        o.ndi,
                        egui::TextEdit::singleline(&mut o.ndi_name).desired_width(140.0),
                    );
                });
                ui.end_row();
                ui.label("Bilder/s (NDI)");
                ui.horizontal(|ui| {
                    for f in [25, 30, 50, 60] {
                        ui.selectable_value(&mut o.fps, f, f.to_string());
                    }
                });
                ui.end_row();
            });
        if let Some(e) = &self.ndi_error {
            ui.horizontal(|ui| {
                ui.label(RichText::new(e).color(DANGER));
                ui.hyperlink_to("ndi.video", ndi::WEBSITE);
            });
        } else if self.ndi.is_some() {
            let (w, h) = self.output_size();
            ui.label(
                RichText::new(format!(
                    "NDI-Quelle „{}“ sendet {w}×{h}",
                    self.show.output.ndi_name
                ))
                .color(LIVE),
            );
        }
        ui.label(
            RichText::new("Der Ausgang zeigt nur die Folien – keine Symbole, kein Mauszeiger. Esc beendet ihn nicht; dafür hier „Aus“ wählen.")
                .small()
                .color(MUTED),
        );

        ui.separator();
        ui.label(RichText::new("Timecode").size(16.0).strong());
        let t = &mut self.show.timecode;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut t.source, TcSource::Off, "Aus");
            ui.selectable_value(&mut t.source, TcSource::Ltc, "LTC (Audio)");
            ui.selectable_value(&mut t.source, TcSource::RtpMidi, "MIDI-Timecode (RTP-MIDI)");
        });
        match t.source {
            TcSource::Off => {}
            TcSource::Ltc => {
                egui::Grid::new("ltc")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Eingang");
                        let shown = if t.ltc_device.is_empty() {
                            "Standard".to_string()
                        } else {
                            t.ltc_device.clone()
                        };
                        egui::ComboBox::from_id_salt("ltcdev")
                            .width(280.0)
                            .selected_text(shown)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut t.ltc_device, String::new(), "Standard");
                                for d in timecode::audio_inputs() {
                                    ui.selectable_value(&mut t.ltc_device, d.clone(), d);
                                }
                            });
                        ui.end_row();
                        ui.label("Kanal");
                        let mut ch = t.ltc_channel + 1;
                        ui.add(egui::DragValue::new(&mut ch).range(1..=64));
                        t.ltc_channel = ch - 1;
                        ui.end_row();
                        ui.label("Pegel");
                        let l = self.tc.level();
                        ui.add(egui::ProgressBar::new(l.min(1.0)).desired_width(200.0));
                        ui.end_row();
                    });
            }
            TcSource::RtpMidi => {
                egui::Grid::new("rtp")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Session-Name");
                        ui.text_edit_singleline(&mut t.rtp_name);
                        ui.end_row();
                        ui.label("Port");
                        ui.add(egui::DragValue::new(&mut t.rtp_port).range(1024..=65000));
                        ui.end_row();
                    });
                let peers = self.tc.connected();
                if peers.is_empty() {
                    ui.label(RichText::new("Noch niemand verbunden. In Show Master unter MIDI die Session „Lantern“ verbinden – oder hier:").color(MUTED));
                } else {
                    ui.label(RichText::new(format!("Verbunden: {}", peers.join(", "))).color(LIVE));
                }
                let mut connect: Option<String> = None;
                for (name, addr) in self.tc.discovered() {
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
        ui.label(RichText::new(self.tc.status()).small().color(MUTED));

        ui.separator();
        ui.label(RichText::new("Fernsteuerung (OSC)").size(16.0).strong());
        ui.horizontal(|ui| {
            ui.label("UDP-Port");
            ui.add(egui::DragValue::new(&mut self.show.osc_port).range(1024..=65000));
            match (&self.osc, &self.osc_error) {
                (Some(_), _) => {
                    ui.label(RichText::new("empfängt").color(LIVE));
                }
                (None, Some((_, e))) => {
                    ui.label(RichText::new(e).color(DANGER));
                }
                _ => {}
            }
        });
        ui.label(
            RichText::new("/lantern/go · /lantern/back · /lantern/slide <n> · /lantern/first · /lantern/blackout [0|1] · /lantern/clear")
                .small()
                .monospace()
                .color(MUTED),
        );

        ui.separator();
        ui.label(RichText::new("LibreOffice").size(16.0).strong());
        match &self.office_found {
            Some(p) => {
                ui.label(
                    RichText::new(format!("Gefunden: {}", p.display()))
                        .small()
                        .color(MUTED),
                );
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
    }

    /// Banner offering to install LibreOffice.
    fn office_banner(&mut self, ui: &mut egui::Ui) {
        if self.office_found.is_some() {
            return;
        }
        let st = self.install.lock().map(|s| s.clone()).unwrap_or_default();
        Frame::new()
            .fill(Color32::from_rgb(48, 38, 14))
            .stroke(Stroke::new(1.0, AMBER))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("LibreOffice fehlt").strong().color(AMBER));
                    ui.label("Lantern braucht es, um PowerPoint-Dateien originalgetreu zu öffnen.");
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
                            if ui.link("Download-Seite").clicked() {
                                office::open_url(office::DOWNLOAD_PAGE);
                            }
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
                            if let Some(e) = &st.error {
                                ui.label(RichText::new(e).small().color(DANGER));
                            } else if !st.text.is_empty() {
                                ui.label(RichText::new(&st.text).small());
                            }
                        }
                    });
                });
            });
        ui.add_space(6.0);
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.25);
            ui.label(RichText::new("Lantern").size(40.0).strong().color(AMBER));
            ui.label(
                RichText::new("Präsentationen für die Bühne")
                    .size(16.0)
                    .color(MUTED),
            );
            ui.add_space(24.0);
            if let Some(l) = &self.loading {
                ui.spinner();
                let s = l.status.lock().map(|s| s.clone()).unwrap_or_default();
                ui.label(format!("{s}  ({} s)", l.started.elapsed().as_secs()));
                return;
            }
            if let Some(e) = &self.load_error {
                ui.label(RichText::new(e).color(DANGER));
                ui.add_space(8.0);
            }
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("Präsentation öffnen…")
                            .size(18.0)
                            .strong()
                            .color(Color32::BLACK),
                    )
                    .fill(AMBER)
                    .min_size(Vec2::new(260.0, 48.0)),
                )
                .clicked()
            {
                self.pick_file(&ctx);
            }
            ui.add_space(8.0);
            ui.label(RichText::new("oder eine .pptx-Datei hierher ziehen").color(MUTED));
        });
    }
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

fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    Frame::new()
        .stroke(Stroke::new(1.0, color))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().strong().color(color));
        });
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
    ctx.set_visuals(v);
    let mut s = (*ctx.global_style()).clone();
    s.spacing.button_padding = Vec2::new(12.0, 6.0);
    s.spacing.item_spacing = Vec2::new(8.0, 6.0);
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
            self.open(&p, &ctx);
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

        egui::Panel::top("top")
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(10, 8)),
            )
            .show(ui, |ui| self.top_bar(ui));
        if self.deck.is_some() {
            egui::Panel::left("slides")
                .frame(Frame::new().fill(PANEL).inner_margin(Margin::same(8)))
                .default_size(330.0)
                .min_size(240.0)
                .resizable(true)
                .show(ui, |ui| self.slide_list(ui));
            egui::Panel::right("inspector")
                .frame(Frame::new().fill(PANEL).inner_margin(Margin::same(12)))
                .default_size(280.0)
                .min_size(230.0)
                .resizable(true)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.inspector(ui));
                });
        }
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(Margin::same(14)))
            .show(ui, |ui| {
                self.office_banner(ui);
                if self.changed_on_disk {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Die Präsentation wurde geändert.").color(AMBER));
                        if ui.button("Neu laden").clicked() {
                            self.load(&ctx);
                        }
                    });
                }
                if let Some(l) = &self.loading {
                    if self.deck.is_some() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            let s = l.status.lock().map(|s| s.clone()).unwrap_or_default();
                            ui.label(s);
                        });
                    }
                }
                if let (Some(e), true) = (&self.load_error, self.deck.is_some()) {
                    ui.label(RichText::new(e).color(DANGER));
                }
                if let Some((m, at)) = &self.message {
                    if at.elapsed() < Duration::from_secs(4) {
                        ui.label(RichText::new(m).color(MUTED));
                    }
                }
                if self.deck.is_some() {
                    self.center(ui);
                } else {
                    self.welcome(ui);
                }
            });
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
