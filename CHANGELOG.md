# Changelog

## 0.12.0

- Stage display over NDI: Settings › Stage › NDI sends the stage display as its own NDI source
  (name and 720p / 1080p to choose). It is drawn off screen like the stage window and only sent
  while a receiver is connected (about 60 Mbit/s at 720p, 120 Mbit/s at 1080p per receiver).
- Stage display in a browser: `/stage` on the web remote's address shows the stage display full
  screen, e.g. on an iPad (uses the web remote's PIN; only a few KB/s).
- The web remote is announced as `http://lantern.local:<port>` (like Show Master); Settings shows
  that address.
- The web remote fits a computer window without scrolling (two columns); it links to the stage view.
- The countdown can be started, paused and reset in Settings › Stage, and its controls under the
  notes show whenever the stage display is on (display, NDI or browser).

## 0.11.6

- CLAUDE.md: the note about tracked `.DS_Store` files is gone (they are no longer in the repository).

## 0.11.5

- The macOS Finder files (`.DS_Store`) are no longer in the repository, so installing no longer reports local changes.

## 0.11.4

- MIDI timecode on Linux: the chosen MIDI input is found again when ALSA gives it another client
  number (after plugging it in again or a restart). The MIDI timecode test now passes on Linux too
  (it failed the release there with "port not listed").

## 0.11.3

- Stagehand is now called Showtime Suite: the install scripts put Lantern into the renamed shared folder
  (`/Applications/Showtime Suite`, `~/.local/share/showtime-suite/apps`, Windows
  `…\Programs\Showtime Suite\Lantern`) and move the old `Stagehand` folder over once, with the menu
  entries and commands that point into it.

## 0.11.2

- CLAUDE.md: how two people (and their Claude Code sessions) work on this repository – straight on
  `main`, pull before work and before push, version and changelog with every change, what to do on
  conflicts.
- `.gitignore` leaves out `.DS_Store`.

## 0.11.1

- release.sh: give an empty release repo a README before publishing

## 0.11.0

- Builds and animations from PowerPoint: shapes and bullet points that come (or go) with a click
  now come with GO, one after the other, before the next slide. Lantern reads the click order from
  the .pptx and lets LibreOffice make a picture of each step; bullets not there yet are invisible,
  so nothing moves. Effects that start by themselves are shown at once.
- A step fades in (as long as the effect, at most a second); "Appear" cuts. The motion of fly-ins,
  zooms and the like is not played.
- GO+ says "Build 2/4" while there are steps left, NEXT shows the next step, LIVE "Slide 2 of 8 ·
  build 1/4"; the list marks slides with builds and shows them finished. GO− goes a step back, and
  to the slide before as it was left (finished).
- Stage display, web remote and show log know the steps too. Time on the slide keeps counting
  through the builds.

## 0.10.0

- Several outputs: Settings › Output › *Also on* sends the same picture to more displays at once –
  projector and LED wall, a second projector, a monitor in the foyer. Each gets its own full-screen
  window without symbols or pointer; slides, transitions, videos, blackout and the test picture go
  to all of them, scaled to fit. The main display sets the resolution. Also in the setup wizard.
- The top bar names every display in use ("Displays 2, 3, 4").
- Output health: red when one of them is not connected, amber when one covers the Lantern window,
  and red when the stage display is on one of them.
- On macOS each output window is kept on its own display (before, windows were found by the start
  of their title).

## 0.9.0

- Show log (*Log* in the top bar, Ctrl/Cmd+L): every cue with the time of day, the timecode when it
  runs, and how it came (GO, GO−, clicked, on timecode, via OSC, via web remote); blackout on and
  off, videos started, paused and ended, stage messages and the countdown, Live on and off, each
  problem from output health and when it was fixed. Filter for cues, problems or notes.
- Notes for the log: type in the field at the bottom of the panel and press Enter – also in Live.
- Written as it happens to `Logs/<date>.txt` in the show folder (one file per day, added to when
  the show is opened again), for the report after the show. *Copy* puts the whole log on the
  clipboard, *File* opens it.
- Messages at the bottom of the window are in the middle now and don't wrap.

## 0.8.0

- Web remote (Settings › Network › Web remote): run the show from a phone or tablet in the same
  network, in its browser – no app to install. Scan the QR code shown there or type the address.
  The page shows the slide now and next (with title and trigger), GO+, GO− and Blackout as big
  buttons, play / pause for the slide's video, the notes, the countdown (Start / Pause, Reset) and
  stage messages (Send, Clear); Reset sits under *More* and asks first. It follows the show a few
  times a second, works in Live, says when the connection is lost and finds it again.
- An optional PIN keeps others in the network out (asked once on each device). Off by default;
  port 8090, changeable. Settings shows how many devices are connected.
- Output health warns when the web remote can't start (e.g. the port is taken).

## 0.7.0

- Stage display (Settings › Stage): a screen of its own for the speaker or stage manager, on any
  display. Across the top the time of day, the slide (3 / 12, time on it), the timecode when it is on
  and a countdown (white, amber in the last minutes, red and counting on when over time). Below a
  message from the operator, the slide now and the next one with its title, and the notes as large
  as they fit. Next slide and notes can be switched off; notes size, countdown length and when it
  turns amber are set there, with a preview.
- Under the notes, while the stage display is on: a message field (Send with Enter, Clear message)
  and the countdown (Start / Pause, Reset). Works in Live too. A new message flashes on stage.
- OSC: `/lantern/stage/message "text"`, `/lantern/stage/clear`, `/lantern/timer/start`,
  `/timer/pause`, `/timer/toggle`, `/timer/reset [minutes]`.
- Output health: warns when the stage display is not connected, and – red – when it is on the same
  display as the output (the audience would see the notes).
- The live and next pictures leave room for the rows below them, so nothing is pushed out of a
  small window.

## 0.6.0

- Output health: Lantern looks at its outputs and inputs twice a second and shows problems as a badge
  in the top bar – red when something does not go out or come in, amber when it is worth a look. A
  new problem is announced and the badge pulses for a few seconds; a click lists every problem with
  what to do, how long it has been there and a button to the settings that fix it. It goes away by
  itself when the problem is gone. Checked:
  - the output display is not connected, or it is the display the Lantern window is on;
  - no output at all (no display, no NDI);
  - NDI does not send, or nobody on the network receives it (receivers are counted);
  - timecode never came although slides wait for it, or it stopped (for more than 2 s);
  - the sound device is gone or does not open.
- A sound device that goes away while playing (unplugged, switched off) is noticed: videos play on
  without sound here instead of freezing, and the device is opened again by itself when it is back.
- The NDI status in the top bar says how many receivers watch.

## 0.5.0

- GO−, GO+, Blackout and Reset keep their places in small windows: the row works out its sizes from
  the room there is – full size when wide, smaller buttons when not, and Blackout and Reset on a row
  of their own (still on the right) when even that does not fit. They never run into each other,
  and there is always a gap between GO+ and Blackout.
- *Clear* is now called *Reset* (back to black before the first slide). OSC: `/lantern/reset`;
  `/lantern/clear` still works.

## 0.4.1

- Video and sound stay together after a jump (timeline, ±10 s, back to the start): sound and clock
  now wait until the first picture at the new place is decoded – the picture starts from a key
  frame and took longer, so it lagged behind the sound. The same at the start of a video.
- Pause stops the picture at once. The picture followed the sound handed to the audio device,
  which is heard only after the device's latency, so it ran ahead and on for a moment after the
  sound had stopped; it now follows the sound actually heard (the device's latency is measured).
- Test tone (Settings › Audio, setup wizard): two seconds of pink noise at about -18 dBFS instead
  of a 1 kHz sine.

## 0.4.0

- Start screen: Lantern opens on a banner with *New show*, *Open show*, *Migrate older show* and the
  recent shows (with folder and last change; missing ones are said and dropped). A file given on the
  command line or dropped on it opens straight away. *Shows* in the top bar goes back to it.
- Show folders: *New show* makes a folder with the show's name at a place you choose and copies or
  moves the presentation into it, next to `<Show name>.lantern` (the presentation is kept relative,
  so the folder can move). A name already taken or a missing presentation is said before anything
  is made.
- Setup wizard for a new show: Output (display, resolution, NDI, frame rate, test picture) →
  Audio (device, channels, test tone, level) → Timecode (LTC, MTC from a MIDI input or the network,
  with what is received) → Ready (a summary, and the presentation loading meanwhile). Back, Next and
  *Skip setup*; the main window opens only after it. The test picture turns off when the show opens.
- *Migrate older show*: a `.lantern` file or a presentation with its `.lantern` next to it becomes a
  show folder (copied or moved); slide settings, outputs, sound and timecode come along. When moved,
  the old show file is kept in the folder as `migrated-from.lantern.bak`.
- Lantern no longer opens the last show by itself; it is at the top of *Recent shows*.

## 0.3.4

- Test picture runs on the real clock: the 1 s flash comes exactly on every full second and the frame
  boxes move by time, however fast the pictures come (a dropped picture shows as a skipped box).
  Before, it counted pictures and ran slow when fewer came than set.
- Test picture is about ten times faster: only the moving parts are drawn again and sent to the
  graphics card (0.6 ms per picture in Full HD, 2.9 ms in 4K).
- Smooth motion in the test picture: a scrolling soft grating, a hand gliding round once a second, a
  ball gliding along the big circle and a sweep bar at its exact sub-pixel place – judder, tearing and
  stepping show there; the steps row and the flash stay for counting frames.
- Settings › Output › Frame rate for all outputs: by default the highest rate the chosen display
  supports (60 without a display; NDI follows up to 60), or a fixed 24 to 144 frames/s. Older show
  files switch to the automatic rate.

## 0.3.3

- GO+, GO−, Blackout and Clear stay in the same place on every slide: the video bar under the Live
  picture is always there (empty when the slide has no video or sound), and a video that can't play
  says so inside the bar.
- Test picture for the outputs (Settings › Output › *Test picture on the outputs*): drawn pixel for
  pixel in the output's resolution, on the display and over NDI. Grid, circles and corner marks
  (geometry, aspect, cropping, mirroring), safe areas, colour bars, grey steps and colour ramps
  (levels, banding), 1-pixel line patches (scaling), and live: output name, resolution, frame rate,
  frame counter, time of day with frames, timecode in, a hand turning once a second, one lit box per
  frame (dropped frames), a flash every full second (delay) and a bar sweeping across (judder).
  While it is on, the top bar says TEST PICTURE ON AIR with a button to turn it off (also in Live);
  it is never saved, Lantern always starts with the slides.

## 0.3.2

- Transport for the live slide's video or sound, in a bar under the Live picture: back to the start,
  10 s back, play / pause / resume / again, 10 s on, and a timeline to click or drag, with the time
  played and the time left. Pausing keeps the picture and fades the sound in a few milliseconds
  (no click); jumping keeps it paused or playing. Key K plays and pauses.
- Sounds (audio clips, e.g. MP3) in slides play too, with the same settings and transport.
- Clock strip: the video cell shows PAUSED while paused.
- OSC: `/lantern/video/play`, `/video/pause`, `/video/toggle`, `/video/restart`,
  `/video/seek <seconds>`, `/video/skip <seconds>`.

## 0.3.1

- Timecode in: MIDI timecode (MTC) on a MIDI input of this computer, e.g. the IAC bus on a Mac
  (loopMIDI on Windows) or a USB MIDI interface. Settings › Timecode › *MTC (MIDI input)*, with the
  input to pick; the clock strip shows "MTC". A missing input is tried again every two seconds.
- *Back* is now *GO−*, *GO* is *GO+*; GO− sits left of GO+.
- Blackout and Clear moved to the far right, apart from GO and from each other (with a line between),
  Blackout in red, Clear in amber, so neither is hit by mistake.

## 0.3.0

- Videos inside PowerPoint slides play: in their place on the display, on the Live picture and
  over NDI, in time with their sound. Start (as in the file, automatic, with GO) and end (hold,
  loop, next slide) and the volume can be set per slide; GO shows *Play video* when a video waits
  for it.
- New Settings › Audio: output device, stereo or mono on any channel pair, sample rate, fade
  length for the sound when the slide changes, test tone and level meter. NDI carries the sound too.
- Live: a *Video* cell in the clock strip with the time left and the sound level; the time left
  also shows above the Live picture.
- The packages for macOS, Windows and Linux bring FFmpeg along (license in `FFMPEG.txt`); Settings ›
  Program shows which FFmpeg is used, a notice tells when a video can't play.

## 0.2.0

- The whole interface is in English now (buttons, settings, messages, LibreOffice and NDI
  notices).
- Buttons keep their size on hover: no outline that squeezes them, hover only lightens the fill.
- Switching between Edit and Live no longer shows a message in the corner.

## 0.1.2

- Colours as in Show Master: the next slide is green, the slide on the output blue; Live mode is
  red, Edit blue, with a red LIVE badge and frame while Live.
- Live: a clock strip across the top with large digits for the time of day, the timecode (with
  source, frame rate and signal state), the time on the slide and the slide number.
- Buttons no longer grow on hover; nothing moves when the mouse passes over them.

## 0.1.1

- Edit and Live mode ("Bearbeiten" / "Live", top right, Ctrl/Cmd+E): in Live the slide settings,
  opening a file, reloading and drag and drop are locked and the settings can only be looked at.
  GO, back, blackout, timecode and OSC work as before. The mode is kept across restarts.
- macOS: the display output covers the whole display, no more strip of desktop at the top.

## 0.1.0

- First version: PowerPoint playback through LibreOffice (one-click install), live layout with
  LIVE / NEXT, GO, blackout and notes, per-slide GO or timecode trigger and transition, LTC and
  RTP-MIDI timecode in, full-screen display and NDI out, OSC remote control.
