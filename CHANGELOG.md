# Changelog

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
