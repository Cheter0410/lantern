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
    assert!(h.query_by_label("Öffnen").is_some());

    // Edit: clicking a slide opens its settings.
    click_at(&h, egui::pos2(150.0, 80.0));
    h.run_steps(3);
    assert_eq!(h.state().selected, Some(0), "slide settings open in Edit");
    assert!(h.query_by_label("Jetzt zeigen").is_some());
    shots.take(&mut h, "1-edit-slide-settings");

    // Switch to Live with the button: settings close, Öffnen is gone.
    h.get_by_label("Live").click();
    h.run_steps(3);
    assert!(h.state().live());
    assert_eq!(h.state().selected, None, "Live closes the slide settings");
    assert!(h.query_by_label("Jetzt zeigen").is_none());
    assert!(h.query_by_label("Öffnen").is_none());
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
    // Hovering a button must not make it grow (nothing moves).
    h.get_by_label("Einstellungen").hover();
    h.run_steps(3);
    let v = h.ctx.global_style().visuals.clone();
    for w in [&v.widgets.hovered, &v.widgets.active, &v.widgets.inactive] {
        assert_eq!(w.expansion, 0.0, "buttons keep their size");
    }
    shots.take(&mut h, "2b-live-running-hover");

    // Live: the settings can be looked at, not changed.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    h.run_steps(3);
    assert!(h.state().settings_open);
    assert!(h.query_by_label_contains("nur ansehen").is_some());
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
    assert!(h.query_by_label("Öffnen").is_some());
    shots.take(&mut h, "4-edit-again");

    let _ = std::fs::remove_dir_all(&dir);
}
