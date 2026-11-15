//! The test picture on the outputs: switched on in Settings › Output (or
//! off from the banner, also in Live). The display and the live preview
//! show it drawn here; the NDI sender draws its own (own frame counter).

use super::*;
use crate::testpattern::{time_of_day, Changed, Info, Live, Pattern};

#[derive(Default)]
pub(super) struct TestPicture {
    pub on: bool,
    pattern: Pattern,
    pub tex: Option<TextureHandle>,
    frame: u64,
    next_at: Option<Instant>,
}

impl TestPicture {
    /// Pictures drawn so far.
    #[cfg(test)]
    pub fn frame(&self) -> u64 {
        self.frame
    }
}

impl App {
    /// What the display's test picture says about its output.
    fn test_info_display(&self) -> Info {
        let (w, h) = self.output_size();
        let o = &self.show.output;
        let output = match o.display.map(|d| (d, self.displays.get(d))) {
            Some((d, Some(x))) => format!("Display {} · {}", d + 1, x.name),
            Some((d, None)) => format!("Display {} (not connected)", d + 1),
            None if o.ndi => format!("NDI · {} (no display output)", self.ndi_name()),
            None => "No display output (preview)".to_string(),
        };
        Info {
            output,
            w,
            h,
            fps: self.output_fps(),
            show: self.show_name(),
        }
    }

    /// The test picture for the NDI sender, with the timecode now.
    pub(super) fn test_for_ndi(&self) -> Option<(Info, Option<String>)> {
        if !self.test.on {
            return None;
        }
        let (w, h) = self.output_size();
        Some((
            Info {
                output: format!("NDI · {}", self.ndi_name()),
                w,
                h,
                fps: self.ndi_fps(),
                show: self.show_name(),
            },
            self.timecode_text(),
        ))
    }

    fn show_name(&self) -> String {
        self.show
            .presentation
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    fn timecode_text(&self) -> Option<String> {
        if self.show.timecode.source == TcSource::Off {
            return None;
        }
        Some(match self.tc.reading() {
            Some(r) => show::format_tc(r.secs, r.fps),
            None => "NO SIGNAL".into(),
        })
    }

    /// Every frame: draw the next picture when its time has come.
    pub(super) fn update_test_picture(&mut self, ctx: &egui::Context) {
        if !self.test.on {
            self.test.tex = None;
            self.test.next_at = None;
            return;
        }
        let fps = self.output_fps();
        let step = Duration::from_secs_f64(1.0 / f64::from(fps));
        let now = Instant::now();
        let due = self.test.next_at.is_none_or(|t| now >= t);
        if due {
            let info = self.test_info_display();
            let live = Live {
                frame: self.test.frame,
                time_of_day: time_of_day(),
                timecode: self.timecode_text(),
            };
            let changed = self.test.pattern.draw(&info, &live);
            let size = [info.w as usize, info.h as usize];
            match (self.test.tex.as_mut(), changed) {
                (Some(t), Changed::Areas(areas)) if t.size() == size => {
                    // Only the parts that moved go to the graphics card.
                    for a in areas {
                        if let Some((a, px)) = self.test.pattern.area(a) {
                            let img = egui::ColorImage::from_rgba_premultiplied(
                                [a[2] as usize, a[3] as usize],
                                &px,
                            );
                            t.set_partial(
                                [a[0] as usize, a[1] as usize],
                                img,
                                egui::TextureOptions::LINEAR,
                            );
                        }
                    }
                }
                _ => {
                    let img =
                        egui::ColorImage::from_rgba_premultiplied(size, self.test.pattern.pixels());
                    match self.test.tex.as_mut() {
                        Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                        None => {
                            self.test.tex = Some(ctx.load_texture(
                                "test-picture",
                                img,
                                egui::TextureOptions::LINEAR,
                            ));
                        }
                    }
                }
            }
            self.test.frame += 1;
            // Keep the beat; after a stall start again from now.
            let next = self.test.next_at.map_or(now, |t| t + step);
            self.test.next_at = Some(if next + step < now { now + step } else { next });
        }
        let wait = self
            .test
            .next_at
            .map_or(step, |t| t.saturating_duration_since(Instant::now()));
        ctx.request_repaint_after(wait);
    }

    /// In the top bar while the test picture is on: a red mark and a
    /// button to turn it off (also in Live).
    pub(super) fn test_badge(&mut self, ui: &mut egui::Ui) {
        if !self.test.on {
            return;
        }
        ui.label(
            RichText::new(" TEST PICTURE ON AIR ")
                .strong()
                .size(13.0)
                .color(Color32::WHITE)
                .background_color(DANGER),
        )
        .on_hover_text("The outputs show the test picture instead of the slides");
        if ui
            .add(egui::Button::new(RichText::new("Turn off").size(13.0)).corner_radius(6))
            .on_hover_text("Back to the slides")
            .clicked()
        {
            self.test.on = false;
        }
        ui.add_space(8.0);
    }

    /// Settings › Output: the switch.
    pub(super) fn test_switch(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.checkbox(
            &mut self.test.on,
            RichText::new("Test picture on the outputs").strong(),
        );
        ui.label(
            RichText::new(
                "Instead of the slides, on the display and over NDI, in the output's resolution: \
                 geometry and cropping, colours and levels, sharpness, and a frame counter, the \
                 time and a moving hand to see dropped frames and delay. Not saved – Lantern \
                 starts with the slides.",
            )
            .small()
            .color(MUTED),
        );
    }
}
