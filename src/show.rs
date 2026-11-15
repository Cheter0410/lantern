//! The show: which presentation, how each slide is triggered, outputs and
//! inputs. Saved as a `.lantern` file (JSON) next to the presentation.

use crate::pptx::{SlideInfo, Transition};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Trigger {
    /// With the GO button (or a clicker, OSC, Show Master).
    #[default]
    Go,
    /// When the incoming timecode reaches the slide's keyframe.
    Timecode,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub enum TransitionCfg {
    /// As in the presentation.
    #[default]
    FromFile,
    Cut,
    Fade(f32),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct SlideCfg {
    /// Title when it was set up (finds the slide again after edits).
    pub title: String,
    pub trigger: Trigger,
    /// Keyframe `HH:MM:SS:FF`.
    pub tc: String,
    pub transition: TransitionCfg,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Resolution {
    /// The display's own resolution (1920 x 1080 for NDI only).
    #[default]
    Auto,
    Hd720,
    Hd1080,
    Uhd2160,
}

impl Resolution {
    pub const ALL: [Resolution; 4] = [
        Resolution::Auto,
        Resolution::Hd720,
        Resolution::Hd1080,
        Resolution::Uhd2160,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Resolution::Auto => "Same as the display",
            Resolution::Hd720 => "1280 × 720",
            Resolution::Hd1080 => "1920 × 1080",
            Resolution::Uhd2160 => "3840 × 2160",
        }
    }
    pub fn size(self) -> Option<(u32, u32)> {
        match self {
            Resolution::Auto => None,
            Resolution::Hd720 => Some((1280, 720)),
            Resolution::Hd1080 => Some((1920, 1080)),
            Resolution::Uhd2160 => Some((3840, 2160)),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Output {
    /// Display number (from 0) for the full-screen output.
    pub display: Option<usize>,
    pub ndi: bool,
    pub ndi_name: String,
    pub resolution: Resolution,
    pub fps: u32,
}

impl Default for Output {
    fn default() -> Self {
        Output {
            display: None,
            ndi: false,
            ndi_name: "Lantern".into(),
            resolution: Resolution::Auto,
            fps: 30,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TcSource {
    #[default]
    Off,
    /// LTC on an audio input.
    Ltc,
    /// MIDI timecode over the network (RTP-MIDI / AppleMIDI).
    RtpMidi,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct TcSettings {
    pub source: TcSource,
    /// Audio input device (empty = default).
    pub ltc_device: String,
    /// Channel of the device carrying LTC (from 0).
    pub ltc_channel: u16,
    /// Name of Lantern's RTP-MIDI session on the network.
    pub rtp_name: String,
    pub rtp_port: u16,
}

impl Default for TcSettings {
    fn default() -> Self {
        TcSettings {
            source: TcSource::Off,
            ltc_device: String::new(),
            ltc_channel: 0,
            rtp_name: "Lantern".into(),
            rtp_port: 5008,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Show {
    /// The presentation (relative to the show file once saved).
    pub presentation: PathBuf,
    pub slides: Vec<SlideCfg>,
    pub output: Output,
    pub timecode: TcSettings,
    /// UDP port for OSC remote control (Show Master, QLab, Companion…).
    pub osc_port: u16,
}

impl Default for Show {
    fn default() -> Self {
        Show {
            presentation: PathBuf::new(),
            slides: Vec::new(),
            output: Output::default(),
            timecode: TcSettings::default(),
            osc_port: 53_100,
        }
    }
}

impl Show {
    pub fn load(file: &Path) -> Result<Show, String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut s: Show =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        if s.presentation.is_relative() {
            if let Some(dir) = file.parent() {
                s.presentation = dir.join(&s.presentation);
            }
        }
        Ok(s)
    }

    pub fn save(&self, file: &Path) -> Result<(), String> {
        let mut s = self.clone();
        if let (Some(dir), Ok(rel)) = (
            file.parent(),
            self.presentation
                .strip_prefix(file.parent().unwrap_or(Path::new(""))),
        ) {
            if !dir.as_os_str().is_empty() {
                s.presentation = rel.to_path_buf();
            }
        }
        let text = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
        std::fs::write(file, text).map_err(|e| format!("{}: {e}", file.display()))
    }

    /// Line the slide settings up with the slides of the (possibly edited)
    /// presentation: same position and title first, else the same title.
    pub fn match_slides(&mut self, slides: &[SlideInfo]) {
        let old = std::mem::take(&mut self.slides);
        let mut used = vec![false; old.len()];
        for (i, s) in slides.iter().enumerate() {
            let pick = if old
                .get(i)
                .is_some_and(|o| o.title == s.title || o.title.is_empty())
            {
                Some(i)
            } else {
                old.iter()
                    .enumerate()
                    .position(|(j, o)| !used[j] && !s.title.is_empty() && o.title == s.title)
            };
            let mut cfg = match pick {
                Some(j) if !used[j] => {
                    used[j] = true;
                    old[j].clone()
                }
                _ => SlideCfg::default(),
            };
            cfg.title = s.title.clone();
            self.slides.push(cfg);
        }
    }
}

/// `HH:MM:SS:FF` (also `HH:MM:SS.FF`, `MM:SS:FF`) to seconds.
pub fn parse_tc(s: &str, fps: f64) -> Option<f64> {
    let parts: Vec<f64> = s
        .trim()
        .split([':', '.', ';'])
        .map(|p| p.trim().parse::<f64>().ok())
        .collect::<Option<_>>()?;
    let (h, m, sec, f) = match parts.as_slice() {
        [h, m, s, f] => (*h, *m, *s, *f),
        [m, s, f] => (0.0, *m, *s, *f),
        _ => return None,
    };
    if m >= 60.0 || sec >= 60.0 || f >= fps.ceil() {
        return None;
    }
    Some(h * 3600.0 + m * 60.0 + sec + f / fps)
}

pub fn format_tc(secs: f64, fps: f64) -> String {
    let fps_n = fps.round().max(1.0);
    let total = (secs.max(0.0) * fps + 1e-6).floor() as u64;
    let f = total % fps_n as u64;
    let s = total / fps_n as u64;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        s / 3600,
        (s / 60) % 60,
        s % 60,
        f
    )
}

/// Follows the timecode: which slide its keyframes call for.
#[derive(Default, Debug)]
pub struct Chase {
    last: Option<f64>,
}

impl Chase {
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// `keys`: (slide, seconds), any order. Returns the slide to show: the
    /// latest keyframe passed since the last call, or after a jump back (and
    /// on the first reading) the latest keyframe before the timecode.
    /// Slides shown with GO in between stay until the next keyframe.
    pub fn update(&mut self, now: f64, keys: &[(usize, f64)]) -> Option<usize> {
        let prev = self.last.replace(now);
        let latest_before = |t: f64| {
            keys.iter()
                .filter(|(_, k)| *k <= t + 1e-6)
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(s, _)| *s)
        };
        match prev {
            // Running (or jumping ahead): the latest keyframe passed.
            Some(p) if now >= p => keys
                .iter()
                .filter(|(_, k)| *k > p + 1e-6 && *k <= now + 1e-6)
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(s, _)| *s),
            // First reading or a jump back: where the timecode is now.
            _ => latest_before(now),
        }
    }
}

/// What is on the output: the slide, the one before during a transition,
/// and the blackout.
#[derive(Debug)]
pub struct Player {
    pub current: Option<usize>,
    pub previous: Option<usize>,
    pub since: Instant,
    pub fade: f32,
    pub blackout: bool,
    black_since: Instant,
    black_from: f32,
}

/// Seconds a blackout takes to fade.
pub const BLACKOUT_FADE: f32 = 0.4;

impl Default for Player {
    fn default() -> Self {
        Player {
            current: None,
            previous: None,
            since: Instant::now(),
            fade: 0.0,
            blackout: false,
            black_since: Instant::now(),
            black_from: 0.0,
        }
    }
}

impl Player {
    pub fn show(&mut self, slide: Option<usize>, t: Transition) {
        if slide == self.current {
            return;
        }
        self.previous = self.current;
        self.current = slide;
        self.since = Instant::now();
        self.fade = match t {
            Transition::Cut => 0.0,
            Transition::Fade(s) => s,
        };
    }

    pub fn set_blackout(&mut self, on: bool) {
        if on != self.blackout {
            self.black_from = self.black_level();
            self.blackout = on;
            self.black_since = Instant::now();
        }
    }

    /// 0 = transition starts, 1 = done.
    pub fn mix(&self) -> f32 {
        if self.fade <= 0.0 {
            1.0
        } else {
            (self.since.elapsed().as_secs_f32() / self.fade).clamp(0.0, 1.0)
        }
    }

    /// 0 = picture, 1 = black.
    pub fn black_level(&self) -> f32 {
        let t = (self.black_since.elapsed().as_secs_f32() / BLACKOUT_FADE).clamp(0.0, 1.0);
        let target = if self.blackout { 1.0 } else { 0.0 };
        self.black_from + (target - self.black_from) * t
    }

    pub fn animating(&self) -> bool {
        self.mix() < 1.0 || {
            let l = self.black_level();
            l > 0.0 && l < 1.0
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn timecode_text() {
        assert_eq!(parse_tc("01:00:05:12", 25.0), Some(3605.0 + 12.0 / 25.0));
        assert_eq!(parse_tc("00:01:00:00", 30.0), Some(60.0));
        assert_eq!(parse_tc("1:2:3", 25.0), Some(62.0 + 3.0 / 25.0));
        assert_eq!(parse_tc("00:61:00:00", 25.0), None);
        assert_eq!(parse_tc("00:00:00:25", 25.0), None);
        assert_eq!(parse_tc("x", 25.0), None);
        assert_eq!(format_tc(3605.48, 25.0), "01:00:05:12");
        assert_eq!(
            parse_tc(&format_tc(12.0 + 7.0 / 30.0, 30.0), 30.0),
            Some(12.0 + 7.0 / 30.0)
        );
    }

    #[test]
    fn chase_fires_keyframes_and_locates_after_jumps() {
        let keys = [(0, 10.0), (2, 20.0), (5, 30.0)];
        let mut c = Chase::default();
        // First timecode: the latest keyframe before it.
        assert_eq!(c.update(15.0, &keys), Some(0));
        assert_eq!(c.update(15.5, &keys), None);
        assert_eq!(c.update(19.98, &keys), None);
        assert_eq!(c.update(20.02, &keys), Some(2));
        assert_eq!(c.update(20.06, &keys), None);
        // Jump back: locate.
        assert_eq!(c.update(12.0, &keys), Some(0));
        // Jump forward past two keyframes.
        assert_eq!(c.update(40.0, &keys), Some(5));
        // Before the first keyframe: nothing.
        c.reset();
        assert_eq!(c.update(5.0, &keys), None);
    }

    #[test]
    fn slide_settings_follow_edited_presentations() {
        let info = |t: &str| SlideInfo {
            number: 1,
            title: t.into(),
            notes: String::new(),
            hidden: false,
            transition: Transition::Cut,
        };
        let mut show = Show::default();
        show.match_slides(&[info("A"), info("B")]);
        show.slides[1].trigger = Trigger::Timecode;
        show.slides[1].tc = "00:00:10:00".into();
        // A slide was inserted in front of B.
        show.match_slides(&[info("A"), info("New"), info("B")]);
        assert_eq!(show.slides[2].trigger, Trigger::Timecode);
        assert_eq!(show.slides[1].trigger, Trigger::Go);
    }

    #[test]
    fn show_file_keeps_the_presentation_relative() {
        let dir = std::env::temp_dir().join(format!("lantern-show-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Show {
            presentation: dir.join("talk.pptx"),
            ..Default::default()
        };
        s.output.ndi = true;
        let f = dir.join("talk.lantern");
        s.save(&f).unwrap();
        assert!(std::fs::read_to_string(&f)
            .unwrap()
            .contains("\"talk.pptx\""));
        assert_eq!(Show::load(&f).unwrap(), s);
        let _ = std::fs::remove_dir_all(dir);
    }
}
