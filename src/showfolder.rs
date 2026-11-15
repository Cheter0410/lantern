//! Show folders: a show is a folder holding its presentation and its show
//! file, `<folder>/<Name>.lantern`, so it can be copied, backed up or taken
//! to another computer as one piece.
//!
//! - New show: make the folder at a place the user picks and copy or move the
//!   presentation into it.
//! - Migrate: an older show (a `.lantern` file next to its presentation, or
//!   just the presentation) becomes a show folder; its settings come along.

use crate::show::Show;
use std::path::{Path, PathBuf};

/// Whether the presentation is copied into the show folder or moved there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Placement {
    #[default]
    Copy,
    Move,
}

/// What a new or migrated show folder is made from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// Name of the show (and of its folder and show file).
    pub name: String,
    /// Where the show folder is made.
    pub location: PathBuf,
    /// The presentation to put into it.
    pub presentation: PathBuf,
    pub placement: Placement,
}

/// Presentations Lantern opens.
pub const PRESENTATIONS: &[&str] = &["pptx", "ppt", "odp", "key", "pdf"];

pub fn is_presentation(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| PRESENTATIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// A name usable for a folder and a file on every system.
pub fn clean_name(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    s.trim_matches(|c: char| c == '.' || c.is_whitespace())
        .to_string()
}

/// A name for a show from its presentation: `Talk 2026.pptx` -> `Talk 2026`.
pub fn name_from(presentation: &Path) -> String {
    presentation
        .file_stem()
        .map(|s| clean_name(&s.to_string_lossy()))
        .unwrap_or_default()
}

/// Where new shows go unless the user picks another place.
pub fn default_location() -> PathBuf {
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Lantern Shows")
}

impl Plan {
    pub fn folder(&self) -> PathBuf {
        self.location.join(clean_name(&self.name))
    }

    pub fn show_file(&self) -> PathBuf {
        self.folder()
            .join(format!("{}.lantern", clean_name(&self.name)))
    }

    /// What is wrong with the plan, in words for the user (None: fine).
    pub fn problem(&self) -> Option<String> {
        if clean_name(&self.name).is_empty() {
            return Some("Give the show a name.".into());
        }
        if self.presentation.as_os_str().is_empty() {
            return Some("Choose the presentation.".into());
        }
        if !self.presentation.is_file() {
            return Some(format!(
                "The presentation is not there: {}",
                self.presentation.display()
            ));
        }
        if !is_presentation(&self.presentation) {
            return Some("That is not a presentation Lantern can play.".into());
        }
        if self.location.as_os_str().is_empty() {
            return Some("Choose where the show folder goes.".into());
        }
        let folder = self.folder();
        if folder.exists() && std::fs::read_dir(&folder).map_or(true, |mut d| d.next().is_some()) {
            return Some(format!(
                "There is already a folder \"{}\" there – choose another name or place.",
                clean_name(&self.name)
            ));
        }
        None
    }

    /// Make the show folder, put the presentation in and save `show` there
    /// (pointing to it). Returns the show file.
    pub fn create(&self, mut show: Show) -> Result<PathBuf, String> {
        if let Some(p) = self.problem() {
            return Err(p);
        }
        let folder = self.folder();
        std::fs::create_dir_all(&folder)
            .map_err(|e| format!("Could not make {}: {e}", folder.display()))?;
        let file_name = self
            .presentation
            .file_name()
            .ok_or("The presentation has no file name")?;
        let target = folder.join(file_name);
        let placed = match self.placement {
            Placement::Copy => copy(&self.presentation, &target),
            Placement::Move => move_file(&self.presentation, &target),
        };
        if let Err(e) = placed {
            // Leave nothing half made behind.
            let _ = std::fs::remove_dir(&folder);
            return Err(e);
        }
        show.presentation = target;
        let file = self.show_file();
        show.save(&file)?;
        Ok(file)
    }
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("Could not copy {}: {e}", from.display()))
}

/// Move; across drives (where renaming fails) copy and then delete.
fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy(from, to)?;
    std::fs::remove_file(from).map_err(|e| {
        format!(
            "Copied, but could not remove the original {}: {e}",
            from.display()
        )
    })
}

/// An older show to migrate: its show file (if any) and its presentation.
#[derive(Clone, Debug, PartialEq)]
pub struct Old {
    pub show: Option<PathBuf>,
    pub presentation: PathBuf,
}

/// Find what belongs to an older show, from its `.lantern` file or its
/// presentation (with the `.lantern` next to it, if there is one).
pub fn find_old(path: &Path) -> Result<Old, String> {
    let is_show = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("lantern"));
    if is_show {
        let s = Show::load(path)?;
        if !s.presentation.is_file() {
            return Err(format!(
                "The presentation of this show is missing: {}",
                s.presentation.display()
            ));
        }
        return Ok(Old {
            show: Some(path.to_path_buf()),
            presentation: s.presentation,
        });
    }
    if !is_presentation(path) {
        return Err("Choose a .lantern show file or a presentation.".into());
    }
    let side = crate::app::show_file_for(path);
    Ok(Old {
        show: side.is_file().then_some(side),
        presentation: path.to_path_buf(),
    })
}

/// Make a show folder from an older show; its settings (slides, outputs,
/// sound, timecode) come along. Returns the new show file.
pub fn migrate(old: &Old, plan: &Plan, fallback: Show) -> Result<PathBuf, String> {
    let show = match &old.show {
        Some(f) => Show::load(f)?,
        None => fallback,
    };
    let file = plan.create(show)?;
    // A moved presentation leaves its old show file behind: move it too,
    // as a backup next to the new one.
    if plan.placement == Placement::Move {
        if let Some(f) = &old.show {
            if let Some(dir) = file.parent() {
                let _ = move_file(f, &dir.join("migrated-from.lantern.bak"));
            }
        }
    }
    Ok(file)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::show::{SlideCfg, TcSource};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lantern-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn names() {
        assert_eq!(clean_name("  Gala: 2026/27?  "), "Gala- 2026-27-");
        assert_eq!(clean_name("..."), "");
        assert_eq!(
            name_from(Path::new("/x/Keynote Final.pptx")),
            "Keynote Final"
        );
    }

    #[test]
    fn new_show_copies_or_moves_the_presentation() {
        let d = temp("newshow");
        let src = d.join("talk.pptx");
        std::fs::write(&src, b"pptx").unwrap();
        let mut plan = Plan {
            name: "Gala 2026".into(),
            location: d.join("shows"),
            presentation: src.clone(),
            placement: Placement::Copy,
        };
        assert_eq!(plan.problem(), None);
        let file = plan.create(Show::default()).unwrap();
        assert_eq!(file, d.join("shows/Gala 2026/Gala 2026.lantern"));
        assert!(src.is_file(), "copy keeps the original");
        let s = Show::load(&file).unwrap();
        assert_eq!(s.presentation, d.join("shows/Gala 2026/talk.pptx"));
        assert!(s.presentation.is_file());
        // The show file keeps the presentation relative: the folder can move.
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"presentation\": \"talk.pptx\""), "{text}");

        // The same name again: refused, nothing changed.
        assert!(plan.problem().unwrap().contains("already a folder"));
        assert!(plan.create(Show::default()).is_err());

        plan.name = "Moved".into();
        plan.placement = Placement::Move;
        let file = plan.create(Show::default()).unwrap();
        assert!(!src.exists(), "move takes the original away");
        assert!(file.parent().unwrap().join("talk.pptx").is_file());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn problems_are_said_in_words() {
        let d = temp("problems");
        let plan = Plan {
            name: " ".into(),
            ..Default::default()
        };
        assert!(plan.problem().unwrap().contains("name"));
        let plan = Plan {
            name: "A".into(),
            location: d.clone(),
            presentation: d.join("missing.pptx"),
            ..Default::default()
        };
        assert!(plan.problem().unwrap().contains("not there"));
        std::fs::write(d.join("notes.txt"), b"x").unwrap();
        let plan = Plan {
            presentation: d.join("notes.txt"),
            ..plan
        };
        assert!(plan.problem().unwrap().contains("not a presentation"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn migrates_an_older_show_with_its_settings() {
        let d = temp("migrate");
        let pres = d.join("old talk.pptx");
        std::fs::write(&pres, b"pptx").unwrap();
        let mut old_show = Show {
            presentation: pres.clone(),
            ..Show::default()
        };
        old_show.timecode.source = TcSource::Ltc;
        old_show.slides.push(SlideCfg {
            title: "Intro".into(),
            ..Default::default()
        });
        let side = crate::app::show_file_for(&pres);
        old_show.save(&side).unwrap();

        // Found from the presentation or from the show file alike.
        let old = find_old(&pres).unwrap();
        assert_eq!(old.show.as_deref(), Some(side.as_path()));
        assert_eq!(find_old(&side).unwrap(), old);

        let plan = Plan {
            name: name_from(&pres),
            location: d.join("new"),
            presentation: old.presentation.clone(),
            placement: Placement::Move,
        };
        let file = migrate(&old, &plan, Show::default()).unwrap();
        let s = Show::load(&file).unwrap();
        assert_eq!(s.timecode.source, TcSource::Ltc, "settings came along");
        assert_eq!(s.slides.len(), 1);
        assert_eq!(s.presentation, d.join("new/old talk/old talk.pptx"));
        assert!(
            !pres.exists() && !side.exists(),
            "moved, old show file kept as backup"
        );
        assert!(d.join("new/old talk/migrated-from.lantern.bak").is_file());

        // A presentation without a show file migrates with the fallback.
        let lone = d.join("lone.pdf");
        std::fs::write(&lone, b"%PDF").unwrap();
        let old = find_old(&lone).unwrap();
        assert_eq!(old.show, None);
        assert!(find_old(&d.join("x.txt")).is_err());
        let _ = std::fs::remove_dir_all(d);
    }
}
