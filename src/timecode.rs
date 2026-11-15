//! Timecode in: LTC from an audio input, or MIDI timecode over the network
//! (RTP-MIDI, advertised with Bonjour so Show Master and Macs find it).
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
    level: Arc<Mutex<f32>>,
}

impl Default for Input {
    fn default() -> Self {
        Input {
            cfg: TcSettings::default(),
            latest: Arc::default(),
            status: Arc::new(Mutex::new("Off".into())),
            _stream: None,
            rtp: None,
            level: Arc::default(),
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

impl Input {
    /// Follow the settings (restarts the input when they changed).
    pub fn apply(&mut self, cfg: &TcSettings) {
        if *cfg == self.cfg
            && (self._stream.is_some() || self.rtp.is_some() || cfg.source == TcSource::Off)
        {
            return;
        }
        if let Some(s) = self.rtp.take() {
            s.close();
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
        };
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
        let latest = self.latest.clone();
        let start = Instant::now();
        let mut rx = MtcReceiver::new();
        session.set_receiver(Box::new(move |bytes| {
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
        }));
        let port = session.local_addr().port();
        self.rtp = Some(session);
        Ok(format!(
            "RTP-MIDI session \"{}\" (port {port}) is waiting for MIDI timecode",
            self.cfg.rtp_name
        ))
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
}
