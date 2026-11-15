# Changelog

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
