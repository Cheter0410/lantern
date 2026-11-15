//! The show log: what happened during the show, with the time of day and the
//! timecode – each slide and who sent it, blackout, videos, stage messages,
//! the countdown, problems and when they went away, and notes typed in by the
//! operator. It is written as it happens to a text file per day in the show
//! folder (`Logs/2026-11-15.txt`), so it is there for the report after the show
//! even if the computer goes down.

use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// What kind of entry (for the colour and the filter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The show was opened, Live switched on or off.
    Show,
    /// A slide went out.
    Cue,
    Blackout,
    Video,
    /// Stage message, countdown.
    Stage,
    /// A problem came up.
    Problem,
    /// A problem went away.
    Fixed,
    /// Typed in by the operator.
    Note,
}

impl Kind {
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Show => "SHOW",
            Kind::Cue => "CUE",
            Kind::Blackout => "BLACK",
            Kind::Video => "VIDEO",
            Kind::Stage => "STAGE",
            Kind::Problem => "PROBLEM",
            Kind::Fixed => "FIXED",
            Kind::Note => "NOTE",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Time of day, `HH:MM:SS`.
    pub at: String,
    /// Timecode at that moment, when it was running.
    pub tc: Option<String>,
    pub kind: Kind,
    pub text: String,
}

impl Entry {
    /// One line of the file: `19:30:12  01:00:04:12  CUE      Slide 4 …`.
    pub fn line(&self) -> String {
        format!(
            "{}  {:<11}  {:<7}  {}",
            self.at,
            self.tc.as_deref().unwrap_or("-"),
            self.kind.tag(),
            self.text.replace(['\n', '\r'], " ")
        )
    }
}

/// Entries kept in memory (the file has them all).
const KEEP: usize = 2000;

#[derive(Default)]
pub struct Log {
    pub entries: VecDeque<Entry>,
    /// The file written to (none: the show has no folder yet).
    file: Option<PathBuf>,
    /// Writing failed (said once in the panel).
    pub error: Option<String>,
}

/// `<show folder>/Logs/<date>.txt`.
pub fn file_for(show_file: &Path, date: &str) -> Option<PathBuf> {
    Some(show_file.parent()?.join("Logs").join(format!("{date}.txt")))
}

impl Log {
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Write to this file from now on (a new show was opened). The entries in
    /// memory start afresh.
    pub fn set_file(&mut self, file: Option<PathBuf>) {
        self.entries.clear();
        self.error = None;
        self.file = file;
    }

    pub fn push(&mut self, at: String, tc: Option<String>, kind: Kind, text: impl Into<String>) {
        let e = Entry {
            at,
            tc,
            kind,
            text: text.into(),
        };
        if let Some(f) = &self.file {
            if let Err(err) = append(f, &e.line()) {
                self.error = Some(format!("Can't write {}: {err}", f.display()));
            } else {
                self.error = None;
            }
        }
        self.entries.push_back(e);
        while self.entries.len() > KEEP {
            self.entries.pop_front();
        }
    }

    /// All entries as text (to copy).
    pub fn text(&self) -> String {
        self.entries
            .iter()
            .map(Entry::line)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn count(&self, kind: Kind) -> usize {
        self.entries.iter().filter(|e| e.kind == kind).count()
    }
}

fn append(file: &Path, line: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let new = !file.exists();
    let mut f = OpenOptions::new().create(true).append(true).open(file)?;
    if new {
        writeln!(f, "Lantern show log\n")?;
        writeln!(f, "{:<8}  {:<11}  {:<7}  WHAT", "TIME", "TIMECODE", "")?;
    }
    writeln!(f, "{line}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "lantern-log-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn lines_line_up() {
        let e = Entry {
            at: "19:30:12".into(),
            tc: Some("01:00:04:12".into()),
            kind: Kind::Cue,
            text: "Slide 4 · Intro\nsecond".into(),
        };
        assert_eq!(
            e.line(),
            "19:30:12  01:00:04:12  CUE      Slide 4 · Intro second"
        );
        let e = Entry { tc: None, ..e };
        assert!(e.line().starts_with("19:30:12  -            CUE"));
    }

    #[test]
    fn writes_to_the_show_folder_and_keeps_appending() {
        let dir = temp();
        let file = file_for(&dir.join("Show.lantern"), "2026-11-15").unwrap();
        assert_eq!(file, dir.join("Logs").join("2026-11-15.txt"));
        let mut log = Log::default();
        log.set_file(Some(file.clone()));
        log.push("19:30:00".into(), None, Kind::Show, "Show opened");
        log.push("19:31:00".into(), None, Kind::Cue, "Slide 1");
        // Opened again the same day: added below.
        let mut log = Log::default();
        log.set_file(Some(file.clone()));
        log.push("21:00:00".into(), None, Kind::Note, "Mic 3 dropped out");
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.matches("Lantern show log").count(), 1);
        assert!(text.contains("CUE      Slide 1"));
        assert!(text.trim_end().ends_with("NOTE     Mic 3 dropped out"));
        assert_eq!(log.entries.len(), 1, "memory starts afresh");
        assert!(log.error.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_the_newest_in_memory() {
        let mut log = Log::default();
        for i in 0..KEEP + 5 {
            log.push(String::new(), None, Kind::Cue, i.to_string());
        }
        assert_eq!(log.entries.len(), KEEP);
        assert_eq!(log.entries[0].text, "5");
        assert_eq!(log.count(Kind::Cue), KEEP);
    }
}
