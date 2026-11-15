//! The web remote: a small HTTP server for a phone or tablet in the same
//! network. It serves one page (GO+, GO−, Blackout, the slide now and next,
//! the countdown and stage messages), the show's state as JSON and the two
//! pictures as JPEG; buttons come back as commands, like OSC.
//!
//! Plain HTTP/1.1 on std (one short thread per request, `Connection: close`):
//! a few phones polling a few times a second is all it has to carry. An
//! optional PIN keeps others in the network out.

use crate::deck::Image;
use crate::osc::{Command, StageCmd, VideoCmd};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PAGE: &str = include_str!("web_remote.html");

/// Clients seen within this count as connected.
const SEEN: Duration = Duration::from_secs(5);

/// A picture for the page: changes `id` when it changes.
struct Pic {
    id: u64,
    image: Image,
    jpeg: Option<Arc<Vec<u8>>>,
}

#[derive(Default)]
struct Shared {
    /// The state as JSON (made by the app).
    state: String,
    pics: [Option<Pic>; 2],
    pin: String,
    clients: HashMap<IpAddr, Instant>,
}

pub struct Server {
    pub port: u16,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Which picture.
#[derive(Clone, Copy)]
pub enum Which {
    Now = 0,
    Next = 1,
}

impl Server {
    /// Listen on `port` (0: any free one) on all addresses.
    pub fn start(
        port: u16,
        tx: Sender<Command>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Server, String> {
        let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                format!("Port {port} is taken by another program – choose another one.")
            } else {
                format!("Port {port}: {e}")
            }
        })?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let shared: Arc<Mutex<Shared>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (sh, st) = (shared.clone(), stop.clone());
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        std::thread::Builder::new()
            .name("lantern-web".into())
            .spawn(move || {
                while !st.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            let (sh, tx, wake) = (sh.clone(), tx.clone(), wake.clone());
                            let _ = std::thread::Builder::new()
                                .name("lantern-web-request".into())
                                .spawn(move || {
                                    let _ = serve(stream, peer, &sh, &tx, &*wake);
                                });
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(15)),
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Server { port, shared, stop })
    }

    pub fn set_state(&self, json: String) {
        if let Ok(mut s) = self.shared.lock() {
            s.state = json;
        }
    }

    pub fn set_pin(&self, pin: &str) {
        if let Ok(mut s) = self.shared.lock() {
            if s.pin != pin {
                s.pin = pin.to_string();
            }
        }
    }

    /// Hand over a picture (`id` says when it is another one).
    pub fn set_pic(&self, which: Which, id: u64, image: Option<&Image>) {
        if let Ok(mut s) = self.shared.lock() {
            let slot = &mut s.pics[which as usize];
            match image {
                None => *slot = None,
                Some(img) if slot.as_ref().is_none_or(|p| p.id != id) => {
                    *slot = Some(Pic {
                        id,
                        image: img.clone(),
                        jpeg: None,
                    });
                }
                Some(_) => {}
            }
        }
    }

    /// Devices that asked within the last seconds.
    pub fn clients(&self) -> usize {
        self.shared
            .lock()
            .map(|s| s.clients.values().filter(|t| t.elapsed() < SEEN).count())
            .unwrap_or(0)
    }
}

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: String,
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 32 * 1024 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        .min(64 * 1024);
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    let (path, q) = target.split_once('?').unwrap_or((&target, ""));
    let query = q
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), unescape(v)))
        .collect();
    Some(Request {
        method,
        path: path.to_string(),
        query,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

/// `%20` and `+` in a query value.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(v) => {
                    out.push(v);
                    i += 2;
                }
                Err(_) => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// What a button on the page does.
pub fn command_for(action: &str, body: &str) -> Option<Command> {
    Some(match action {
        "go" => Command::Go,
        "back" => Command::Back,
        "blackout" => Command::Blackout(None),
        "reset" => Command::Reset,
        "video" => Command::Video(VideoCmd::Toggle),
        "timer" => Command::Stage(StageCmd::TimerToggle),
        "timer-reset" => Command::Stage(StageCmd::TimerReset(None)),
        "message" => {
            if body.trim().is_empty() {
                Command::Stage(StageCmd::ClearMessage)
            } else {
                Command::Stage(StageCmd::Message(body.chars().take(300).collect()))
            }
        }
        "clear-message" => Command::Stage(StageCmd::ClearMessage),
        _ => return None,
    })
}

fn serve(
    mut stream: TcpStream,
    peer: SocketAddr,
    shared: &Mutex<Shared>,
    tx: &Sender<Command>,
    wake: &(dyn Fn() + Send + Sync),
) -> std::io::Result<()> {
    let _ = stream.set_nonblocking(false);
    let Some(req) = read_request(&mut stream) else {
        return Ok(());
    };
    if req.path == "/" || req.path == "/index.html" {
        return respond(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes(),
        );
    }
    let pin_ok = {
        let Ok(mut s) = shared.lock() else {
            return Ok(());
        };
        let given = req
            .headers
            .get("x-pin")
            .filter(|p| !p.is_empty())
            .or_else(|| req.query.get("pin"))
            .map(String::as_str)
            .unwrap_or("");
        let ok = s.pin.is_empty() || given == s.pin;
        if ok {
            s.clients.insert(peer.ip(), Instant::now());
            s.clients.retain(|_, t| t.elapsed() < SEEN * 4);
        }
        ok
    };
    if !pin_ok {
        return respond(&mut stream, "403 Forbidden", "text/plain", b"PIN");
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/api/state") => {
            let state = shared.lock().map(|s| s.state.clone()).unwrap_or_default();
            respond(&mut stream, "200 OK", "application/json", state.as_bytes())
        }
        ("GET", p) if p == "/img/now" || p == "/img/next" => {
            let which = if p == "/img/now" { 0 } else { 1 };
            // Encode once per picture (outside the lock: it takes a moment).
            let todo = shared.lock().ok().and_then(|s| {
                s.pics[which].as_ref().map(|p| match &p.jpeg {
                    Some(j) => Ok(j.clone()),
                    None => Err((p.id, p.image.clone())),
                })
            });
            let jpeg = match todo {
                None => return respond(&mut stream, "404 Not Found", "text/plain", b"none"),
                Some(Ok(j)) => j,
                Some(Err((id, img))) => {
                    let j = Arc::new(jpeg(&img).unwrap_or_default());
                    if let Ok(mut s) = shared.lock() {
                        if let Some(p) = s.pics[which].as_mut().filter(|p| p.id == id) {
                            p.jpeg = Some(j.clone());
                        }
                    }
                    j
                }
            };
            respond(&mut stream, "200 OK", "image/jpeg", &jpeg)
        }
        ("POST", p) => match p
            .strip_prefix("/api/")
            .and_then(|a| command_for(a, &req.body))
        {
            Some(c) => {
                let _ = tx.send(c);
                wake();
                respond(&mut stream, "200 OK", "text/plain", b"ok")
            }
            None => respond(&mut stream, "404 Not Found", "text/plain", b"unknown"),
        },
        _ => respond(&mut stream, "404 Not Found", "text/plain", b"not found"),
    }
}

/// A JPEG of at most 960 pixels across.
fn jpeg(img: &Image) -> Option<Vec<u8>> {
    let rgba = image::RgbaImage::from_raw(img.w, img.h, img.rgba.to_vec())?;
    let scale = (960.0 / img.w.max(1) as f32).min(1.0);
    let (w, h) = (
        ((img.w as f32 * scale) as u32).max(1),
        ((img.h as f32 * scale) as u32).max(1),
    );
    let small = if scale < 1.0 {
        image::imageops::thumbnail(&rgba, w, h)
    } else {
        rgba
    };
    let rgb = image::DynamicImage::ImageRgba8(small).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80)
        .encode_image(&rgb)
        .ok()?;
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn http(port: u16, method: &str, path: &str, pin: &str, body: &str) -> (u16, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: x\r\nX-Pin: {pin}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).unwrap();
        let end = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let head = String::from_utf8_lossy(&out[..end]).to_string();
        let code = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        (code, out[end..].to_vec())
    }

    #[test]
    fn serves_the_page_state_pictures_and_commands() {
        let (tx, rx) = channel();
        let srv = Server::start(0, tx, || {}).unwrap();
        let port = srv.port;
        let (code, page) = http(port, "GET", "/", "", "");
        assert_eq!(code, 200);
        assert!(String::from_utf8_lossy(&page).contains("Lantern"));

        srv.set_state(r#"{"slide":1}"#.into());
        assert_eq!(http(port, "GET", "/api/state", "", "").1, br#"{"slide":1}"#);
        assert_eq!(srv.clients(), 1, "the phone counts");

        assert_eq!(
            http(port, "GET", "/img/now", "", "").0,
            404,
            "no picture yet"
        );
        let img = Image {
            w: 4,
            h: 2,
            rgba: Arc::new(vec![200; 32]),
        };
        srv.set_pic(Which::Now, 1, Some(&img));
        let (code, jpg) = http(port, "GET", "/img/now?v=1", "", "");
        assert_eq!(code, 200);
        assert_eq!(&jpg[..2], &[0xFF, 0xD8], "a JPEG");

        assert_eq!(http(port, "POST", "/api/go", "", "").0, 200);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Command::Go
        );
        http(port, "POST", "/api/message", "", "Wrap up, please");
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Command::Stage(StageCmd::Message("Wrap up, please".into()))
        );
        assert_eq!(http(port, "POST", "/api/explode", "", "").0, 404);

        // With a PIN: nothing without it, everything with it.
        srv.set_pin("4711");
        assert_eq!(
            http(port, "GET", "/", "", "").0,
            200,
            "the page asks for it"
        );
        assert_eq!(http(port, "GET", "/api/state", "", "").0, 403);
        assert_eq!(http(port, "POST", "/api/go", "1234", "").0, 403);
        assert!(rx.try_recv().is_err(), "no GO with a wrong PIN");
        assert_eq!(http(port, "POST", "/api/blackout", "4711", "").0, 200);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Command::Blackout(None)
        );
        assert_eq!(http(port, "GET", "/img/now?pin=4711", "", "").0, 200);
    }

    #[test]
    fn a_taken_port_is_said() {
        let (tx, _rx) = channel();
        let a = Server::start(0, tx.clone(), || {}).unwrap();
        let e = Server::start(a.port, tx, || {}).err().unwrap();
        assert!(e.contains("taken"), "{e}");
    }

    #[test]
    fn query_values() {
        assert_eq!(unescape("a%20b+c"), "a b c");
        assert_eq!(unescape("100%"), "100%");
    }
}
