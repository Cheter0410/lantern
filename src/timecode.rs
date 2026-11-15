//! Timecode in: LTC from an audio input, MIDI timecode over the network
//! (RTP-MIDI, advertised with Bonjour so Show Master and Macs find it), or
//! MIDI timecode on a MIDI input of this computer (IAC bus, USB interface).
//! Gives the current position in seconds while the timecode runs.

use crate::show::{TcSettings, TcSource};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use sm_timecode::{LtcDecoder, MtcEvent, MtcReceiver, Timecode};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The latest timecode received.
#[derive(Clone, Copy, Debug, Default)]
struct Latest {
    /// Position (seconds) at `at`.
    secs: f64,
    at: Option<Instant>,
    fps: f64,
    /// Frames keep coming (else: stopped / located).
    running: bool,
    /// Highest frame number seen (finds the frame rate of LTC).
    max_ff: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub secs: f64,
    pub fps: f64,
    pub running: bool,
}

fn tc_secs(tc: Timecode, fps: f64) -> f64 {
    f64::from(tc.hours) * 3600.0
        + f64::from(tc.minutes) * 60.0
        + f64::from(tc.seconds)
        + f64::from(tc.frames) / fps
}

/// A frame stops counting as "running" this long after the last one.
const HOLD: Duration = Duration::from_millis(250);

pub struct Input {
    cfg: TcSettings,
    latest: Arc<Mutex<Latest>>,
    status: Arc<Mutex<String>>,
    _stream: Option<cpal::Stream>,
    rtp: Option<sm_rtpmidi::Session>,
    midi: Option<midir::MidiInputConnection<()>>,
    level: Arc<Mutex<f32>>,
    /// The input could not start: try again from then on (a port may appear).
    retry_at: Option<Instant>,
}

impl Default for Input {
    fn default() -> Self {
        Input {
            cfg: TcSettings::default(),
            latest: Arc::default(),
            status: Arc::new(Mutex::new("Off".into())),
            _stream: None,
            rtp: None,
            midi: None,
            level: Arc::default(),
            retry_at: None,
        }
    }
}

/// Audio inputs by name.
pub fn audio_inputs() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|d| d.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// MIDI inputs of this computer by name (IAC buses, interfaces, virtual ports).
pub fn midi_inputs() -> Vec<String> {
    let Ok(m) = midir::MidiInput::new("Lantern ports") else {
        return Vec::new();
    };
    m.ports()
        .iter()
        .filter_map(|p| m.port_name(p).ok())
        .collect()
}

/// Feeds MIDI messages into the latest reading: quarter frames and full frames.
fn mtc_sink(latest: Arc<Mutex<Latest>>) -> impl FnMut(&[u8]) + Send + 'static {
    let start = Instant::now();
    let mut rx = MtcReceiver::new();
    move |bytes| {
        let now = start.elapsed().as_micros() as u64;
        let Some(ev) = rx.push(bytes, now) else {
            return;
        };
        let (tc, code, at, running) = match ev {
            MtcEvent::Frame {
                timecode,
                rate_code,
                at,
            } => (timecode, rate_code, at, true),
            MtcEvent::FullFrame {
                timecode,
                rate_code,
                at,
            } => (timecode, rate_code, at, false),
        };
        let fps: f64 = match code & 3 {
            0 => 24.0,
            1 => 25.0,
            2 => 29.97,
            _ => 30.0,
        };
        let when = start + Duration::from_micros(at);
        if let Ok(mut l) = latest.lock() {
            l.secs = tc_secs(tc, fps.round());
            l.at = Some(when);
            l.fps = fps;
            l.running = running;
        }
    }
}

impl Input {
    /// Follow the settings (restarts the input when they changed).
    pub fn apply(&mut self, cfg: &TcSettings) {
        let running = self._stream.is_some()
            || self.rtp.is_some()
            || self.midi.is_some()
            || cfg.source == TcSource::Off;
        let waiting = self.retry_at.is_some_and(|t| Instant::now() < t);
        if *cfg == self.cfg && (running || waiting) {
            return;
        }
        if let Some(s) = self.rtp.take() {
            s.close();
        }
        if let Some(c) = self.midi.take() {
            c.close();
        }
        self._stream = None;
        self.cfg = cfg.clone();
        if let Ok(mut l) = self.latest.lock() {
            *l = Latest::default();
        }
        let r = match cfg.source {
            TcSource::Off => Ok("Off".to_string()),
            TcSource::Ltc => self.start_ltc(),
            TcSource::RtpMidi => self.start_rtp(),
            TcSource::Midi => self.start_midi(),
        };
        self.retry_at = r.is_err().then(|| Instant::now() + Duration::from_secs(2));
        self.set_status(r.unwrap_or_else(|e| e));
    }

    fn set_status(&self, s: String) {
        if let Ok(mut g) = self.status.lock() {
            *g = s;
        }
    }

    pub fn status(&self) -> String {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Input level of the LTC channel (0..1), for the meter.
    pub fn level(&self) -> f32 {
        self.level.lock().map(|l| *l).unwrap_or(0.0)
    }

    pub fn reading(&self) -> Option<Reading> {
        let l = *self.latest.lock().ok()?;
        let at = l.at?;
        let running = l.running && at.elapsed() < HOLD;
        let secs = if running {
            l.secs + at.elapsed().as_secs_f64()
        } else {
            l.secs
        };
        Some(Reading {
            secs,
            fps: l.fps,
            running,
        })
    }

    /// The local port of the RTP-MIDI session while it runs (to advertise it).
    pub fn rtp_port(&self) -> Option<u16> {
        self.rtp.as_ref().map(|s| s.local_addr().port())
    }

    pub fn connected(&self) -> Vec<String> {
        self.rtp
            .as_ref()
            .map(|s| {
                s.peers()
                    .into_iter()
                    .filter(|p| p.state == sm_rtpmidi::PeerState::Connected)
                    .map(|p| p.name)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn connect(&self, addr: &str) -> Result<(), String> {
        let a: std::net::SocketAddr = addr
            .parse()
            .map_err(|_| format!("Address {addr} is not valid"))?;
        self.rtp
            .as_ref()
            .ok_or("RTP-MIDI is off")?
            .connect(a)
            .map_err(|e| e.to_string())
    }

    fn start_rtp(&mut self) -> Result<String, String> {
        let mut sc = sm_rtpmidi::SessionConfig::new(self.cfg.rtp_name.clone());
        sc.port = self.cfg.rtp_port;
        let session = sm_rtpmidi::Session::start(sc).map_err(|e| format!("RTP-MIDI: {e}"))?;
        let mut sink = mtc_sink(self.latest.clone());
        session.set_receiver(Box::new(move |bytes| sink(bytes)));
        let port = session.local_addr().port();
        self.rtp = Some(session);
        Ok(format!(
            "RTP-MIDI session \"{}\" (port {port}) is waiting for MIDI timecode",
            self.cfg.rtp_name
        ))
    }

    fn start_midi(&mut self) -> Result<String, String> {
        let mut m = midir::MidiInput::new("Lantern").map_err(|e| format!("MIDI: {e}"))?;
        // Quarter frames and full frames (SysEx) must come through.
        m.ignore(midir::Ignore::ActiveSense);
        let ports = m.ports();
        let want = &self.cfg.midi_port;
        let port = ports
            .iter()
            .find(|p| want.is_empty() || m.port_name(p).ok().as_deref() == Some(want.as_str()))
            .ok_or_else(|| {
                if want.is_empty() {
                    "No MIDI input found (on a Mac: Audio MIDI Setup › MIDI Studio › IAC Driver › \"Device is online\")".to_string()
                } else {
                    format!("MIDI input \"{want}\" not found")
                }
            })?
            .clone();
        let name = m.port_name(&port).unwrap_or_default();
        let mut sink = mtc_sink(self.latest.clone());
        let conn = m
            .connect(&port, "Lantern MTC", move |_, bytes, _| sink(bytes), ())
            .map_err(|e| format!("{name}: {e}"))?;
        self.midi = Some(conn);
        Ok(format!("Waiting for MIDI timecode on \"{name}\""))
    }

    fn start_ltc(&mut self) -> Result<String, String> {
        let host = cpal::default_host();
        let dev = if self.cfg.ltc_device.is_empty() {
            host.default_input_device()
        } else {
            host.input_devices().ok().and_then(|mut d| {
                d.find(|d| d.name().ok().as_deref() == Some(self.cfg.ltc_device.as_str()))
            })
        }
        .ok_or("Audio input not found")?;
        let name = dev.name().unwrap_or_default();
        let config = dev
            .default_input_config()
            .map_err(|e| format!("{name}: {e}"))?;
        let channels = usize::from(config.channels());
        let ch = usize::from(self.cfg.ltc_channel).min(channels.saturating_sub(1));
        let rate = config.sample_rate().0;
        let latest = self.latest.clone();
        let level = self.level.clone();
        let mut decoder = LtcDecoder::new(rate);
        let mut mono: Vec<f32> = Vec::new();
        let mut frames = Vec::new();
        let mut peak_at = Instant::now();
        let mut peak = 0.0f32;
        let mut feed = move |samples: &mut dyn Iterator<Item = f32>| {
            mono.clear();
            mono.extend(samples.skip(ch).step_by(channels.max(1)));
            peak = mono.iter().fold(peak, |p, s| p.max(s.abs()));
            if peak_at.elapsed() > Duration::from_millis(100) {
                if let Ok(mut l) = level.lock() {
                    *l = peak;
                }
                peak = 0.0;
                peak_at = Instant::now();
            }
            frames.clear();
            decoder.push(&mono, &mut frames);
            if let Some(f) = frames.last() {
                if let Ok(mut l) = latest.lock() {
                    l.max_ff = l.max_ff.max(f.timecode.frames);
                    let fps: f64 = match l.max_ff {
                        0..=23 => 24.0,
                        24 => 25.0,
                        _ if f.drop_frame => 29.97,
                        _ => 30.0,
                    };
                    // The frame just decoded ended now: the next one begins.
                    l.secs = tc_secs(f.timecode, fps.round()) + 1.0 / fps;
                    l.at = Some(Instant::now());
                    l.fps = fps;
                    l.running = !f.reverse;
                }
            }
        };
        let err = |e: cpal::StreamError| eprintln!("LTC input: {e}");
        let cfg: cpal::StreamConfig = config.clone().into();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => dev.build_input_stream(
                &cfg,
                move |d: &[f32], _| feed(&mut d.iter().copied()),
                err,
                None,
            ),
            cpal::SampleFormat::I16 => dev.build_input_stream(
                &cfg,
                move |d: &[i16], _| feed(&mut d.iter().map(|s| f32::from(*s) / 32768.0)),
                err,
                None,
            ),
            cpal::SampleFormat::I32 => dev.build_input_stream(
                &cfg,
                move |d: &[i32], _| feed(&mut d.iter().map(|s| *s as f32 / 2_147_483_648.0)),
                err,
                None,
            ),
            cpal::SampleFormat::U16 => dev.build_input_stream(
                &cfg,
                move |d: &[u16], _| {
                    feed(&mut d.iter().map(|s| (f32::from(*s) - 32768.0) / 32768.0))
                },
                err,
                None,
            ),
            f => return Err(format!("{name}: sample format {f} is not supported")),
        }
        .map_err(|e| format!("{name}: {e}"))?;
        stream.play().map_err(|e| format!("{name}: {e}"))?;
        self._stream = Some(stream);
        Ok(format!("LTC on \"{name}\", channel {}", ch + 1))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn mtc_over_rtp_midi_moves_the_reading() {
        let mut input = Input::default();
        let cfg = TcSettings {
            source: TcSource::RtpMidi,
            rtp_name: "Lantern Test".into(),
            rtp_port: 0,
            ..Default::default()
        };
        // Without a network (sandbox) the session may not start: nothing to test.
        input.apply(&cfg);
        let Some(session) = &input.rtp else { return };
        let port = session.local_addr().port();
        let mut sc = sm_rtpmidi::SessionConfig::new("Sender");
        sc.port = 0;
        let sender = sm_rtpmidi::Session::start(sc).unwrap();
        sender
            .connect(format!("127.0.0.1:{port}").parse().unwrap())
            .unwrap();
        let t0 = Instant::now();
        while sender
            .peers()
            .iter()
            .all(|p| p.state != sm_rtpmidi::PeerState::Connected)
        {
            assert!(t0.elapsed() < Duration::from_secs(5), "no connection");
            std::thread::sleep(Duration::from_millis(20));
        }
        // 01:00:05:10 at 25 fps, quarter frames 0..7 (rate code 1).
        let (h, m, s, f) = (1u8, 0u8, 5u8, 10u8);
        let pieces = [
            f & 0xF,
            f >> 4,
            s & 0xF,
            s >> 4,
            m & 0xF,
            m >> 4,
            h & 0xF,
            (h >> 4) | (1 << 1),
        ];
        for (n, v) in pieces.iter().enumerate() {
            sender.send(&[0xF1, ((n as u8) << 4) | v]).unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        let t0 = Instant::now();
        let r = loop {
            if let Some(r) = input.reading() {
                break r;
            }
            assert!(t0.elapsed() < Duration::from_secs(3), "no timecode");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(r.fps, 25.0);
        assert!((r.secs - (3605.0 + 10.0 / 25.0)).abs() < 0.5, "{r:?}");
    }

    /// A virtual MIDI source (like an IAC bus) carrying MTC moves the reading.
    #[cfg(unix)]
    #[test]
    fn mtc_on_a_midi_input_moves_the_reading() {
        use midir::os::unix::VirtualOutput;
        let name = format!("Lantern MTC test {}", std::process::id());
        let Ok(out) = midir::MidiOutput::new("Lantern test") else {
            return; // no MIDI system (e.g. Linux without ALSA sequencer)
        };
        let Ok(mut src) = out.create_virtual(&name) else {
            return;
        };
        let t0 = Instant::now();
        while !midi_inputs().contains(&name) {
            assert!(t0.elapsed() < Duration::from_secs(3), "port not listed");
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut input = Input::default();
        input.apply(&TcSettings {
            source: TcSource::Midi,
            midi_port: name.clone(),
            ..Default::default()
        });
        assert!(input.midi.is_some(), "{}", input.status());
        assert!(input.status().contains(&name), "{}", input.status());

        // Full frame 10:20:30:12 at 25 fps: located, not running.
        src.send(&[
            0xF0,
            0x7F,
            0x7F,
            0x01,
            0x01,
            (1 << 5) | 10,
            20,
            30,
            12,
            0xF7,
        ])
        .unwrap();
        let t0 = Instant::now();
        let r = loop {
            if let Some(r) = input.reading() {
                break r;
            }
            assert!(t0.elapsed() < Duration::from_secs(3), "no timecode");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(!r.running);
        assert!((r.secs - (37230.0 + 12.0 / 25.0)).abs() < 0.05, "{r:?}");

        // Running quarter frames from 01:00:05:10.
        let (h, m, s, f) = (1u8, 0u8, 5u8, 10u8);
        let pieces = [
            f & 0xF,
            f >> 4,
            s & 0xF,
            s >> 4,
            m & 0xF,
            m >> 4,
            h & 0xF,
            (h >> 4) | (1 << 1),
        ];
        for _ in 0..2 {
            for (n, v) in pieces.iter().enumerate() {
                src.send(&[0xF1, ((n as u8) << 4) | v]).unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        std::thread::sleep(Duration::from_millis(30));
        let r = input.reading().unwrap();
        assert!(r.running, "{r:?}");
        assert_eq!(r.fps, 25.0);
        assert!((r.secs - (3605.0 + 10.0 / 25.0)).abs() < 0.5, "{r:?}");

        // A missing port: a clear message, and no busy retrying.
        input.apply(&TcSettings {
            source: TcSource::Midi,
            midi_port: "No such port".into(),
            ..Default::default()
        });
        assert!(input.midi.is_none());
        assert!(input.status().contains("not found"), "{}", input.status());
    }
}
