//! Lantern: presentation playback for live shows. Opens PowerPoint files
//! (through LibreOffice), plays them on a display and over NDI, slide by
//! slide with GO or on timecode (LTC, MIDI timecode over RTP-MIDI).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod builds;
mod deck;
mod health;
mod media;
mod ndi;
mod net;
mod office;
mod osc;
mod platform;
mod pptx;
mod show;
mod showfolder;
mod showlog;
mod stage;
mod terminal;
mod testpattern;
mod timecode;
mod web;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("lantern {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let open = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(std::path::PathBuf::from);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Lantern")
        .with_app_id("lantern")
        .with_inner_size([1400.0, 860.0])
        .with_min_inner_size([900.0, 600.0])
        .with_drag_and_drop(true);
    if let Ok(img) = image::load_from_memory(include_bytes!("../assets/icon.png")) {
        let img = img.to_rgba8();
        let (w, h) = img.dimensions();
        viewport = viewport.with_icon(egui::IconData {
            rgba: img.into_raw(),
            width: w,
            height: h,
        });
    }
    eframe::run_native(
        "Lantern",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(app::App::new(&cc.egui_ctx, open)))),
    )
}
