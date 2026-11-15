//! Output health: what is wrong with the outputs and inputs right now, said
//! plainly – a display unplugged, NDI without a receiver, timecode stopped,
//! the sound device gone. The window shows it in the top bar; new problems
//! are announced.

use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Worth a look (amber).
    Warn,
    /// Something does not go out or come in (red).
    Error,
}

/// Where a problem is fixed (opens that settings tab).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Output,
    Audio,
    Timecode,
    Stage,
    Network,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    /// Stays the same while the problem lasts (to tell new from old).
    pub key: &'static str,
    pub level: Level,
    pub title: String,
    pub detail: String,
    pub area: Area,
}

/// What the timecode input does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tc {
    Off,
    /// Switched on, nothing came yet.
    Never,
    Running,
    /// Came, stands since `for_secs` at `at` (seconds of the day).
    Stopped {
        at: f64,
        fps: f64,
        for_secs: f64,
    },
}

/// What the app knows, for one look.
#[derive(Clone, Debug)]
pub struct Facts {
    /// Output display chosen (index) and how many there are.
    pub display: Option<usize>,
    pub displays: usize,
    /// The output display is the one the Lantern window is on.
    pub display_is_here: bool,
    pub ndi: bool,
    pub ndi_error: Option<String>,
    /// Receivers watching NDI (None: not known).
    pub ndi_receivers: Option<u32>,
    pub tc: Tc,
    /// Slides wait for timecode.
    pub tc_slides: bool,
    pub audio_enabled: bool,
    pub audio_ok: bool,
    pub audio_disconnected: bool,
    pub audio_status: String,
    /// The show has videos or sounds with sound.
    pub sound_in_show: bool,
    /// Stage display chosen.
    pub stage_display: Option<usize>,
    /// The web remote could not start.
    pub web_error: Option<String>,
}

/// Timecode standing longer than this is a problem.
const TC_STOPPED: f64 = 2.0;

pub fn check(f: &Facts) -> Vec<Issue> {
    let mut v = Vec::new();
    let mut add = |key, level, title: String, detail: String, area| {
        v.push(Issue {
            key,
            level,
            title,
            detail,
            area,
        });
    };
    match f.display {
        Some(d) if d >= f.displays => add(
            "display-missing",
            Level::Error,
            format!("Display {} is not connected", d + 1),
            "The slides don't go out there. Plug it in again – Lantern finds it by itself – or \
             choose another display."
                .into(),
            Area::Output,
        ),
        Some(_) if f.display_is_here && f.displays > 1 => add(
            "display-here",
            Level::Warn,
            "The output covers the Lantern window".into(),
            "The output display is the one Lantern itself is on. Move the Lantern window to the \
             other display, or choose the other one as output."
                .into(),
            Area::Output,
        ),
        None if !f.ndi => add(
            "no-output",
            Level::Warn,
            "No output".into(),
            "Neither a display nor NDI is on – the slides go nowhere.".into(),
            Area::Output,
        ),
        _ => {}
    }
    match f.stage_display {
        Some(d) if d >= f.displays => add(
            "stage-missing",
            Level::Warn,
            format!("Stage display {} is not connected", d + 1),
            "The stage display does not show. Plug it in again or choose another display.".into(),
            Area::Stage,
        ),
        Some(d) if f.display == Some(d) => add(
            "stage-on-output",
            Level::Error,
            "Stage display on the output".into(),
            "The stage display and the output are on the same display – the audience would \
             see the notes. Choose another display for one of them."
                .into(),
            Area::Stage,
        ),
        _ => {}
    }
    if let Some(e) = &f.web_error {
        add(
            "web-error",
            Level::Warn,
            "Web remote does not run".into(),
            e.clone(),
            Area::Network,
        );
    }
    if f.ndi {
        if let Some(e) = &f.ndi_error {
            add(
                "ndi-error",
                Level::Error,
                "NDI does not send".into(),
                e.clone(),
                Area::Output,
            );
        } else if f.ndi_receivers == Some(0) {
            add(
                "ndi-nobody",
                Level::Warn,
                "NDI: no receiver".into(),
                "Lantern sends, but nothing on the network is watching (video mixer, NDI \
                 Monitor). Check the source chosen there."
                    .into(),
                Area::Output,
            );
        }
    }
    match f.tc {
        Tc::Never if f.tc_slides => add(
            "tc-never",
            Level::Warn,
            "No timecode yet".into(),
            "Slides wait for timecode, but none has come in. Check the source and the cable or \
             connection."
                .into(),
            Area::Timecode,
        ),
        Tc::Stopped { at, fps, for_secs } if for_secs >= TC_STOPPED => add(
            "tc-stopped",
            if f.tc_slides {
                Level::Error
            } else {
                Level::Warn
            },
            format!("Timecode stopped at {}", crate::show::format_tc(at, fps)),
            format!(
                "Nothing new for {} s.{}",
                for_secs.round(),
                if f.tc_slides {
                    " Slides on timecode wait; GO still works."
                } else {
                    ""
                }
            ),
            Area::Timecode,
        ),
        _ => {}
    }
    if f.audio_enabled && !f.audio_ok {
        let level = if f.sound_in_show {
            Level::Error
        } else {
            Level::Warn
        };
        if f.audio_disconnected {
            add(
                "audio-gone",
                level,
                "Sound device gone".into(),
                format!(
                    "{} It is opened again by itself when it is back.",
                    f.audio_status
                ),
                Area::Audio,
            );
        } else {
            add(
                "audio-error",
                level,
                "No sound out".into(),
                f.audio_status.clone(),
                Area::Audio,
            );
        }
    }
    // Errors first.
    v.sort_by_key(|i| std::cmp::Reverse(i.level));
    v
}

/// Keeps the problems between looks: since when each one is there, and
/// which are new.
#[derive(Default)]
pub struct Monitor {
    pub issues: Vec<Issue>,
    since: HashMap<&'static str, Instant>,
    /// A problem came up (for a moment: the top bar flashes).
    pub new_at: Option<Instant>,
}

impl Monitor {
    /// Take a new look; returns problems that were not there before.
    pub fn update(&mut self, now: Vec<Issue>) -> Vec<Issue> {
        let fresh: Vec<Issue> = now
            .iter()
            .filter(|i| !self.since.contains_key(i.key))
            .cloned()
            .collect();
        self.since.retain(|k, _| now.iter().any(|i| i.key == *k));
        for i in &now {
            self.since.entry(i.key).or_insert_with(Instant::now);
        }
        if !fresh.is_empty() {
            self.new_at = Some(Instant::now());
        }
        self.issues = now;
        fresh
    }

    pub fn since(&self, key: &str) -> Option<Duration> {
        self.since.get(key).map(|t| t.elapsed())
    }

    pub fn worst(&self) -> Option<Level> {
        self.issues.iter().map(|i| i.level).max()
    }

    /// The top bar flashes for a few seconds after a new problem.
    pub fn flashing(&self) -> bool {
        self.new_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(4))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fine() -> Facts {
        Facts {
            display: Some(1),
            displays: 2,
            display_is_here: false,
            ndi: false,
            ndi_error: None,
            ndi_receivers: None,
            tc: Tc::Off,
            tc_slides: false,
            audio_enabled: true,
            audio_ok: true,
            audio_disconnected: false,
            audio_status: String::new(),
            sound_in_show: true,
            stage_display: None,
            web_error: None,
        }
    }

    fn keys(f: &Facts) -> Vec<&'static str> {
        check(f).iter().map(|i| i.key).collect()
    }

    #[test]
    fn all_fine_says_nothing() {
        assert!(check(&fine()).is_empty());
    }

    #[test]
    fn outputs() {
        let f = Facts {
            displays: 1,
            ..fine()
        };
        let i = check(&f);
        assert_eq!(i[0].key, "display-missing");
        assert_eq!(i[0].title, "Display 2 is not connected");
        assert_eq!(i[0].level, Level::Error);
        let f = Facts {
            display_is_here: true,
            ..fine()
        };
        assert_eq!(keys(&f), ["display-here"]);
        // One display only: nothing else to put it on, no warning.
        let f = Facts {
            display: Some(0),
            displays: 1,
            display_is_here: true,
            ..fine()
        };
        assert!(keys(&f).is_empty());
        let f = Facts {
            display: None,
            ..fine()
        };
        assert_eq!(keys(&f), ["no-output"]);
        let f = Facts {
            display: None,
            ndi: true,
            ndi_receivers: Some(0),
            ..fine()
        };
        assert_eq!(keys(&f), ["ndi-nobody"]);
        let f = Facts {
            ndi: true,
            ndi_receivers: Some(2),
            ..fine()
        };
        assert!(keys(&f).is_empty());
        let f = Facts {
            ndi: true,
            ndi_error: Some("NDI runtime not found".into()),
            ..fine()
        };
        assert_eq!(keys(&f), ["ndi-error"]);
    }

    #[test]
    fn stage_display() {
        let f = Facts {
            stage_display: Some(1),
            ..fine()
        };
        assert_eq!(keys(&f), ["stage-on-output"]);
        let f = Facts {
            stage_display: Some(4),
            ..fine()
        };
        assert_eq!(keys(&f), ["stage-missing"]);
        let f = Facts {
            stage_display: Some(0),
            ..fine()
        };
        assert!(keys(&f).is_empty());
    }

    #[test]
    fn timecode() {
        let stopped = |s| Tc::Stopped {
            at: 3600.0,
            fps: 25.0,
            for_secs: s,
        };
        let f = Facts {
            tc: stopped(0.5),
            tc_slides: true,
            ..fine()
        };
        assert!(keys(&f).is_empty(), "a short stand is no problem");
        let f = Facts {
            tc: stopped(5.0),
            tc_slides: true,
            ..fine()
        };
        let i = check(&f);
        assert_eq!(i[0].title, "Timecode stopped at 01:00:00:00");
        assert_eq!(i[0].level, Level::Error, "slides wait for it");
        let f = Facts {
            tc: stopped(5.0),
            ..fine()
        };
        assert_eq!(check(&f)[0].level, Level::Warn);
        let f = Facts {
            tc: Tc::Never,
            ..fine()
        };
        assert!(keys(&f).is_empty(), "nothing waits for it");
        let f = Facts {
            tc: Tc::Never,
            tc_slides: true,
            ..fine()
        };
        assert_eq!(keys(&f), ["tc-never"]);
    }

    #[test]
    fn sound_and_order() {
        let f = Facts {
            audio_ok: false,
            audio_disconnected: true,
            audio_status: "\"MOTU\" is gone.".into(),
            display: None,
            ..fine()
        };
        let i = check(&f);
        assert_eq!(i[0].key, "audio-gone", "errors first");
        assert_eq!(i[1].key, "no-output");
        let f = Facts {
            audio_ok: false,
            sound_in_show: false,
            ..fine()
        };
        assert_eq!(check(&f)[0].level, Level::Warn);
        let f = Facts {
            audio_enabled: false,
            audio_ok: false,
            ..fine()
        };
        assert!(keys(&f).is_empty(), "switched off on purpose");
    }

    #[test]
    fn monitor_tells_new_from_old() {
        let mut m = Monitor::default();
        let gone = Facts {
            displays: 1,
            ..fine()
        };
        assert_eq!(m.update(check(&gone)).len(), 1);
        assert!(m.flashing());
        assert!(
            m.update(check(&gone)).is_empty(),
            "the same again is not new"
        );
        assert_eq!(m.worst(), Some(Level::Error));
        assert!(m.since("display-missing").is_some());
        m.update(check(&fine()));
        assert!(m.issues.is_empty() && m.since("display-missing").is_none());
        assert_eq!(m.update(check(&gone)).len(), 1, "back again: new");
    }
}
