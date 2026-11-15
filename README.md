# Lantern

Presentation playback for live shows. Opens PowerPoint files and plays them on a display and over NDI,
slide by slide with GO or on timecode. It is a playback tool, not an editor: the presentation is made
in PowerPoint, Keynote or LibreOffice. Written in Rust (egui), for macOS, Windows and Linux.

- **PowerPoint as it looks:** LibreOffice turns the file into a PDF once (cached by file content),
  Lantern renders every slide in the output resolution. Also `.ppt`, `.odp`, `.key` and plain PDFs
  (those without LibreOffice). If LibreOffice is missing, Lantern installs it with one click.
- **Live layout:** slide list with thumbnails, LIVE and NEXT, big GO+ and GO− (one slide back), blackout and clear (set apart on the right, in red and amber), speaker
  notes, time on the slide, clock and timecode.
- **Per slide:** triggered with **GO** (button, clicker, keyboard, OSC, Show Master) or at a
  **timecode keyframe**; transition as in the file, cut, or fade with its own length.
- **Timecode in:** LTC on any audio input, MIDI timecode (MTC) on a MIDI input of this computer
  (e.g. the IAC bus on a Mac, loopMIDI on Windows, or a USB interface), or MTC over RTP-MIDI (Bonjour,
  so Show Master and Macs find the session "Lantern"). Jumps back locate to the right slide.
- **Outputs:** full-screen on any display (pure picture: no cursor, no symbols, no menus) and NDI
  (needs the NDI runtime, e.g. with NDI Tools from ndi.video).
- **Remote control (OSC, UDP 53100):** `/lantern/go`, `/lantern/back`, `/lantern/slide <n>`,
  `/lantern/first`, `/lantern/blackout [0|1]`, `/lantern/clear`; for the live slide's video or sound
  `/lantern/video/play`, `/video/pause`, `/video/toggle`, `/video/restart`, `/video/seek <s>`,
  `/video/skip <±s>`.
- **On another computer:** Lantern announces itself on the network (Bonjour, `_lantern._udp`); in
  Show Master the Lantern cue lists it under *Network*. Settings › Network shows the name and the IP
  address. Timecode comes over the network too (RTP-MIDI), or as LTC by cable.
- The settings of a presentation are kept next to it (`talk.pptx` → `talk.lantern`); a changed
  presentation is noticed and can be reloaded without losing them.

Keys: Space, →, ↓, Page Down, Enter, N = GO+ · ←, ↑, Page Up, Backspace, P = GO− · B or . = blackout · K = play / pause the video ·
Home = first slide · Esc = close the slide settings · Ctrl/⌘+O = open · Ctrl/⌘+R = read the file again ·
Ctrl/⌘+E = switch between Edit and Live.

**Edit and Live:** in *Live* (top right) the show is locked against accidental changes: no slide
settings, no opening, reloading or dropping files, settings only to look at. GO, back, blackout,
timecode and OSC work as always. The mode is kept across restarts.

**Videos in slides:** a video in a PowerPoint slide plays in its place on the display and over NDI,
with its sound. As in the file it starts by itself or with a click; per slide it can also start
automatically or wait for GO (the GO button then says *Play video*), and at its end hold the last
frame, loop or go on to the next slide. Volume per video, the sound fades out with the slide
(Settings › Audio › Fade). In Live a *Video* cell shows the time left and the sound level. Sounds
(audio clips) in slides play the same way. Under the Live picture a transport bar controls the live
slide's video or sound: back to the start, ±10 s, play / pause (key K) and a timeline to click or drag.
Lantern plays videos with FFmpeg, which the packages bring along (see `packaging/FFMPEG.txt`);
when built from source it uses an installed `ffmpeg` (e.g. `brew install ffmpeg`).

**Sound (Settings › Audio):** output device, stereo or mono and on which channels (e.g. 3–4 of an
8-channel interface), sample rate (44.1 to 96 kHz or the device's own), fade length, a test tone and
a level meter. With NDI on, the sound goes along with the picture too.

Not yet: animations inside slides (a slide shows its final state).

## Build and install

Linux (no sudo: it goes into `~/.local/share/stagehand/apps`, the folder Stagehand installs all apps
into; run it again to update, `./install.sh --uninstall` removes it):

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
