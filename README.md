# Lantern

Presentation playback for live shows. Opens PowerPoint files and plays them on a display and over NDI,
slide by slide with GO or on timecode. It is a playback tool, not an editor: the presentation is made
in PowerPoint, Keynote or LibreOffice. Written in Rust (egui), for macOS, Windows and Linux.

- **PowerPoint as it looks:** LibreOffice turns the file into a PDF once (cached by file content),
  Lantern renders every slide in the output resolution. Also `.ppt`, `.odp`, `.key` and plain PDFs
  (those without LibreOffice). If LibreOffice is missing, Lantern installs it with one click.
- **Live layout:** slide list with thumbnails, LIVE and NEXT, big GO, back, blackout, clear, speaker
  notes, time on the slide, clock and timecode.
- **Per slide:** triggered with **GO** (button, clicker, keyboard, OSC, Show Master) or at a
  **timecode keyframe**; transition as in the file, cut, or fade with its own length.
- **Timecode in:** LTC on any audio input, MIDI timecode over RTP-MIDI (Bonjour, so Show Master and
  Macs find the session "Lantern"). Jumps back locate to the right slide.
- **Outputs:** full-screen on any display (pure picture: no cursor, no symbols, no menus) and NDI
  (needs the NDI runtime, e.g. with NDI Tools from ndi.video).
- **Remote control (OSC, UDP 53100):** `/lantern/go`, `/lantern/back`, `/lantern/slide <n>`,
  `/lantern/first`, `/lantern/blackout [0|1]`, `/lantern/clear`.
- The settings of a presentation are kept next to it (`talk.pptx` → `talk.lantern`); a changed
  presentation is noticed and can be reloaded without losing them.

Keys: Space, →, ↓, Page Down, Enter, N = GO · ←, ↑, Page Up, Backspace, P = back · B or . = blackout ·
Home = first slide · Ctrl/⌘+O = open.

Not yet: videos and animations inside slides (a slide shows its final state).

## Build and install

Linux:

```bash
./install.sh
```

macOS:

```bash
bash install-mac.sh
```

## Release

```bash
./release.sh
```

See the release guide of Show Master (`docs/release-anleitung.md`); Lantern works the same way and
publishes to `Cheter0410/lantern-releases`.
