//! The window driven like a user would: Edit and Live mode.
//!
//! Set `LANTERN_SHOTS=<folder>` to also write a picture of every step there.

#![allow(clippy::expect_used)]

use super::App;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A small PDF with `n` pages, so no LibreOffice is needed.
fn pdf(n: usize) -> Vec<u8> {
    let mut objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        String::new(), // pages, filled in below
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    let mut kids = Vec::new();
    for i in 0..n {
        let text = format!("BT /F1 96 Tf 200 250 Td (Folie {}) Tj ET", i + 1);
        objs.push(format!(
            "<< /Length {} >>\nstream\n{text}\nendstream",
            text.len()
        ));
        let content = objs.len();
        objs.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 960 540] \
             /Resources << /Font << /F1 3 0 R >> >> /Contents {content} 0 R >>"
        ));
        kids.push(format!("{} 0 R", objs.len()));
    }
    objs[1] = format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" "));
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .bytes(),
    );
    out
}

struct Shots(Option<PathBuf>);

impl Shots {
    fn take(&self, h: &mut Harness<'_, App>, name: &str) {
        let Some(dir) = &self.0 else { return };
        match h.render() {
            Ok(img) => {
                let _ = img.save(dir.join(format!("{name}.png")));
            }
            Err(e) => eprintln!("no picture of {name}: {e}"),
        }
    }
}

/// A left click at a point of the window (the slide list paints its rows itself).
fn click_at(h: &Harness<'_, App>, pos: egui::Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
}

impl Shots {
    /// With pictures on: the box of pixels around `r` that differ from the
    /// background just outside it (the button as painted).
    fn painted(&self, h: &mut Harness<'_, App>, r: egui::Rect) -> Option<[u32; 4]> {
        self.0.as_ref()?;
        let img = h.render().ok()?;
        let ppp = img.width() as f32 / 1400.0;
        let px = |x: f32| (x * ppp).round() as u32;
        let (x0, y0, x1, y1) = (
            px(r.left() - 4.0),
            px(r.top() - 4.0),
            px(r.right() + 4.0),
            px(r.bottom() + 4.0),
        );
        let bg = *img.get_pixel(x0, y0);
        let mut b = [u32::MAX, u32::MAX, 0, 0];
        for y in y0..y1 {
            for x in x0..x1 {
                if *img.get_pixel(x, y) != bg {
                    b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
                }
            }
        }
        Some(b)
    }
}

fn run_until(h: &mut Harness<'_, App>, what: &str, done: impl Fn(&App) -> bool) {
    let start = Instant::now();
    while !done(h.state()) {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "waited for {what}"
        );
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    h.run_steps(3);
}

fn harness(dir: &Path) -> Harness<'static, App> {
    let file = dir.join("show.pdf");
    std::fs::write(&file, pdf(3)).expect("write pdf");
    harness_for(file)
}

fn harness_for(file: PathBuf) -> Harness<'static, App> {
    let mut b = Harness::builder().with_size([1400.0, 860.0]);
    if std::env::var_os("LANTERN_SHOTS").is_some() {
        b = b.wgpu();
    }
    b.build_eframe(move |cc| App::new(&cc.egui_ctx, Some(file)))
}

#[test]
fn live_mode_locks_the_show() {
    let dir = std::env::temp_dir().join(format!("lantern-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    // Keep the app's own settings (last file, mode) away from the real ones.
    super::set_test_config(dir.join("config"));
    let shots = Shots(std::env::var_os("LANTERN_SHOTS").map(PathBuf::from));
    if let Some(d) = &shots.0 {
        let _ = std::fs::create_dir_all(d);
    }

    let mut h = harness(&dir);
    run_until(&mut h, "the slides", |a| {
        a.slides.len() == 3 && a.thumbs.len() == 3
    });
    assert!(!h.state().live(), "starts in Edit");
    assert!(h.query_by_label("Open").is_some());

    // Edit: clicking a slide opens its settings.
    click_at(&h, egui::pos2(150.0, 80.0));
    h.run_steps(3);
    assert_eq!(h.state().selected, Some(0), "slide settings open in Edit");
    assert!(h.query_by_label("Show now").is_some());
    shots.take(&mut h, "1-edit-slide-settings");

    // Switch to Live with the button: settings close, Open is gone.
    h.get_by_label("Live").click();
    h.run_steps(3);
    assert!(h.state().live());
    assert_eq!(h.state().selected, None, "Live closes the slide settings");
    assert!(h.query_by_label("Show now").is_none());
    assert!(h.query_by_label("Open").is_none());
    shots.take(&mut h, "2-live");

    // Live: clicking a slide does not open its settings, the show still runs.
    click_at(&h, egui::pos2(150.0, 80.0));
    h.run_steps(3);
    assert_eq!(h.state().selected, None, "no slide settings in Live");
    h.key_press(Key::Space);
    h.run_steps(3);
    assert_eq!(h.state().player.current, Some(0), "GO works in Live");
    run_until(&mut h, "the live and next pictures", |a| {
        a.fulls.contains_key(&0) && a.fulls.contains_key(&1)
    });
    // Hovering a button must not change its size (nothing moves).
    let button = h.get_by_label("Settings").rect();
    let before = shots.painted(&mut h, button);
    // Pointer in the top left corner, so the painted cursor stays inside.
    h.event(egui::Event::PointerMoved(
        button.left_top() + egui::vec2(2.0, 2.0),
    ));
    h.run_steps(3);
    let after = shots.painted(&mut h, button);
    assert_eq!(before, after, "the painted button keeps its size on hover");
    let v = h.ctx.global_style().visuals.clone();
    for w in [&v.widgets.hovered, &v.widgets.active, &v.widgets.inactive] {
        assert_eq!(w.expansion, 0.0, "buttons keep their size");
        assert_eq!(
            w.bg_stroke.width, 0.0,
            "no hover outline that shrinks the inside"
        );
    }
    shots.take(&mut h, "2b-live-running-hover");

    // GO- and GO+ side by side; Blackout and Reset set apart on the right.
    let back = h.get_by_label("GO\u{2212}").rect();
    let go = h.get_by_label_contains("GO+").rect();
    let blackout = h.get_by_label("Blackout").rect();
    let reset = h.get_by_label("Reset").rect();
    assert!(back.right() < go.left(), "GO- left of GO+");
    assert!(
        go.right() + 100.0 < blackout.left(),
        "Blackout far from GO+"
    );
    assert!(
        blackout.right() + 24.0 < reset.left(),
        "a clear gap between Blackout and Reset"
    );
    h.get_by_label("GO\u{2212}").click();
    h.run_steps(3);
    assert_eq!(
        h.state().player.current,
        Some(0),
        "GO- at the first slide stays"
    );
    h.get_by_label_contains("GO+").click();
    h.run_steps(3);
    assert_eq!(h.state().player.current, Some(1), "GO+ goes on");
    h.get_by_label("GO\u{2212}").click();
    h.run_steps(3);
    assert_eq!(h.state().player.current, Some(0), "GO- goes back");

    // Live: the settings can be looked at, not changed.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    h.run_steps(3);
    assert!(h.state().settings_open);
    assert!(h.query_by_label_contains("view only").is_some());
    shots.take(&mut h, "3-live-settings");
    h.key_press(Key::Escape);
    h.run_steps(3);

    // The mode is kept across a restart.
    drop(h);
    let mut h = harness(&dir);
    run_until(&mut h, "the slides again", |a| a.slides.len() == 3);
    assert!(h.state().live(), "Live is kept across restarts");

    // Cmd/Ctrl+E goes back to Edit.
    h.key_press_modifiers(Modifiers::COMMAND, Key::E);
    h.run_steps(3);
    assert!(!h.state().live());
    assert!(h.query_by_label("Open").is_some());
    shots.take(&mut h, "4-edit-again");

    let _ = std::fs::remove_dir_all(&dir);
}

fn clip_now(a: &App) -> Option<&crate::media::Playing> {
    a.video.as_ref().and_then(|v| v.playing.as_ref())
}

#[test]
fn plays_the_video_of_a_pptx() {
    if crate::office::find().is_none() || crate::media::ffmpeg().is_none() {
        eprintln!("skipped: needs LibreOffice and FFmpeg");
        return;
    }
    let dir = std::env::temp_dir().join(format!("lantern-video-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    super::set_test_config(dir.join("config"));
    let shots = Shots(std::env::var_os("LANTERN_SHOTS").map(PathBuf::from));
    if let Some(d) = &shots.0 {
        let _ = std::fs::create_dir_all(d);
    }
    let file = dir.join("video.pptx");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/video.pptx"),
        &file,
    )
    .expect("copy pptx");

    let mut h = harness_for(file);
    run_until(&mut h, "the slides and the clip", |a| {
        a.slides.len() >= 2 && a.loading.is_none() && a.clips.contains_key(&1)
    });
    assert!(
        h.state().clips[&1].is_ok(),
        "clip ready: {:?}",
        h.state().clips[&1].as_ref().err()
    );
    // Quiet tests: no sound on the real interface.
    h.state_mut().show.audio.enabled = false;

    // Edit: the video slide has its own settings.
    click_at(&h, egui::pos2(150.0, 160.0));
    h.run_steps(3);
    assert_eq!(h.state().selected, Some(1));
    assert!(h.query_by_label_contains("At the end").is_some());
    shots.take(&mut h, "5-video-slide-settings");
    // The file says "start after the slide"; here it waits for GO.
    h.state_mut().selected = None;
    let a = h.state_mut();
    if a.show.slides.len() < 2 {
        a.show.slides.resize_with(2, Default::default);
    }
    a.show.slides[1].video.start = crate::show::VideoStart::Go;
    h.state_mut().set_live(true);
    h.run_steps(2);

    h.key_press(Key::Space);
    h.run_steps(3);
    let go_without = h.get_by_label_contains("GO+").rect();
    let back_without = h.get_by_label("GO\u{2212}").rect();
    h.key_press(Key::Space);
    run_until(&mut h, "slide 2 live", |a| a.player.current == Some(1));
    h.run_steps(3);
    assert_eq!(
        h.get_by_label_contains("GO+").rect(),
        go_without,
        "GO+ stays where it is when a slide has a video"
    );
    assert_eq!(h.get_by_label("GO\u{2212}").rect(), back_without);
    assert!(h.state().go_starts_video(), "the video waits for GO");
    assert!(h.query_by_label_contains("Play video").is_some());
    let (len, playing) = h.state().video_left().expect("video length");
    assert!(!playing && len > 1.0, "length before start: {len}");
    shots.take(&mut h, "6-video-armed");

    h.key_press(Key::Space);
    run_until(&mut h, "video frames", |a| {
        a.video.as_ref().is_some_and(|v| v.overlay.is_some())
    });
    assert_eq!(h.state().player.current, Some(1), "GO played, did not skip");
    std::thread::sleep(Duration::from_millis(600));
    h.run_steps(3);
    let (left, playing) = h.state().video_left().expect("time left");
    assert!(playing && left < len, "counts down: {left} of {len}");
    shots.take(&mut h, "7-video-playing");

    // Transport: pause holds the time, the timeline jumps, K plays on.
    h.get_by_label_contains("Pause").click();
    h.run_steps(3);
    assert!(clip_now(h.state()).expect("playing").is_paused());
    let held = clip_now(h.state()).expect("playing").position();
    std::thread::sleep(Duration::from_millis(400));
    h.run_steps(3);
    let now = clip_now(h.state()).expect("playing").position();
    assert!((now - held).abs() < 0.05, "paused: {held} -> {now}");
    assert!(h.query_by_label_contains("Resume").is_some());
    // Click at three quarters of the timeline (a 4 s clip: about 3 s).
    let line = h.get_by_label("Timeline").rect();
    click_at(
        &h,
        egui::pos2(line.left() + line.width() * 0.75, line.center().y),
    );
    run_until(&mut h, "the jump", |a| {
        a.video
            .as_ref()
            .and_then(|v| v.playing.as_ref())
            .is_some_and(|p| (p.position() - 3.0).abs() < 0.3)
    });
    assert!(
        clip_now(h.state()).expect("playing").is_paused(),
        "stays paused"
    );
    shots.take(&mut h, "7b-video-paused");
    h.key_press(Key::K);
    h.run_steps(3);
    assert!(
        !clip_now(h.state()).expect("playing").is_paused(),
        "K plays on"
    );
    h.get_by_label_contains("\u{2212}10").click();
    h.run_steps(3);
    assert!(
        clip_now(h.state()).expect("playing").position() < 0.5,
        "back to the start"
    );

    // Settings › Audio.
    h.state_mut().set_live(false);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    h.run_steps(3);
    h.get_by_label("Audio").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("Sample rate").is_some());
    shots.take(&mut h, "8-settings-audio");

    // Settings › Timecode: MTC from a MIDI input of this computer.
    h.get_by_label("Timecode").click();
    h.run_steps(3);
    h.get_by_label("MTC (MIDI input)").click();
    h.run_steps(3);
    assert_eq!(h.state().show.timecode.source, crate::show::TcSource::Midi);
    assert!(h.query_all_by_label_contains("IAC").next().is_some());
    shots.take(&mut h, "9-settings-mtc");

    // Settings › Output: the test picture, on the display and the preview.
    h.get_by_label("Output").click();
    h.run_steps(3);
    // Frame rate: by default the highest the display can (no display: 60).
    assert!(h.state().show.output.fps_auto);
    assert_eq!(h.state().output_fps(), 60);
    shots.take(&mut h, "9b-settings-output");
    h.state_mut().show.output.fps_auto = false;
    h.state_mut().show.output.fps = 25;
    assert_eq!(h.state().output_fps(), 25);
    h.state_mut().show.output.fps_auto = true;
    h.get_by_label("Test picture on the outputs").click();
    run_until(&mut h, "the test picture", |a| a.test.tex.is_some());
    h.key_press(Key::Escape);
    h.state_mut().set_live(true);
    std::thread::sleep(Duration::from_millis(100));
    h.run_steps(3);
    assert!(h.query_by_label_contains("TEST PICTURE ON AIR").is_some());
    let n0 = h.state().test.frame();
    std::thread::sleep(Duration::from_millis(200));
    h.run_steps(3);
    assert!(h.state().test.frame() > n0, "the test picture moves on");
    assert_eq!(
        h.get_by_label_contains("GO+").rect(),
        go_without,
        "GO+ stays put with the test picture on"
    );
    shots.take(&mut h, "10-test-picture-live");
    // Off from the banner, also in Live.
    h.get_by_label("Turn off").click();
    h.run_steps(3);
    assert!(!h.state().test.on && h.state().test.tex.is_none());

    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}

fn start_harness() -> Harness<'static, App> {
    let mut b = Harness::builder().with_size([1400.0, 860.0]);
    if std::env::var_os("LANTERN_SHOTS").is_some() {
        b = b.wgpu();
    }
    let mut h = b.build_eframe(|cc| App::new(&cc.egui_ctx, None));
    // Other tests share the settings folder: start clean.
    h.state_mut().settings = super::Settings::default();
    h.run_steps(2);
    h
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lantern-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    super::set_test_config(dir.join("config"));
    dir
}

#[test]
fn new_show_runs_the_setup_wizard() {
    use super::start::{Screen, Step};
    let dir = temp_dir("startscreen");
    let shots = Shots(std::env::var_os("LANTERN_SHOTS").map(PathBuf::from));
    if let Some(d) = &shots.0 {
        let _ = std::fs::create_dir_all(d);
    }
    let talk = dir.join("Talk.pdf");
    std::fs::write(&talk, pdf(3)).expect("write pdf");

    let mut h = start_harness();
    assert_eq!(h.state().screen, Screen::Start, "no file: the start screen");
    for l in ["New show", "Open show", "Migrate older show"] {
        assert!(h.query_by_label(l).is_some(), "{l}");
    }
    shots.take(&mut h, "s1-start");

    h.get_by_label("New show").click();
    h.run_steps(3);
    assert!(h.query_by_label("Create and set up").is_some());
    {
        let p = &mut h.state_mut().start.plan;
        // The file dialogs can't run in a test: fill in what they would.
        p.name = "Gala 2026".into();
        p.presentation = talk.clone();
        p.location = dir.join("shows");
    }
    h.run_steps(3);
    shots.take(&mut h, "s2-new-show");
    h.get_by_label("Create and set up").click();
    h.run_steps(3);
    let folder = dir.join("shows/Gala 2026");
    assert_eq!(h.state().screen, Screen::Wizard(Step::Output));
    assert!(folder.join("Talk.pdf").is_file(), "presentation copied in");
    assert!(talk.is_file(), "copy keeps the original");
    assert!(folder.join("Gala 2026.lantern").is_file());
    shots.take(&mut h, "s3-wizard-output");

    // Output → Audio → Timecode → Ready, and Back.
    h.get_by_label("Next").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Wizard(Step::Audio));
    assert!(h.query_by_label("Test tone").is_some());
    shots.take(&mut h, "s4-wizard-audio");
    h.get_by_label("Back").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Wizard(Step::Output));
    h.get_by_label("Next").click();
    h.run_steps(3);
    h.get_by_label("Next").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Wizard(Step::Timecode));
    h.get_by_label("LTC (Audio)").click();
    h.run_steps(3);
    shots.take(&mut h, "s5-wizard-timecode");
    h.get_by_label("Next").click();
    run_until(&mut h, "the slides", |a| a.slides.len() == 3);
    assert_eq!(h.state().screen, Screen::Wizard(Step::Ready));
    assert!(h.query_by_label("3 slides ready").is_some());
    shots.take(&mut h, "s6-wizard-ready");
    h.get_by_label("Open show").click();
    h.run_steps(3);
    assert_eq!(
        h.state().screen,
        Screen::Main,
        "the main window after the wizard"
    );
    assert_eq!(h.state().show.timecode.source, super::TcSource::Ltc);
    shots.take(&mut h, "s7-main");
    // The wizard's choices are in the show file.
    let saved = crate::show::Show::load(&folder.join("Gala 2026.lantern")).expect("show");
    assert_eq!(saved.timecode.source, super::TcSource::Ltc);

    // Back to the start screen: the show is under Recent.
    h.get_by_label("Shows").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Start);
    assert_eq!(
        h.state().settings.recent.first(),
        Some(&folder.join("Gala 2026.lantern"))
    );
    shots.take(&mut h, "s8-start-recent");
    h.get_by_label("Gala 2026").click();
    h.run_steps(3);
    assert_eq!(h.state().screen, Screen::Main);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn migrates_an_older_show() {
    use super::start::Screen;
    let dir = temp_dir("migrateui");
    let shots = Shots(std::env::var_os("LANTERN_SHOTS").map(PathBuf::from));
    let pres = dir.join("Old Show.pdf");
    std::fs::write(&pres, pdf(2)).expect("write pdf");
    let mut old = crate::show::Show {
        presentation: pres.clone(),
        ..Default::default()
    };
    old.timecode.source = super::TcSource::Midi;
    old.save(&super::show_file_for(&pres)).expect("old show");

    let mut h = start_harness();
    h.get_by_label("Migrate older show").click();
    h.run_steps(3);
    h.state_mut().choose_old(&pres);
    h.state_mut().start.plan.location = dir.join("shows");
    h.run_steps(3);
    shots.take(&mut h, "s9-migrate");
    h.get_by_label("Migrate and open").click();
    h.run_steps(3);
    assert_eq!(
        h.state().screen,
        Screen::Main,
        "migrated shows open directly"
    );
    assert_eq!(h.state().show.timecode.source, super::TcSource::Midi);
    assert_eq!(
        h.state().show_file.as_deref(),
        Some(dir.join("shows/Old Show/Old Show.lantern").as_path())
    );
    assert!(pres.is_file(), "copied: the old show is still there");
    run_until(&mut h, "the slides", |a| a.slides.len() == 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn transport_layout_never_overlaps() {
    use super::TransportLayout as L;
    let mut w = 240.0;
    while w < 1800.0 {
        let l = L::for_width(w);
        assert!(l.go >= 100.0, "GO+ usable at {w}");
        if l.two_rows {
            assert!(
                l.left_width() <= w + 0.5 && l.right_width() <= w,
                "fits at {w}"
            );
        } else {
            assert!(
                l.left_width() + L::MIN_GAP + l.right_width() <= w + 0.5,
                "room between GO+ and Blackout at {w}: {l:?}"
            );
        }
        w += 7.0;
    }
    assert!(!L::for_width(1000.0).two_rows);
    assert!(L::for_width(380.0).two_rows);
}

/// Small windows (with the slide list and the slide settings open): GO−,
/// GO+, Blackout and Reset never run into each other.
#[test]
fn transport_fits_small_windows() {
    let dir = temp_dir("smallwin");
    let shots = Shots(std::env::var_os("LANTERN_SHOTS").map(PathBuf::from));
    let mut h = harness(&dir);
    h.state_mut().settings.live = false;
    run_until(&mut h, "the slides", |a| a.slides.len() == 3);
    for (w, ht, settings) in [
        (1400.0, 860.0, false),
        (1100.0, 700.0, false),
        (1100.0, 700.0, true),
        (900.0, 600.0, false),
        (900.0, 600.0, true),
    ] {
        h.state_mut().selected = settings.then_some(0);
        h.set_size(egui::vec2(w, ht));
        h.run_steps(4);
        let name = format!("t-{w}x{ht}{}", if settings { "-settings" } else { "" });
        shots.take(&mut h, &name);
        let r = [
            h.get_by_label("GO\u{2212}").rect(),
            h.get_by_label_contains("GO+").rect(),
            h.get_by_label("Blackout").rect(),
            h.get_by_label("Reset").rect(),
        ];
        for (i, a) in r.iter().enumerate() {
            assert!(a.width() >= 60.0, "{name}: button {i} too small: {a:?}");
            for b in &r[i + 1..] {
                assert!(!a.intersects(*b), "{name}: {a:?} and {b:?} overlap");
            }
        }
        // On one row: a real gap between GO+ and Blackout.
        if (r[1].center().y - r[2].center().y).abs() < 1.0 {
            assert!(
                r[2].left() - r[1].right() >= 30.0,
                "{name}: GO+ next to Blackout"
            );
        }
        assert!(r[3].right() <= w, "{name}: Reset inside the window");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Problems with the outputs show in the top bar, with a list and the way
/// to the settings that fix them; they go when the problem goes.
#[test]
fn output_problems_show_in_the_top_bar() {
    use crate::show::{TcSource, Trigger};
    let dir = temp_dir("health");
    let shots = Shots(std::env::var_os("LANTERN_SHOTS").map(PathBuf::from));
    let mut h = harness(&dir);
    h.state_mut().settings.live = false;
    run_until(&mut h, "the slides", |a| a.slides.len() == 3);
    let wait = |h: &mut Harness<'_, App>| {
        // The look comes every half second.
        std::thread::sleep(Duration::from_millis(550));
        h.run_steps(3);
    };

    // A display that is not there (the test has none).
    h.state_mut().show.output.display = Some(1);
    wait(&mut h);
    assert!(
        h.query_by_label("Health: Display 2 is not connected")
            .is_some(),
        "the badge names the problem"
    );
    shots.take(&mut h, "h1-display-missing");

    // And slides on timecode that never comes: two problems.
    {
        let a = h.state_mut();
        a.show.timecode.source = TcSource::RtpMidi;
        a.show.timecode.rtp_port = 0;
        if let Some(s) = a.show.slides.get_mut(1) {
            s.trigger = Trigger::Timecode;
        }
    }
    wait(&mut h);
    assert!(h.query_by_label("Health: 2 problems").is_some());
    h.get_by_label("Health: 2 problems").click();
    h.run_steps(3);
    assert!(h.state().health.open);
    assert!(h.query_by_label("No timecode yet").is_some());
    shots.take(&mut h, "h2-list");
    h.get_by_label("Timecode settings").click();
    h.run_steps(3);
    assert!(h.state().settings_open && !h.state().health.open);
    assert!(h.state().settings_tab == super::SettingsTab::Timecode);
    h.key_press(Key::Escape);
    h.run_steps(2);

    // Fixed: the badge goes.
    {
        let a = h.state_mut();
        a.settings_open = false;
        a.show.timecode.source = TcSource::Off;
        a.show.output.display = None;
        a.show.output.ndi = true;
        a.show.output.ndi_name = "Health test".into();
    }
    wait(&mut h);
    let titles: Vec<String> = h
        .state()
        .health
        .monitor
        .issues
        .iter()
        .map(|i| i.title.clone())
        .collect();
    assert!(
        !titles
            .iter()
            .any(|t| t.contains("Display") || t.contains("timecode")),
        "{titles:?}"
    );
    shots.take(&mut h, "h3-fixed");
    let _ = std::fs::remove_dir_all(dir);
}

/// The stage display: message and countdown from under the notes (also in
/// Live) and over OSC; the picture for the stage with notes, now and next.
#[test]
fn stage_display_shows_message_countdown_and_notes() {
    use crate::osc::StageCmd;
    let dir = temp_dir("stage");
    let shots = Shots(std::env::var_os("LANTERN_SHOTS").map(PathBuf::from));
    let mut h = harness(&dir);
    h.state_mut().settings.live = false;
    run_until(&mut h, "the slides", |a| a.slides.len() == 3);
    {
        let a = h.state_mut();
        a.slides[0].notes =
            "Welcome everyone – thank the sponsors.\n\nIntroduce the first speaker: \
                             Dr. Jane Miller, head of research. Keep it under a minute."
                .into();
        a.slides[1].title = "Agenda".into();
        a.show.stage.display = Some(1);
        a.show.stage.timer_minutes = 20;
    }
    h.key_press(Key::Space);
    run_until(&mut h, "the pictures", |a| {
        a.fulls.contains_key(&0) && a.fulls.contains_key(&1)
    });
    assert!(
        h.query_by_label("Message to the stage…").is_some() || h.query_by_label("Send").is_some(),
        "stage controls under the notes"
    );

    // Live: the message and the countdown still work.
    h.state_mut().set_live(true);
    h.state_mut().stage.draft = "5 minutes left – please wrap up".into();
    h.run_steps(2);
    h.get_by_label("Send").click();
    h.run_steps(3);
    assert_eq!(
        h.state().stage.message.as_ref().map(|m| m.text.as_str()),
        Some("5 minutes left – please wrap up")
    );
    assert!(h.state().stage.draft.is_empty());
    h.get_by_label("Start").click();
    h.run_steps(3);
    assert!(h.state().stage.timer.running());
    assert_eq!(h.state().stage.timer.length, 20.0 * 60.0);
    shots.take(&mut h, "st1-controls-live");

    // The stage picture, full size.
    h.state_mut().stage.preview = true;
    h.set_size(egui::vec2(1920.0, 1080.0));
    std::thread::sleep(Duration::from_millis(1100));
    h.run_steps(4);
    shots.take(&mut h, "st2-stage");
    h.state_mut().stage.message = None;
    h.state_mut().show.stage.next = false;
    h.run_steps(3);
    shots.take(&mut h, "st3-stage-no-next");
    h.state_mut().stage.preview = false;
    h.state_mut().show.stage.next = true;
    h.set_size(egui::vec2(1400.0, 860.0));

    // Over OSC (Show Master, Companion).
    let a = h.state_mut();
    a.stage_command(StageCmd::Message("Mic 2 is off".into()));
    assert_eq!(
        a.stage.message.as_ref().map(|m| m.text.as_str()),
        Some("Mic 2 is off")
    );
    a.stage_command(StageCmd::ClearMessage);
    assert!(a.stage.message.is_none());
    a.stage_command(StageCmd::TimerPause);
    assert!(!a.stage.timer.running());
    a.stage_command(StageCmd::TimerReset(Some(45)));
    assert_eq!(a.stage.timer.remaining(), 45.0 * 60.0);
    assert_eq!(a.show.stage.timer_minutes, 45);

    // Settings › Stage with its preview.
    a.set_live(false);
    a.settings_open = true;
    a.settings_tab = super::SettingsTab::Stage;
    h.run_steps(4);
    assert!(h.query_by_label("Notes size").is_some());
    shots.take(&mut h, "st4-settings");
    let _ = std::fs::remove_dir_all(dir);
}
