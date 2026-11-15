//! A moving test picture for the outputs, drawn pixel for pixel in the
//! output's resolution (so scaling, cropping and dropped frames show):
//! grid and circles (geometry, aspect), corner marks (cropping, mirroring),
//! safe areas, colour bars, grey steps and ramps (levels, banding), one
//! pixel line patches (sharpness, scaling), and live information: output,
//! resolution, frame rate, frame counter, time of day, timecode. Moving
//! parts: a hand turning once a second, one lit box per frame, a sweep
//! across the picture and a flash every full second.

use crate::deck::Image;
use std::sync::Arc;

/// What the picture says about the output it is on.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    /// "DISPLAY 2 · DELL U2720Q" or "NDI · LANTERN".
    pub output: String,
    pub w: u32,
    pub h: u32,
    pub fps: u32,
    /// The presentation open (or empty).
    pub show: String,
}

/// Changes every frame.
#[derive(Clone, Debug, Default)]
pub struct Live {
    /// Frames sent on this output.
    pub frame: u64,
    /// Seconds since midnight (local time).
    pub time_of_day: f64,
    /// Timecode in, formatted, if any.
    pub timecode: Option<String>,
}

/// Seconds since midnight, local time.
pub fn time_of_day() -> f64 {
    use chrono::Timelike;
    let now = chrono::Local::now();
    f64::from(now.num_seconds_from_midnight()) + f64::from(now.nanosecond() % 1_000_000_000) / 1e9
}

/// Keeps the still part, draws the moving part on top for each frame.
#[derive(Default)]
pub struct Pattern {
    base: Option<(Info, Canvas)>,
}

impl Pattern {
    pub fn render(&mut self, info: &Info, live: &Live) -> Image {
        if self.base.as_ref().is_none_or(|(i, _)| i != info) {
            self.base = Some((info.clone(), base(info)));
        }
        let Some((_, base)) = &self.base else {
            return Image::black(info.w.max(1), info.h.max(1));
        };
        let mut c = base.clone();
        moving(&mut c, info, live);
        c.into_image()
    }
}

#[derive(Clone)]
struct Canvas {
    w: i32,
    h: i32,
    px: Vec<u8>,
}

type Rgb = [u8; 3];

const WHITE: Rgb = [255, 255, 255];
const BLACK: Rgb = [0, 0, 0];
const BG: Rgb = [46, 48, 54];
const GRID: Rgb = [120, 124, 132];
const PANEL: Rgb = [12, 13, 16];
const DIM: Rgb = [150, 155, 165];
const ACCENT: Rgb = [255, 196, 92];
const LIVE_BLUE: Rgb = [59, 130, 246];
const GREEN: Rgb = [34, 197, 94];

impl Canvas {
    fn new(w: u32, h: u32, fill: Rgb) -> Self {
        let mut px = vec![255u8; (w * h * 4) as usize];
        for p in px.chunks_exact_mut(4) {
            p[..3].copy_from_slice(&fill);
        }
        Canvas {
            w: w as i32,
            h: h as i32,
            px,
        }
    }

    fn into_image(self) -> Image {
        Image {
            w: self.w as u32,
            h: self.h as u32,
            rgba: Arc::new(self.px),
        }
    }

    fn set(&mut self, x: i32, y: i32, c: Rgb) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            let i = ((y * self.w + x) * 4) as usize;
            self.px[i..i + 3].copy_from_slice(&c);
        }
    }

    /// Blend `c` over the pixel with coverage `a` (0..1).
    fn blend(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if a <= 0.0 || x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let a = a.min(1.0);
        let i = ((y * self.w + x) * 4) as usize;
        for (p, c) in self.px[i..i + 3].iter_mut().zip(c) {
            *p = (f32::from(*p) * (1.0 - a) + f32::from(c) * a).round() as u8;
        }
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x + w).min(self.w), (y + h).min(self.h));
        for yy in y0..y1 {
            for xx in x0..x1 {
                let i = ((yy * self.w + xx) * 4) as usize;
                self.px[i..i + 3].copy_from_slice(&c);
            }
        }
    }

    /// Outline, `t` pixels thick, inside the box.
    fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, t: i32, c: Rgb) {
        self.rect(x, y, w, t, c);
        self.rect(x, y + h - t, w, t, c);
        self.rect(x, y, t, h, c);
        self.rect(x + w - t, y, t, h, c);
    }

    /// Dashed outline (1 px), for the safe areas.
    fn dashed(&mut self, x: i32, y: i32, w: i32, h: i32, dash: i32, c: Rgb) {
        let on = |i: i32| (i / dash.max(1)) % 2 == 0;
        for i in 0..w {
            if on(i) {
                self.set(x + i, y, c);
                self.set(x + i, y + h - 1, c);
            }
        }
        for i in 0..h {
            if on(i) {
                self.set(x, y + i, c);
                self.set(x + w - 1, y + i, c);
            }
        }
    }

    /// Anti-aliased ring.
    fn ring(&mut self, cx: f32, cy: f32, r: f32, t: f32, c: Rgb) {
        let ext = r + t + 1.0;
        for y in (cy - ext).floor() as i32..=(cy + ext).ceil() as i32 {
            for x in (cx - ext).floor() as i32..=(cx + ext).ceil() as i32 {
                let d = ((x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) - r).abs();
                self.blend(x, y, c, t / 2.0 + 0.5 - d);
            }
        }
    }

    fn disc(&mut self, cx: f32, cy: f32, r: f32, c: Rgb) {
        for y in (cy - r - 1.0).floor() as i32..=(cy + r + 1.0).ceil() as i32 {
            for x in (cx - r - 1.0).floor() as i32..=(cx + r + 1.0).ceil() as i32 {
                let d = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
                self.blend(x, y, c, r + 0.5 - d);
            }
        }
    }

    /// Anti-aliased line `t` pixels thick.
    fn line(&mut self, a: (f32, f32), b: (f32, f32), t: f32, c: Rgb) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        let pad = t + 1.0;
        let (x0, x1) = (a.0.min(b.0) - pad, a.0.max(b.0) + pad);
        let (y0, y1) = (a.1.min(b.1) - pad, a.1.max(b.1) + pad);
        for y in y0.floor() as i32..=y1.ceil() as i32 {
            for x in x0.floor() as i32..=x1.ceil() as i32 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let k = (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0);
                let d = (px - (a.0 + k * dx)).hypot(py - (a.1 + k * dy));
                self.blend(x, y, c, t / 2.0 + 0.5 - d);
            }
        }
    }

    /// Text in the 5x7 font, `s` pixels per font pixel. Returns the width.
    fn text(&mut self, x: i32, y: i32, s: i32, text: &str, c: Rgb) -> i32 {
        let mut cx = x;
        for ch in text.chars() {
            let g = glyph(ch);
            for (row, bits) in g.iter().enumerate() {
                for col in 0..5 {
                    if bits & (0x10 >> col) != 0 {
                        self.rect(cx + col * s, y + row as i32 * s, s, s, c);
                    }
                }
            }
            cx += 6 * s;
        }
        cx - x - s
    }

    fn text_centered(&mut self, cx: i32, y: i32, s: i32, text: &str, c: Rgb) {
        let w = text_width(text, s);
        self.text(cx - w / 2, y, s, text, c);
    }
}

fn text_width(text: &str, s: i32) -> i32 {
    (text.chars().count() as i32 * 6 - 1).max(0) * s
}

/// 5x7 pixel font, upper case (lower case is shown as upper case).
fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        ' ' => [0; 7],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, 0x08],
        '-' | '–' | '−' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        '\'' => [0x0C, 0x04, 0x08, 0x00, 0x00, 0x00, 0x00],
        '"' => [0x0A, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00],
        '·' | '*' => [0x00, 0x00, 0x0C, 0x0C, 0x00, 0x00, 0x00],
        '×' => [0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x00],
        '#' => [0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A],
        '=' => [0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        'Ä' => [0x0A, 0x0E, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'Ö' => [0x0A, 0x0E, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'Ü' => [0x0A, 0x00, 0x11, 0x11, 0x11, 0x11, 0x0E],
        _ => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04], // ?
    }
}

/// Sizes that follow the picture height (1080 lines = 1).
struct Layout {
    w: i32,
    h: i32,
    /// Grid cell (square).
    cell: i32,
    /// Grid origin, so the centre lines go through the middle.
    gx: i32,
    gy: i32,
    /// Font pixel size: big and small.
    big: i32,
    small: i32,
    /// Centre panel.
    panel: (i32, i32, i32, i32),
}

fn layout(info: &Info) -> Layout {
    let (w, h) = (info.w as i32, info.h as i32);
    let u = h as f32 / 1080.0;
    let cell = ((h as f32 / 12.0).round() as i32).max(8);
    let big = ((5.0 * u).round() as i32).max(1);
    let small = ((3.0 * u).round() as i32).max(1);
    let pw = (cell * 9).min(w - 2 * cell);
    let ph = cell * 4;
    let panel = ((w - pw) / 2, (h - ph) / 2, pw, ph);
    Layout {
        w,
        h,
        cell,
        gx: (w / 2) % cell,
        gy: (h / 2) % cell,
        big,
        small,
        panel,
    }
}

/// The still part.
fn base(info: &Info) -> Canvas {
    let l = layout(info);
    let (w, h, cell) = (l.w, l.h, l.cell);
    let mut c = Canvas::new(info.w.max(1), info.h.max(1), BG);

    // Grid, centre cross a bit brighter.
    let mut x = l.gx;
    while x < w {
        c.rect(x, 0, 1, h, GRID);
        x += cell;
    }
    let mut y = l.gy;
    while y < h {
        c.rect(0, y, w, 1, GRID);
        y += cell;
    }
    c.rect(w / 2, 0, 1, h, WHITE);
    c.rect(0, h / 2, w, 1, WHITE);

    // Big circle (round = right aspect) and one in every corner.
    let r = h as f32 * 0.45;
    c.ring(w as f32 / 2.0, h as f32 / 2.0, r, 2.0, WHITE);
    let cr = cell as f32 * 1.4;
    for (cx, cy) in [
        (cr + cell as f32 * 0.4, cr + cell as f32 * 0.4),
        (w as f32 - cr - cell as f32 * 0.4, cr + cell as f32 * 0.4),
        (cr + cell as f32 * 0.4, h as f32 - cr - cell as f32 * 0.4),
        (
            w as f32 - cr - cell as f32 * 0.4,
            h as f32 - cr - cell as f32 * 0.4,
        ),
    ] {
        c.ring(cx, cy, cr, 2.0, WHITE);
    }

    // Safe areas: 93 % (action) and 90 % (title), dashed.
    for (k, col) in [(0.93, GREEN), (0.90, ACCENT)] {
        let (sw, sh) = ((w as f32 * k) as i32, (h as f32 * k) as i32);
        c.dashed((w - sw) / 2, (h - sh) / 2, sw, sh, (cell / 8).max(3), col);
    }

    // Edge: 1 px white, then 1 px black, so a lost row or column shows.
    c.frame(0, 0, w, h, 1, WHITE);
    c.frame(1, 1, w - 2, h - 2, 1, BLACK);
    // Corner marks with their names (cropping, mirroring, rotation).
    let m = cell / 2;
    let s = l.small;
    for (x, y, col, name) in [
        (2, 2, [220, 38, 38], "TOP LEFT"),
        (w - 2 - m, 2, GREEN, "TOP RIGHT"),
        (2, h - 2 - m, LIVE_BLUE, "BOTTOM LEFT"),
        (w - 2 - m, h - 2 - m, ACCENT, "BOTTOM RIGHT"),
    ] {
        c.rect(x, y, m, m, col);
        let tw = text_width(name, s);
        let tx = if x < w / 2 {
            x + m + s * 3
        } else {
            x - s * 3 - tw
        };
        let ty = if y < h / 2 { y + s } else { y + m - 8 * s };
        c.text(tx, ty, s, name, col);
    }

    // Colour bars (75 %) above the panel, grey steps below it.
    let (px, py, pw, ph) = l.panel;
    let bar_h = cell;
    let bars: [Rgb; 7] = [
        [191, 191, 191],
        [191, 191, 0],
        [0, 191, 191],
        [0, 191, 0],
        [191, 0, 191],
        [191, 0, 0],
        [0, 0, 191],
    ];
    let by = py - bar_h - cell / 3;
    for (i, col) in bars.iter().enumerate() {
        let x0 = px + pw * i as i32 / 7;
        let x1 = px + pw * (i as i32 + 1) / 7;
        c.rect(x0, by, x1 - x0, bar_h, *col);
    }
    let gy = py + ph + cell / 3;
    for i in 0..11 {
        let v = (i * 255 / 10) as u8;
        let x0 = px + pw * i / 11;
        let x1 = px + pw * (i + 1) / 11;
        c.rect(x0, gy, x1 - x0, bar_h * 2 / 3, [v, v, v]);
        let label = format!("{}", i * 10);
        let col = if v > 128 { BLACK } else { WHITE };
        c.text_centered((x0 + x1) / 2, gy + bar_h / 3 - 4 * s, s, &label, col);
    }
    // Smooth ramps (banding): grey, red, green, blue.
    let ry = gy + bar_h * 2 / 3;
    let rh = (bar_h / 6).max(2);
    for (k, mask) in [[1, 1, 1], [1, 0, 0], [0, 1, 0], [0, 0, 1]]
        .iter()
        .enumerate()
    {
        for x in 0..pw {
            let v = (x * 255 / (pw - 1).max(1)) as u8;
            let col = [v * mask[0], v * mask[1], v * mask[2]];
            c.rect(px + x, ry + k as i32 * rh, 1, rh, col);
        }
    }

    // Sharpness patches left and right of the panel: 1 px lines both ways,
    // 2 px lines, 1 px checkerboard.
    let patch = cell;
    let side_x = [px - cell / 2 - patch * 2, px + pw + cell / 2];
    for sx in side_x {
        let sy = py;
        for (k, kind) in [0, 1, 2, 3].iter().enumerate() {
            let x0 = sx + (k as i32 % 2) * patch;
            let y0 = sy + (k as i32 / 2) * patch * 2;
            for y in 0..patch * 2 {
                for x in 0..patch {
                    let on = match kind {
                        0 => x % 2 == 0,
                        1 => y % 2 == 0,
                        2 => (x / 2) % 2 == 0,
                        _ => (x + y) % 2 == 0,
                    };
                    c.set(x0 + x, y0 + y, if on { WHITE } else { BLACK });
                }
            }
        }
    }

    // Centre panel with the still text.
    c.rect(px, py, pw, ph, PANEL);
    c.frame(px, py, pw, ph, 2, WHITE);
    let b = l.big;
    c.text_centered(w / 2, py + cell / 4, b, "LANTERN TEST PICTURE", WHITE);
    let line2 = format!("{} × {}  ·  {} FPS", info.w, info.h, info.fps);
    c.text_centered(w / 2, py + cell / 4 + 11 * b, s, &line2, ACCENT);
    let out = info.output.to_uppercase();
    c.text_centered(w / 2, py + cell / 4 + 11 * b + 11 * s, s, &out, DIM);
    if !info.show.is_empty() {
        let show = format!("SHOW: {}", info.show);
        c.text_centered(w / 2, py + ph - cell / 4 - 7 * s, s, &show, DIM);
    }
    c
}

/// Clock text `hh:mm:ss:ff`.
fn clock(secs: f64, fps: u32) -> String {
    let s = secs.max(0.0);
    let whole = s.floor() as u64;
    let ff = ((s - s.floor()) * f64::from(fps.max(1))).floor() as u64;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        (whole / 3600) % 24,
        (whole / 60) % 60,
        whole % 60,
        ff
    )
}

/// The moving part.
fn moving(c: &mut Canvas, info: &Info, live: &Live) {
    let l = layout(info);
    let (w, h, cell) = (l.w, l.h, l.cell);
    let (px, py, pw, ph) = l.panel;
    let fps = info.fps.max(1);
    let n = live.frame;
    let in_second = (n % u64::from(fps)) as i32;
    let s = l.small;
    let b = l.big;

    // Frame counter and time of day in the panel.
    let mid = py + ph / 2 + cell / 6;
    let left = format!("FRAME {:07}", n);
    let right = clock(live.time_of_day, fps);
    c.text(px + cell / 3, mid, b, &left, WHITE);
    let rw = text_width(&right, b);
    c.text(px + pw - cell / 3 - rw, mid, b, &right, WHITE);
    let tc = match &live.timecode {
        Some(t) => format!("TC IN {t}"),
        None => "TC IN OFF".to_string(),
    };
    c.text_centered(w / 2, mid + 9 * b, s, &tc, LIVE_BLUE);

    // One box per frame of the second: the lit one moves on every frame,
    // a dropped or doubled frame shows as a jump or a stop.
    let boxes = fps.min(60) as i32;
    let row_y = py + ph + cell / 3 + cell * 2 / 3 + (cell / 6).max(2) * 4 + cell / 4;
    let bw = (pw / boxes).max(2);
    let row_x = px + (pw - bw * boxes) / 2;
    for i in 0..boxes {
        let lit = i == in_second % boxes;
        let col = if lit { WHITE } else { [70, 74, 82] };
        c.rect(row_x + i * bw + 1, row_y, bw - 2, cell / 3, col);
    }

    // A hand turning once a second, in a dial left of the panel.
    let dial_r = cell as f32 * 0.9;
    let (dx, dy) = ((px - cell * 3 / 2) as f32, (py + ph + cell * 6 / 5) as f32);
    if dx - dial_r > 0.0 {
        c.disc(dx, dy, dial_r, PANEL);
        c.ring(dx, dy, dial_r, 2.0, WHITE);
        for k in 0..fps.min(60) {
            let a = k as f32 / fps.min(60) as f32 * std::f32::consts::TAU;
            let (sa, ca) = a.sin_cos();
            c.set(
                (dx + sa * dial_r * 0.85) as i32,
                (dy - ca * dial_r * 0.85) as i32,
                DIM,
            );
        }
        let a = in_second as f32 / fps as f32 * std::f32::consts::TAU;
        let (sa, ca) = a.sin_cos();
        c.line(
            (dx, dy),
            (dx + sa * dial_r * 0.8, dy - ca * dial_r * 0.8),
            3.0,
            ACCENT,
        );
        c.disc(dx, dy, 4.0, ACCENT);
    }

    // Flash on the first frame of every second (lines up with a beep or a
    // camera; shows the output's delay against the clock).
    let fx = px + pw + cell;
    let fy = py + ph + cell * 6 / 5 - cell / 2;
    if fx + cell < w {
        let col = if in_second == 0 { WHITE } else { PANEL };
        c.rect(fx, fy, cell, cell, col);
        c.frame(fx, fy, cell, cell, 1, DIM);
        c.text_centered(fx + cell / 2, fy + cell + s * 2, s, "1 S", DIM);
    }

    // A bar sweeping across the whole picture, 4 seconds per pass, one step
    // per frame: smooth motion, tearing and judder show here.
    let period = u64::from(fps) * 4;
    let k = (n % period) as f32 / period as f32;
    let bar_w = (w / 200).max(2);
    let x = (k * (w - bar_w) as f32) as i32;
    let y0 = h - cell + cell / 4;
    c.rect(x, y0, bar_w, cell / 2, WHITE);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn info(w: u32, h: u32) -> Info {
        Info {
            output: "Display 2 · Test".into(),
            w,
            h,
            fps: 25,
            show: "talk.pptx".into(),
        }
    }

    fn px(img: &Image, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * img.w + x) * 4) as usize;
        [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]]
    }

    #[test]
    fn edges_corners_and_size() {
        let mut p = Pattern::default();
        let img = p.render(&info(1920, 1080), &Live::default());
        assert_eq!((img.w, img.h), (1920, 1080));
        assert_eq!(img.rgba.len(), 1920 * 1080 * 4);
        // Outermost pixels white, the next ones black.
        for (x, y) in [(0, 500), (1919, 500), (900, 0), (900, 1079)] {
            assert_eq!(px(&img, x, y), WHITE, "edge at {x},{y}");
        }
        assert_eq!(px(&img, 1, 500), BLACK);
        // Corner marks in their colours.
        assert_eq!(px(&img, 10, 10), [220, 38, 38]);
        assert_eq!(px(&img, 1909, 10), GREEN);
        // Centre lines.
        assert_eq!(px(&img, 960, 100), WHITE);
    }

    #[test]
    fn moves_every_frame_and_flashes_each_second() {
        let mut p = Pattern::default();
        let i = info(1280, 720);
        let a = p.render(
            &i,
            &Live {
                frame: 0,
                ..Default::default()
            },
        );
        let b = p.render(
            &i,
            &Live {
                frame: 1,
                ..Default::default()
            },
        );
        let c = p.render(
            &i,
            &Live {
                frame: 1,
                ..Default::default()
            },
        );
        assert!(a.rgba != b.rgba, "frame 0 and 1 differ");
        assert!(b.rgba == c.rgba, "the same frame is the same picture");
        let l = layout(&i);
        let (px0, py, pw, ph) = l.panel;
        let (fx, fy) = (
            px0 + pw + l.cell + 5,
            py + ph + l.cell * 6 / 5 - l.cell / 2 + 5,
        );
        assert_eq!(
            px(&a, fx as u32, fy as u32),
            WHITE,
            "flash at the full second"
        );
        assert_eq!(px(&b, fx as u32, fy as u32), PANEL);
        let d = p.render(
            &i,
            &Live {
                frame: 25,
                ..Default::default()
            },
        );
        assert_eq!(
            px(&d, fx as u32, fy as u32),
            WHITE,
            "and again a second later"
        );
    }

    #[test]
    fn small_and_odd_sizes_do_not_panic() {
        let mut p = Pattern::default();
        for (w, h) in [
            (320, 240),
            (1024, 768),
            (3840, 2160),
            (1080, 1920),
            (33, 17),
        ] {
            let img = p.render(
                &info(w, h),
                &Live {
                    frame: 7,
                    time_of_day: 3600.5,
                    timecode: Some("01:00:00:00".into()),
                },
            );
            assert_eq!(img.rgba.len(), (w * h * 4) as usize);
        }
    }

    #[test]
    fn clock_text() {
        assert_eq!(clock(3661.5, 30), "01:01:01:15");
        assert_eq!(clock(86399.99, 25), "23:59:59:24");
    }

    /// With `LANTERN_SHOTS=<folder>`: a picture to look at.
    #[test]
    fn picture_for_looking_at() {
        let Some(dir) = std::env::var_os("LANTERN_SHOTS") else {
            return;
        };
        let mut p = Pattern::default();
        let live = Live {
            frame: 1234,
            time_of_day: 15.0 * 3600.0 + 51.0 * 60.0 + 2.4,
            timecode: Some("01:00:12:05".into()),
        };
        let img = p.render(&info(1920, 1080), &live);
        let out = image::RgbaImage::from_raw(img.w, img.h, (*img.rgba).clone()).unwrap();
        let _ = std::fs::create_dir_all(&dir);
        out.save(std::path::Path::new(&dir).join("testpattern-1080.png"))
            .unwrap();
    }
}
