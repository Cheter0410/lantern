# Lantern

Presentation playback for live shows. Opens PowerPoint files and plays them on a display and over NDI,
slide by slide with GO or on timecode. It is a playback tool, not an editor: the presentation is made
in PowerPoint, Keynote or LibreOffice. Written in Rust (egui), for macOS, Windows and Linux.

- **PowerPoint as it looks:** LibreOffice turns the file into a PDF once (cached by file content),
  Lantern renders every slide in the output resolution. Also `.ppt`, `.odp`, `.key` and plain PDFs
  (those without LibreOffice). If LibreOffice is missing, Lantern installs it with one click.
- **Live layout:** slide list with thumbnails, LIVE and NEXT, big GO+ and GO− (one slide back), blackout and reset (set apart on the right, in red and amber; on a row of their own in narrow windows), speaker
  notes, time on the slide, clock and timecode.
- **Per slide:** triggered with **GO** (button, clicker, keyboard, OSC, Show Master) or at a
  **timecode keyframe**; transition as in the file, cut, or fade with its own length.
- **Timecode in:** LTC on any audio input, MIDI timecode (MTC) on a MIDI input of this computer
  (e.g. the IAC bus on a Mac, loopMIDI on Windows, or a USB interface), or MTC over RTP-MIDI (Bonjour,
  so Show Master and Macs find the session "Lantern"). Jumps back locate to the right slide.
- **Outputs:** full-screen on any display, or on several at once with the same picture (*Also on*);
  pure picture: no cursor, no symbols, no menus. And NDI
  (needs the NDI runtime, e.g. with NDI Tools from ndi.video).
- **Test picture:** Settings › Output switches the outputs to a moving test picture in their own
  resolution (geometry, cropping, levels, sharpness, frame counter, clock, timecode, dropped frames
  and delay, smooth motion for judder and tearing); the top bar shows it is on air and turns it off.
- **Stage display:** for the speaker or stage manager on a display of its own (Settings › Stage):
  time, slide, timecode and a countdown, a message from the operator, the slide now and next, and
  the notes in large type. Messages and the countdown are set under the notes (also in Live) or
  over OSC.
- **Web remote:** a page for a phone or tablet in the same network (Settings › Network, with a QR
  code): GO+, GO−, Blackout, the slide now and next, notes, video play / pause, the countdown and
  stage messages; optional PIN.
- **Show log:** every cue with time of day, timecode and how it came (GO, timecode, OSC, web
  remote), blackout, videos, stage messages, problems and when they were fixed, and notes typed in
  live; written to `Logs/<date>.txt` in the show folder (Ctrl/Cmd+L).
- **Output health:** a badge in the top bar when something is wrong – display not connected or
  covering the Lantern window, no output, NDI not sending or without a receiver, timecode missing or
  stopped, sound device gone – with what to do and a way to the settings. A sound device that is
  unplugged is opened again by itself when it is back; videos play on meanwhile.
- **Frame rate:** by default the highest the display supports (Settings › Output › Frame rate), or a
  fixed rate.
- **Remote control (OSC, UDP 53100):** `/lantern/go`, `/lantern/back`, `/lantern/slide <n>`,
  `/lantern/first`, `/lantern/blackout [0|1]`, `/lantern/reset` (or `/clear`); for the live slide's video or sound
  `/lantern/video/play`, `/video/pause`, `/video/toggle`, `/video/restart`, `/video/seek <s>`,
  `/video/skip <±s>`; for the stage display `/lantern/stage/message "text"`, `/stage/clear`,
  `/lantern/timer/start`, `/timer/pause`, `/timer/toggle`, `/timer/reset [minutes]`.
- **On another computer:** Lantern announces itself on the network (Bonjour, `_lantern._udp`); in
  Show Master the Lantern cue lists it under *Network*. Settings › Network shows the name and the IP
  address. Timecode comes over the network too (RTP-MIDI), or as LTC by cable.
- **Start screen and show folders:** Lantern starts with *New show*, *Open show*, *Migrate older
  show* and the recent shows. A new show is a folder at a place you choose, holding the presentation
  (copied or moved there) and `<Show name>.lantern` – copy, back up or take it to another computer as
  one piece. A setup wizard then goes through the output (display, NDI, frame rate, test picture),
  the sound (device, channels, test tone) and the timecode before the main window opens.
  *Migrate* turns an older show (`talk.pptx` with `talk.lantern` next to it) into a show folder,
  with all its settings. *Shows* in the top bar goes back to the start screen.
- A changed presentation is noticed and can be reloaded without losing the slide settings. A
  presentation opened on its own still keeps its settings next to it (`talk.pptx` → `talk.lantern`).

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
8-channel interface), sample rate (44.1 to 96 kHz or the device's own), fade length, a test tone (pink noise) and
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
