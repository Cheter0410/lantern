//! The stage display as pictures, for NDI: drawn by its own egui (the same `paint_stage` as the
//! window) into a texture on the GPU and read back. The read-back is collected one call later, so
//! the window never waits for the GPU.

use super::*;
use eframe::egui_wgpu::{self, wgpu};
use std::sync::mpsc;

/// Draws the stage display off screen at a fixed size.
pub(in crate::app) struct StageRender {
    rs: egui_wgpu::RenderState,
    ctx: egui::Context,
    renderer: egui_wgpu::Renderer,
    pub(in crate::app) size: (u32, u32),
    texture: wgpu::Texture,
    buffer: wgpu::Buffer,
    padded: u32,
    /// A read-back on its way: done when this says so.
    pending: Option<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    /// Pictures uploaded to this egui (by the address of their pixels).
    pics: HashMap<usize, TextureHandle>,
}

/// The pictures for one frame: now, next and the video over now.
pub(in crate::app) struct StagePics {
    pub cur: Option<Image>,
    pub next: Option<Image>,
    pub video: Option<video::Overlay>,
}

impl StageRender {
    pub(in crate::app) fn new(rs: &egui_wgpu::RenderState, size: (u32, u32)) -> Self {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let renderer = egui_wgpu::Renderer::new(&rs.device, format, egui_wgpu::RendererOptions::default());
        let texture = rs.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lantern stage"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = (size.0 * 4).div_ceil(align) * align;
        let buffer = rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lantern stage read-back"),
            size: u64::from(padded) * u64::from(size.1),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        StageRender {
            rs: rs.clone(),
            ctx: egui::Context::default(),
            renderer,
            size,
            texture,
            buffer,
            padded,
            pending: None,
            pics: HashMap::new(),
        }
    }

    fn texture_for(&mut self, img: &Image) -> TextureHandle {
        let key = Arc::as_ptr(&img.rgba) as usize;
        if let Some(t) = self.pics.get(&key) {
            return t.clone();
        }
        // Old pictures go (the slides change, videos every frame).
        if self.pics.len() > 6 {
            self.pics.clear();
        }
        let ci = egui::ColorImage::from_rgba_unmultiplied([img.w as usize, img.h as usize], &img.rgba);
        let t = self.ctx.load_texture("stage-pic", ci, egui::TextureOptions::LINEAR);
        self.pics.insert(key, t.clone());
        t
    }

    /// The picture drawn on an earlier call, once the GPU has it ready (RGBA, `size`).
    pub(in crate::app) fn collect(&mut self) -> Option<Vec<u8>> {
        let rx = self.pending.as_ref()?;
        let _ = self.rs.device.poll(wgpu::PollType::Poll);
        match rx.try_recv() {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                return None;
            }
            Err(mpsc::TryRecvError::Empty) => return None,
        }
        self.pending = None;
        let (w, h) = self.size;
        let row = (w * 4) as usize;
        let mut out = Vec::with_capacity(row * h as usize);
        {
            let slice = self.buffer.slice(..);
            let Ok(data) = slice.get_mapped_range() else {
                self.buffer.unmap();
                return None;
            };
            for r in data.chunks_exact(self.padded as usize).take(h as usize) {
                out.extend_from_slice(&r[..row]);
            }
        }
        self.buffer.unmap();
        Some(out)
    }

    /// Whether a picture is still on its way (then `draw` does nothing).
    pub(in crate::app) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Draw `scene` with `pics`; the result comes from a later `collect`.
    pub(in crate::app) fn draw(&mut self, mut scene: StageScene, pics: StagePics) {
        if self.pending.is_some() {
            return;
        }
        scene.cur = pics.cur.as_ref().map(|i| self.texture_for(i));
        scene.next = pics.next.as_ref().map(|i| self.texture_for(i));
        scene.cur_video = pics.video.map(|mut v| {
            v.tex = self.texture_for(&v.image);
            v
        });
        let (w, h) = self.size;
        let rect = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(w as f32, h as f32));
        let input = egui::RawInput {
            screen_rect: Some(rect),
            ..Default::default()
        };
        let mut out = self.ctx.run_ui(input, |ui| paint_stage(ui, rect, &scene));
        let ppp = out.pixels_per_point;
        let mut delta = std::mem::take(&mut out.textures_delta);
        let prims = self.ctx.tessellate(out.shapes, ppp);
        let (device, queue) = (&self.rs.device, &self.rs.queue);
        for (id, deltas) in delta.set.drain() {
            for d in deltas {
                self.renderer.update_texture(device, queue, id, &d);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [w, h], pixels_per_point: ppp };
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("lantern stage") });
        let user = self.renderer.update_buffers(device, queue, &mut enc, &prims, &screen);
        let view = self.texture.create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut pass = enc
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("lantern stage"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &prims, &screen);
        }
        enc.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit(user.into_iter().chain(std::iter::once(enc.finish())));
        for id in delta.free.drain() {
            self.renderer.free_texture(&id);
        }
        let (tx, rx) = mpsc::channel();
        self.buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.pending = Some(rx);
    }

    /// Draw and wait for the picture (tests).
    #[cfg(test)]
    pub(in crate::app) fn draw_now(&mut self, scene: StageScene, pics: StagePics) -> Option<Vec<u8>> {
        self.draw(scene, pics);
        let _ = self.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(10)) });
        self.collect()
    }
}

impl App {
    /// The pictures the stage display shows, as pixels.
    pub(in crate::app) fn stage_pics(&self) -> StagePics {
        let full = |p: Option<usize>| p.and_then(|p| self.fulls.get(&p)).map(|f| f.image.clone());
        StagePics {
            cur: full(self.live_page()),
            next: full(self.cue_page(self.next_cue())),
            video: self.overlays().1.cloned(),
        }
    }

    /// The stage display as an NDI source (Settings › Stage).
    pub(in crate::app) fn update_stage_ndi(&mut self, frame: &eframe::Frame) {
        let cfg = self.show.stage.clone();
        if !cfg.ndi {
            self.stage_out.sender = None;
            self.stage_out.render = None;
            self.stage_out.error = None;
            return;
        }
        let size = cfg.ndi_size();
        // Moving clocks and a flashing message: 25 pictures a second are plenty.
        let fps = 25;
        let name = if cfg.ndi_name.trim().is_empty() { "Lantern Stage".to_string() } else { cfg.ndi_name.trim().to_string() };
        let fresh = self.stage_out.sender.as_ref().is_some_and(|s| s.name == name && s.size == size);
        if !fresh {
            self.stage_out.sender = None;
            match ndi::FrameSender::start(&name, size.0, size.1, fps) {
                Ok(s) => {
                    self.stage_out.sender = Some(s);
                    self.stage_out.error = None;
                }
                Err(e) => {
                    self.stage_out.error = Some(e);
                    return;
                }
            }
        }
        let Some(rs) = frame.wgpu_render_state() else {
            return;
        };
        if self.stage_out.render.as_ref().is_none_or(|r| r.size != size) {
            self.stage_out.render = Some(StageRender::new(rs, size));
        }
        // Nobody watching: no need to draw (the sender repeats the last picture).
        let watched = self.stage_out.sender.as_ref().and_then(ndi::FrameSender::receivers) != Some(0);
        let due = self.stage_out.at.is_none_or(|t| t.elapsed() >= Duration::from_millis(1000 / u64::from(fps)));
        let scene = (watched && due).then(|| (self.stage_scene(), self.stage_pics()));
        let (Some(render), Some(sender)) = (self.stage_out.render.as_mut(), self.stage_out.sender.as_ref()) else {
            return;
        };
        if let Some(px) = render.collect() {
            sender.set(Arc::new(px));
        }
        if let Some((scene, pics)) = scene {
            if !render.busy() {
                render.draw(scene, pics);
                self.stage_out.at = Some(Instant::now());
            }
        }
    }
}

/// The NDI stage output.
#[derive(Default)]
pub(in crate::app) struct StageOut {
    pub sender: Option<ndi::FrameSender>,
    pub render: Option<StageRender>,
    pub error: Option<String>,
    at: Option<Instant>,
}
