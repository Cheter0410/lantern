# Lantern – notes for Claude Code

Presentation playback for live shows: PowerPoint/Keynote/PDF via LibreOffice, GO or timecode, full-screen
outputs on several displays and NDI, videos, stage display, web remote, show log, builds. Rust + egui.

## Who works here

Two people work on this repository, each with Claude Code: **Cheter0410** (owner) and **jay212-1**
(Jan). Both push to the same branch, so the rules below exist to keep us on the same state and away
from merge conflicts. Follow them in every session.

## Git workflow

- **Work directly on `main`.** No feature branches, no pull requests (agreed on 2026-11-15).
- **Before you start:** `git pull --rebase`. The other person (or a release run) may have pushed.
- **Before every push:** `git pull --rebase` again, then build and test, then `git push`.
- **Commit and push soon** – small commits, pushed the same day. Unpushed work piling up is what makes
  conflicts.
- Never force-push `main`, never rewrite pushed history, never delete or move tags (`vX.Y.Z` tags are
  what the release builds).
- A rebase stops on a conflict: resolve it, `git rebase --continue`. When unsure whose change wins,
  stop and ask instead of guessing.

## Version and changelog with every change

- Every change raises the version: `version = "x.y.z"` in `Cargo.toml` (`[workspace.package]`),
  then `cargo update -w` so `Cargo.lock` follows. Small changes: patch step (z+1); bigger features: minor step.
- Add a `## x.y.z` section at the top of `CHANGELOG.md` (English, like the entries below it).
- Do the bump **right after** `git pull --rebase`, just before committing – never on a stale state.
- **Conflict on the version** (both of us bumped): keep the other one's version and entry, put yours
  on top as the next number. In `CHANGELOG.md` keep both sections, newest first.
- **Conflict in `Cargo.lock`:** never merge it by hand. Take the file that is already on `main`
  (during a rebase that is `git checkout --ours Cargo.lock`), run `cargo update -w` and `cargo build`,
  then `git add Cargo.lock` and continue.
- `release.sh` also bumps the version by itself when the released version has changes after it, and
  commits, tags and pushes that. So pull before you work – a release may have added a commit.

## Never commit

Build output and local files: `target/`, `dist/`, `.DS_Store`, logs, editor files. They are in
`.gitignore`; keep it that way.

## Releases

Released files go to the public repository **`Cheter0410/lantern-releases`**. It only holds the downloads (and a
README) – never develop there. Installed copies and Showtime Suite read the newest release from it.

`./release.sh` publishes (Arch PC: Linux + Windows, Mac: the DMG); `./release.sh status` shows what is
up. Works like Show Master's (`show-master/docs/release-anleitung.md`).

## The other projects

All live side by side in `~/Desktop/dev` (folder names in lower case):

| Folder | Repository | Release downloads |
|---|---|---|
| `show-master` | Cheter0410/Show-Master | Cheter0410/show-master-releases |
| `lantern` | Cheter0410/lantern | Cheter0410/lantern-releases |
| `showtime-suite` | Cheter0410/showtime-suite | Cheter0410/showtime-suite-releases |
| `beamweave` | Cheter0410/beamweave | Cheter0410/beamweave-releases |

Showtime Suite installs, updates and starts the others; its catalog is `showtime-suite/apps.json`.

## Build and test

```bash
cargo test                       # unit and UI tests (egui_kittest); some need LibreOffice / FFmpeg
cargo clippy --all-targets
LANTERN_SHOTS=/some/folder cargo test   # the UI tests also write screenshots of every step
```

UI tests use their own settings folder – they never touch the real Lantern settings.

## Project notes

- The UI is in English; keep new texts English and plain.
- Changes to the window: check them with a UI test and its screenshots (small windows too).
- `crates/sm-timecode`, `sm-rtpmidi`, `sm-mdns` are copies from Show Master – a fix in one usually
  belongs in the other too.
- `scripts/setup.sh` is the same file in Show Master, Showtime Suite and here – change all three together.
