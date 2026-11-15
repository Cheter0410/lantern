//! NDI output: a sender thread that sends the output picture at the chosen
//! frame rate, mixing transitions and blackouts itself so they stay smooth.
//! The NDI runtime (from ndi.video, e.g. with NDI Tools) is loaded at run time.
#![allow(unsafe_code)]

use crate::deck::Image;
use libloading::Library;
use std::ffi::{c_char, c_void, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const WEBSITE: &str = "https://ndi.video/tools/";

#[repr(C)]
struct SendCreate {
    name: *const c_char,
    groups: *const c_char,
    clock_video: bool,
    clock_audio: bool,
}

#[repr(C)]
struct VideoFrame {
    xres: i32,
    yres: i32,
    fourcc: u32,
    rate_n: i32,
    rate_d: i32,
    aspect: f32,
    format: i32,
    timecode: i64,
    data: *const u8,
    stride: i32,
    metadata: *const c_char,
    timestamp: i64,
}

const FOURCC_RGBX: u32 = u32::from_le_bytes(*b"RGBX");
/// NDIlib_send_timecode_synthesize
const SYNTHESIZE: i64 = i64::MAX;

type Handle = *mut c_void;

struct Api {
    _lib: Library,
    send_create: unsafe extern "C" fn(*const SendCreate) -> Handle,
    send_video: unsafe extern "C" fn(Handle, *const VideoFrame),
    send_destroy: unsafe extern "C" fn(Handle),
}

// The library's functions may be called from any thread.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

const LIB_NAME: &str = if cfg!(windows) {
    "Processing.NDI.Lib.x64.dll"
} else if cfg!(target_os = "macos") {
    "libndi.dylib"
} else {
    "libndi.so"
};

fn candidates() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
    {
        if cfg!(target_os = "macos") {
            v.push(
                dir.join("../Frameworks")
                    .join(LIB_NAME)
                    .display()
                    .to_string(),
            );
        }
        v.push(dir.join(LIB_NAME).display().to_string());
    }
    for var in ["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5"] {
        if let Ok(dir) = std::env::var(var) {
            v.push(format!("{dir}/{LIB_NAME}"));
        }
    }
    if cfg!(windows) {
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            v.push(
                PathBuf::from(pf)
                    .join("NDI")
                    .join("NDI 6 Runtime")
                    .join("v6")
                    .join(LIB_NAME)
                    .display()
                    .to_string(),
            );
        }
    }
    if cfg!(target_os = "macos") {
        v.push("/usr/local/lib/libndi.dylib".into());
        v.push("/Library/NDI SDK for Apple/lib/macOS/libndi.dylib".into());
        v.push(
            "/Applications/NDI Video Monitor.app/Contents/Frameworks/libndi_advanced.dylib".into(),
        );
    }
    if cfg!(target_os = "linux") {
        v.push("libndi.so.6".into());
        v.push("libndi.so.5".into());
    }
    v.push(LIB_NAME.into());
    v
}

fn load() -> Result<Api, String> {
    for path in candidates() {
        // SAFETY: loading the NDI runtime and looking up its documented C functions.
        unsafe {
            let Ok(lib) = Library::new(&path) else {
                continue;
            };
            macro_rules! sym {
                ($n:literal) => {
                    match lib.get($n) {
                        Ok(s) => *s,
                        Err(e) => return Err(format!("NDI library incomplete: {e}")),
                    }
                };
            }
            let init: unsafe extern "C" fn() -> bool = sym!(b"NDIlib_initialize\0");
            if !init() {
                return Err("NDI does not run on this processor".into());
            }
            return Ok(Api {
                send_create: sym!(b"NDIlib_send_create\0"),
                send_video: sym!(b"NDIlib_send_send_video_v2\0"),
                send_destroy: sym!(b"NDIlib_send_destroy\0"),
                _lib: lib,
            });
        }
    }
    Err("NDI runtime not found – install NDI Tools from ndi.video and restart Lantern.".into())
}

/// What the output shows, with the timing to animate it.
#[derive(Clone)]
pub struct Scene {
    pub previous: Option<Image>,
    pub current: Option<Image>,
    pub since: Instant,
    pub fade: f32,
    pub black_from: f32,
    pub black_to: f32,
    pub black_since: Instant,
    pub black_fade: f32,
}

impl Scene {
    pub fn mix(&self) -> f32 {
        if self.fade <= 0.0 {
            1.0
        } else {
            (self.since.elapsed().as_secs_f32() / self.fade).clamp(0.0, 1.0)
        }
    }
    pub fn black(&self) -> f32 {
        let t =
            (self.black_since.elapsed().as_secs_f32() / self.black_fade.max(0.001)).clamp(0.0, 1.0);
        self.black_from + (self.black_to - self.black_from) * t
    }
}

/// `out = (a·(1-m) + b·m)·(1-black)`; a missing picture is black.
pub fn compose(
    a: Option<&Image>,
    b: Option<&Image>,
    m: f32,
    black: f32,
    w: u32,
    h: u32,
    out: &mut Vec<u8>,
) {
    let n = (w * h * 4) as usize;
    out.resize(n, 0);
    let (a, b) = (
        a.filter(|i| i.w == w && i.h == h),
        b.filter(|i| i.w == w && i.h == h),
    );
    let ka = ((1.0 - m) * (1.0 - black) * 256.0) as u32;
    let kb = (m * (1.0 - black) * 256.0) as u32;
    match (a, b) {
        (_, Some(b)) if ka == 0 && kb == 256 => out.copy_from_slice(&b.rgba),
        _ => {
            let za = vec![];
            let zb = vec![];
            let pa: &[u8] = a.map(|i| i.rgba.as_slice()).unwrap_or(&za);
            let pb: &[u8] = b.map(|i| i.rgba.as_slice()).unwrap_or(&zb);
            for (i, o) in out.iter_mut().enumerate() {
                if i % 4 == 3 {
                    *o = 255;
                    continue;
                }
                let va = pa.get(i).map_or(0, |v| u32::from(*v));
                let vb = pb.get(i).map_or(0, |v| u32::from(*v));
                *o = ((va * ka + vb * kb) >> 8).min(255) as u8;
            }
        }
    }
}

pub struct Sender {
    scene: Arc<Mutex<Option<Scene>>>,
    stop: Arc<AtomicBool>,
    pub name: String,
    pub size: (u32, u32),
    pub fps: u32,
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

static API: Mutex<Option<Result<Arc<Api>, String>>> = Mutex::new(None);

fn api() -> Result<Arc<Api>, String> {
    let mut g = API.lock().map_err(|_| "NDI".to_string())?;
    g.get_or_insert_with(|| load().map(Arc::new)).clone()
}

impl Sender {
    pub fn start(name: &str, w: u32, h: u32, fps: u32) -> Result<Sender, String> {
        let api = api()?;
        let scene: Arc<Mutex<Option<Scene>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (scene2, stop2) = (scene.clone(), stop.clone());
        let cname = CString::new(name).map_err(|_| "Invalid NDI name")?;
        let fps = fps.clamp(1, 120);
        std::thread::Builder::new()
            .name("lantern-ndi".into())
            .spawn(move || {
                let cfg = SendCreate {
                    name: cname.as_ptr(),
                    groups: std::ptr::null(),
                    clock_video: true,
                    clock_audio: false,
                };
                // SAFETY: valid settings; the sender is destroyed below.
                let snd = unsafe { (api.send_create)(&cfg) };
                if snd.is_null() {
                    return;
                }
                let mut buf: Vec<u8> = Vec::new();
                while !stop2.load(Ordering::Relaxed) {
                    let s = scene2.lock().ok().and_then(|s| s.clone());
                    match s {
                        Some(s) => compose(
                            s.previous.as_ref(),
                            s.current.as_ref(),
                            s.mix(),
                            s.black(),
                            w,
                            h,
                            &mut buf,
                        ),
                        None => compose(None, None, 1.0, 1.0, w, h, &mut buf),
                    }
                    let f = VideoFrame {
                        xres: w as i32,
                        yres: h as i32,
                        fourcc: FOURCC_RGBX,
                        rate_n: (fps * 1000) as i32,
                        rate_d: 1000,
                        aspect: w as f32 / h as f32,
                        format: 1, // progressive
                        timecode: SYNTHESIZE,
                        data: buf.as_ptr(),
                        stride: (w * 4) as i32,
                        metadata: std::ptr::null(),
                        timestamp: 0,
                    };
                    // SAFETY: the frame points into `buf`, alive during the
                    // (synchronous, clocked) call.
                    unsafe { (api.send_video)(snd, &f) };
                }
                // SAFETY: created above, no longer used.
                unsafe { (api.send_destroy)(snd) };
            })
            .map_err(|e| e.to_string())?;
        Ok(Sender {
            scene,
            stop,
            name: name.to_string(),
            size: (w, h),
            fps,
        })
    }

    pub fn set(&self, s: Scene) {
        if let Ok(mut g) = self.scene.lock() {
            *g = Some(s);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn img(v: u8) -> Image {
        Image {
            w: 2,
            h: 1,
            rgba: Arc::new(vec![v, v, v, 255, v, v, v, 255]),
        }
    }

    #[test]
    fn mixes_and_blacks_out() {
        let mut out = Vec::new();
        compose(Some(&img(0)), Some(&img(200)), 0.5, 0.0, 2, 1, &mut out);
        assert_eq!(out[0], 100);
        assert_eq!(out[3], 255);
        compose(Some(&img(0)), Some(&img(200)), 1.0, 0.0, 2, 1, &mut out);
        assert_eq!(out[0], 200);
        compose(None, Some(&img(200)), 1.0, 1.0, 2, 1, &mut out);
        assert_eq!(out[0], 0);
        // A picture of the wrong size counts as black.
        compose(None, Some(&img(200)), 1.0, 0.0, 4, 1, &mut out);
        assert_eq!(out.len(), 16);
        assert_eq!(out[0], 0);
    }
}
