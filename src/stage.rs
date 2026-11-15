//! The stage display's countdown and message: kept while Lantern runs (not
//! in the show file), controlled from the window and over OSC.

use std::time::Instant;

/// A countdown that can be started, paused and set again; past zero it
/// counts the overrun.
#[derive(Clone, Debug)]
pub struct Timer {
    /// Seconds.
    pub length: f64,
    running_since: Option<Instant>,
    /// Seconds run before the last start.
    before: f64,
}

impl Timer {
    pub fn new(minutes: u32) -> Self {
        Timer {
            length: f64::from(minutes) * 60.0,
            running_since: None,
            before: 0.0,
        }
    }

    pub fn running(&self) -> bool {
        self.running_since.is_some()
    }

    /// Never started, or set back.
    pub fn untouched(&self) -> bool {
        !self.running() && self.before == 0.0
    }

    pub fn start(&mut self) {
        if self.running_since.is_none() {
            self.running_since = Some(Instant::now());
        }
    }

    pub fn pause(&mut self) {
        if let Some(t) = self.running_since.take() {
            self.before += t.elapsed().as_secs_f64();
        }
    }

    pub fn toggle(&mut self) {
        if self.running() {
            self.pause();
        } else {
            self.start();
        }
    }

    /// Back to the full length (stopped).
    pub fn reset(&mut self, minutes: u32) {
        *self = Timer::new(minutes);
    }

    pub fn elapsed(&self) -> f64 {
        self.before
            + self
                .running_since
                .map_or(0.0, |t| t.elapsed().as_secs_f64())
    }

    /// Seconds left (negative: over time).
    pub fn remaining(&self) -> f64 {
        self.length - self.elapsed()
    }
}

/// `12:34`, `1:02:03`; over time `+0:12`.
pub fn fmt_countdown(secs: f64) -> String {
    let over = secs < 0.0;
    // Count up while left (ceil), so 0:00 only shows when it is over.
    let s = if over {
        (-secs).floor() as u64
    } else {
        secs.ceil() as u64
    };
    let t = if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    };
    if over {
        format!("+{t}")
    } else {
        t
    }
}

/// A message to the stage, shown until it is cleared.
#[derive(Clone, Debug)]
pub struct Message {
    pub text: String,
    pub at: Instant,
}

impl Message {
    pub fn new(text: &str) -> Option<Message> {
        let text = text.trim();
        (!text.is_empty()).then(|| Message {
            text: text.to_string(),
            at: Instant::now(),
        })
    }

    /// Flashing for the first seconds, so it is seen.
    pub fn flashing(&self) -> bool {
        self.at.elapsed().as_secs_f32() < 4.0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn counts_down_pauses_and_over_runs() {
        let mut t = Timer::new(1);
        assert!(t.untouched());
        assert_eq!(t.remaining(), 60.0);
        t.start();
        std::thread::sleep(Duration::from_millis(60));
        t.pause();
        let left = t.remaining();
        assert!((59.8..59.95).contains(&left), "{left}");
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(t.remaining(), left, "paused stands");
        assert!(!t.untouched());
        t.length = 0.05;
        assert!(t.remaining() < 0.0, "over time");
        t.reset(2);
        assert_eq!(t.remaining(), 120.0);
        assert!(t.untouched());
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_countdown(1800.0), "30:00");
        assert_eq!(fmt_countdown(59.2), "1:00");
        assert_eq!(fmt_countdown(0.4), "0:01");
        assert_eq!(fmt_countdown(0.0), "0:00");
        assert_eq!(fmt_countdown(-12.7), "+0:12");
        assert_eq!(fmt_countdown(3723.0), "1:02:03");
        assert!(Message::new("  ").is_none());
        assert_eq!(Message::new(" 5 min ").unwrap().text, "5 min");
    }
}
