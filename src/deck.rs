//! Slide pictures: a worker thread owns the PDF and renders pages on
//! request, fitted (letterboxed) into the size asked for.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, PixmapSettings, RenderCache, RenderSettings};
use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;

/// RGBA pixels.
#[derive(Clone)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Arc<Vec<u8>>,
}

impl Image {
    pub fn black(w: u32, h: u32) -> Self {
        let mut v = vec![0u8; (w * h * 4) as usize];
        for px in v.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Image {
            w,
            h,
            rgba: Arc::new(v),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// For the output, in its resolution.
    Full,
    /// Small picture for the slide list.
    Thumb,
}

enum Req {
    Render(usize, u32, u32, Kind),
}

pub struct Rendered {
    pub page: usize,
    pub kind: Kind,
    pub image: Image,
}

pub struct Deck {
    pub pages: usize,
    /// Page aspect ratio (width / height).
    pub aspect: f32,
    tx: Sender<Req>,
    rx: Receiver<Rendered>,
}

/// Fit `pw` x `ph` into `w` x `h`: scale and offset.
pub fn fit(pw: f32, ph: f32, w: u32, h: u32) -> (f32, u32, u32) {
    let s = (w as f32 / pw).min(h as f32 / ph);
    let (rw, rh) = ((pw * s).round() as u32, (ph * s).round() as u32);
    (s, w.saturating_sub(rw) / 2, h.saturating_sub(rh) / 2)
}

fn render_page<'a>(
    pdf: &'a Pdf,
    cache: &RenderCache<'a>,
    page: usize,
    w: u32,
    h: u32,
) -> Option<Image> {
    let pages = pdf.pages();
    let p = pages.get(page)?;
    let (pw, ph) = p.render_dimensions();
    let (s, ox, oy) = fit(pw, ph, w, h);
    let settings = PixmapSettings {
        x_scale: s,
        y_scale: s,
        bg_color: WHITE,
    };
    let pm = render(
        p,
        cache,
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &settings,
    );
    let (sw, sh) = (u32::from(pm.width()), u32::from(pm.height()));
    let src = pm.data_as_u8_slice();
    let img = Image::black(w, h);
    let mut out = (*img.rgba).clone();
    for y in 0..sh.min(h.saturating_sub(oy)) {
        let cols = sw.min(w.saturating_sub(ox)) as usize;
        let s0 = (y * sw) as usize * 4;
        let d0 = (((y + oy) * w + ox) as usize) * 4;
        out[d0..d0 + cols * 4].copy_from_slice(&src[s0..s0 + cols * 4]);
    }
    Some(Image {
        w,
        h,
        rgba: Arc::new(out),
    })
}

impl Deck {
    pub fn open(pdf_bytes: Vec<u8>) -> Result<Self, String> {
        let pdf = Pdf::new(pdf_bytes.clone()).map_err(|e| format!("PDF: {e:?}"))?;
        let pages = pdf.pages().len();
        if pages == 0 {
            return Err("The converted presentation has no pages.".into());
        }
        let aspect = pdf
            .pages()
            .first()
            .map(|p| {
                let (w, h) = p.render_dimensions();
                w / h.max(1.0)
            })
            .unwrap_or(16.0 / 9.0);
        drop(pdf);
        let (tx, wrx) = channel::<Req>();
        let (wtx, rx) = channel::<Rendered>();
        std::thread::Builder::new()
            .name("lantern-render".into())
            .spawn(move || {
                let Ok(pdf) = Pdf::new(pdf_bytes) else { return };
                let cache = RenderCache::new();
                let mut queue: VecDeque<Req> = VecDeque::new();
                loop {
                    // Take everything waiting; output pictures first, newest first.
                    loop {
                        match wrx.try_recv() {
                            Ok(r) => queue.push_back(r),
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }
                    let next = queue
                        .iter()
                        .rposition(|Req::Render(_, _, _, k)| *k == Kind::Full)
                        .or(if queue.is_empty() { None } else { Some(0) });
                    let Some(i) = next else {
                        match wrx.recv() {
                            Ok(r) => queue.push_back(r),
                            Err(_) => return,
                        }
                        continue;
                    };
                    let Some(Req::Render(page, w, h, kind)) = queue.remove(i) else {
                        continue;
                    };
                    queue.retain(|Req::Render(p, a, b, k)| {
                        !(*p == page && *a == w && *b == h && *k == kind)
                    });
                    if let Some(image) = render_page(&pdf, &cache, page, w, h) {
                        if wtx.send(Rendered { page, kind, image }).is_err() {
                            return;
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Deck {
            pages,
            aspect,
            tx,
            rx,
        })
    }

    pub fn request(&self, page: usize, w: u32, h: u32, kind: Kind) {
        let _ = self.tx.send(Req::Render(page, w.max(16), h.max(16), kind));
    }

    pub fn poll(&self) -> Vec<Rendered> {
        self.rx.try_iter().collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn fits_with_bars() {
        // 4:3 into 16:9: bars left and right.
        let (s, ox, oy) = fit(400.0, 300.0, 1920, 1080);
        assert!((s - 3.6).abs() < 1e-4);
        assert_eq!((ox, oy), (240, 0));
    }

    #[test]
    fn renders_a_page_with_bars() {
        let pdf = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/demo.pdf");
        let Ok(bytes) = std::fs::read(pdf) else {
            return;
        };
        let deck = Deck::open(bytes).unwrap();
        assert_eq!(deck.pages, 3);
        deck.request(1, 400, 400, Kind::Full);
        let r = loop {
            if let Some(r) = deck.poll().pop() {
                break r;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!((r.image.w, r.image.h, r.page), (400, 400, 1));
        // Top row is a black bar, the middle is the slide.
        assert_eq!(&r.image.rgba[0..4], &[0, 0, 0, 255]);
        let mid = ((200 * 400 + 200) * 4) as usize;
        assert_ne!(&r.image.rgba[mid..mid + 4], &[0, 0, 0, 255]);
    }
}
