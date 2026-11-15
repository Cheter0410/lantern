//! OSC remote control over UDP (Show Master, QLab, Bitfocus Companion…).
//!
//! `/lantern/go` (or `/lantern/next`), `/lantern/back`, `/lantern/slide <n>`
//! (from 1), `/lantern/first`, `/lantern/blackout [0|1]` (no argument =
//! toggle), `/lantern/clear` (back to black before the first slide).
//! The live slide's video or sound: `/lantern/video/play`, `/video/pause`,
//! `/video/toggle`, `/video/restart`, `/video/seek <seconds>`,
//! `/video/skip <seconds, negative = back>`.
//! The `/lantern` prefix may be left out.

use rosc::{OscPacket, OscType};
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Go,
    Back,
    Slide(usize),
    First,
    Blackout(Option<bool>),
    Clear,
    Video(VideoCmd),
}

/// Transport of the live slide's video (or sound).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VideoCmd {
    Play,
    Pause,
    Toggle,
    Restart,
    /// To this many seconds.
    Seek(f64),
    /// By this many seconds (negative: back).
    Skip(f64),
}

fn number(args: &[OscType]) -> Option<f64> {
    args.first().and_then(|a| match a {
        OscType::Int(i) => Some(f64::from(*i)),
        OscType::Float(f) => Some(f64::from(*f)),
        OscType::Double(d) => Some(*d),
        OscType::Long(l) => Some(*l as f64),
        OscType::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        OscType::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

pub fn parse(addr: &str, args: &[OscType]) -> Option<Command> {
    let a = addr.trim_end_matches('/').to_ascii_lowercase();
    let a = a.strip_prefix("/lantern").unwrap_or(&a);
    match a {
        "/go" | "/next" => Some(Command::Go),
        "/back" | "/prev" | "/previous" => Some(Command::Back),
        "/first" => Some(Command::First),
        "/clear" => Some(Command::Clear),
        "/slide" => {
            let n = number(args)?;
            (n >= 1.0).then(|| Command::Slide(n as usize - 1))
        }
        "/blackout" => Some(Command::Blackout(number(args).map(|v| v >= 0.5))),
        "/video/play" | "/video/resume" => Some(Command::Video(VideoCmd::Play)),
        "/video/pause" => Some(Command::Video(VideoCmd::Pause)),
        "/video" | "/video/toggle" => Some(Command::Video(VideoCmd::Toggle)),
        "/video/restart" => Some(Command::Video(VideoCmd::Restart)),
        "/video/seek" => Some(Command::Video(VideoCmd::Seek(number(args)?.max(0.0)))),
        "/video/skip" => Some(Command::Video(VideoCmd::Skip(number(args)?))),
        _ => {
            // `/lantern/slide/3`
            let n = a.strip_prefix("/slide/")?.parse::<usize>().ok()?;
            (n >= 1).then(|| Command::Slide(n - 1))
        }
    }
}

fn flatten(p: OscPacket, out: &mut Vec<rosc::OscMessage>) {
    match p {
        OscPacket::Message(m) => out.push(m),
        OscPacket::Bundle(b) => {
            for p in b.content {
                flatten(p, out);
            }
        }
    }
}

pub struct Listener {
    pub port: u16,
    stop: Arc<AtomicBool>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Listener {
    pub fn start(
        port: u16,
        tx: Sender<Command>,
        wake: impl Fn() + Send + 'static,
    ) -> Result<Listener, String> {
        let sock =
            UdpSocket::bind(("0.0.0.0", port)).map_err(|e| format!("OSC-Port {port}: {e}"))?;
        sock.set_read_timeout(Some(Duration::from_millis(250)))
            .map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        std::thread::Builder::new()
            .name("lantern-osc".into())
            .spawn(move || {
                let mut buf = vec![0u8; 65_536];
                while !stop2.load(Ordering::Relaxed) {
                    let Ok((n, _)) = sock.recv_from(&mut buf) else {
                        continue;
                    };
                    let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..n]) else {
                        continue;
                    };
                    let mut msgs = Vec::new();
                    flatten(packet, &mut msgs);
                    for m in msgs {
                        if let Some(c) = parse(&m.addr, &m.args) {
                            let _ = tx.send(c);
                            wake();
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Listener { port, stop })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(parse("/lantern/go", &[]), Some(Command::Go));
        assert_eq!(
            parse("/lantern/video/pause", &[]),
            Some(Command::Video(VideoCmd::Pause))
        );
        assert_eq!(
            parse("/video/seek", &[OscType::Float(12.5)]),
            Some(Command::Video(VideoCmd::Seek(12.5)))
        );
        assert_eq!(
            parse("/lantern/video/skip", &[OscType::Int(-10)]),
            Some(Command::Video(VideoCmd::Skip(-10.0)))
        );
        assert_eq!(parse("/lantern/video/seek", &[]), None);
        assert_eq!(parse("/next", &[]), Some(Command::Go));
        assert_eq!(
            parse("/lantern/slide", &[OscType::Int(3)]),
            Some(Command::Slide(2))
        );
        assert_eq!(
            parse("/lantern/slide", &[OscType::String("2".into())]),
            Some(Command::Slide(1))
        );
        assert_eq!(parse("/lantern/slide/5", &[]), Some(Command::Slide(4)));
        assert_eq!(parse("/lantern/slide", &[OscType::Int(0)]), None);
        assert_eq!(
            parse("/lantern/blackout", &[]),
            Some(Command::Blackout(None))
        );
        assert_eq!(
            parse("/lantern/blackout", &[OscType::Float(1.0)]),
            Some(Command::Blackout(Some(true)))
        );
        assert_eq!(parse("/other", &[]), None);
    }

    #[test]
    fn receives_over_udp() {
        let (tx, rx) = std::sync::mpsc::channel();
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = sock.local_addr().unwrap().port();
        drop(sock);
        let _l = Listener::start(port, tx, || {}).unwrap();
        let msg = rosc::encoder::encode(&OscPacket::Message(rosc::OscMessage {
            addr: "/lantern/slide".into(),
            args: vec![OscType::Int(4)],
        }))
        .unwrap();
        UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .send_to(&msg, ("127.0.0.1", port))
            .unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Command::Slide(3)
        );
    }
}
