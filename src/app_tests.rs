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
    std::env::set_var("LANTERN_CONFIG_DIR", dir.join("config"));
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

    // GO- and GO+ side by side; Blackout and Clear set apart on the right.
    let back = h.get_by_label("GO\u{2212}").rect();
    let go = h.get_by_label_contains("GO+").rect();
    let blackout = h.get_by_label("Blackout").rect();
    let clear = h.get_by_label("Clear").rect();
    assert!(back.right() < go.left(), "GO- left of GO+");
    assert!(
        go.right() + 100.0 < blackout.left(),
        "Blackout far from GO+"
    );
    assert!(
        blackout.right() + 24.0 < clear.left(),
        "a clear gap between Blackout and Clear"
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
    std::env::set_var("LANTERN_CONFIG_DIR", dir.join("config"));
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
