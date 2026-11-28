# Testing, benches and GUI checks

<!-- Moved from CLAUDE.md, which points here. -->

## Commands

```
uv run build.py                  # closes a metalamp.exe running from target\, then cargo build --release
cargo build --release            # target\release\metalamp.exe (fat LTO)
cargo test                       # unit tests (<what they cover>)
cargo clippy --all-targets       # must stay warning-free
python tools/make_release.py     # tests, release build, dist\MetalAMP_<ver>_Setup.exe (Inno Setup 6, tools/setup.iss) and _portable.zip
python tools/make_release.py --no-tests --install   # the same, then installs it silently over the installed copy
.\target\release\metalamp.exe --export-icon file.ico   # the app icon for the installer
python tools/icon_png.py         # images\icon.png (the README's icon) from the exe's icon
python tools/notices.py          # licenses\THIRD_PARTY_NOTICES.txt after the dependencies change (a test fails until then)
.\target\release\metalamp.exe --bench scan D:\Music   # a full scan without the index: walk, tags, grouping, index size, folders split into albums
.\target\release\metalamp.exe --bench gapless a.mp3 b.mp3   # what the decoder gives (trimmed) against each file's LAME tag
.\target\release\metalamp.exe --bench loudness a.mp3   # EBU R128 loudness, peak and the normalisation gain, with the time taken (ffmpeg -af ebur128 agrees to 0.1 LU)
.\target\release\metalamp.exe --bench render a.mp3 out.wav 60 12   # the passage (from 60 s, 12 s; a gain in dB after them, as normalisation's) through the chain with the settings' sound, as a float WAV with the chain's delay taken out, to compare with a recording of another player (the enhancer's model); `raw` after the gain: the enhancer alone, without the limiter
RUST_LOG=metalamp=debug          # debug log (index and scan timings); METALAMP_SCAN_THREADS=n sets the tag readers (default min(cores, 8))
cargo test reads_a_real_file -- --ignored   # with METALAMP_SAMPLE=<an mp3>: lofty reads its tags
cargo test decoder_opens_sample -- --ignored   # METALAMP_SAMPLE: Media Foundation decodes it and seeks
cargo test engine_goes_on_without_a_gap -- --ignored --nocapture   # METALAMP_SAMPLE and the next file of its folder: plays ~3 s quietly on the default device
cargo test play_while_paused_plays -- --ignored --nocapture   # METALAMP_SAMPLE: a Play after a Pause plays (~2 s quietly)
cargo test --release engine_crossfades_into_a_track_chosen -- --ignored --nocapture   # a track chosen while another plays crossfades into it (generated sines, gain 0: silent)
cargo test --release engine_crossfades_a_seek -- --ignored --nocapture   # a seek with the crossfade on (silent)
cargo test --release engine_pauses_without_a_click -- --ignored --nocapture   # a sine played into VB-Audio's virtual cable and recorded from its other end: no click at a pause, resume, seek or stop (nothing reaches the speakers; METALAMP_CAPTURE=<file> keeps the recording as raw f32; on this machine the capture stream's `Initialize` waits for the antivirus to allow it (the user's answer; unanswered, it failed with E_INVALIDARG after 8-10 min); warn the user before running it)
METALAMP_ALBUM=<folder> cargo test --release prepares_a_copy -- --ignored --nocapture   # a copy of an album's folder (in %TEMP%) prepared by the default profile (the other files kept), the plan printed, prepared again: nothing left to do
cargo test --release paint_costs -- --ignored --nocapture   # a frame of the ridges, strata, vortex, spectrum, fire, fireworks, aurora, goniometer and strings, filled, in a full bar's display: feed, paint, tessellation (ridges ~0.8 ms in all; the scene clock advances by 20 ms per frame, so fire and fireworks are filled too)
cargo test --release visual_frames -- --ignored   # with METALAMP_VISUAL=<the settings' name> and METALAMP_FRAMES=<file.png> (METALAMP_THEME=light for the light theme): seven frames of a visualization fed made-up music (`visual::tests::music`: a kick, a snare, hi-hats, a bass line, chords), drawn by hand into a PNG to look at (`ui/raster.rs`: egui's meshes rasterised with their textures, no window; also the spectrum's PNG; `METALAMP_FRAMES_EVERY=<n>`: the frames n apart, 25 by default, for quick moves; `METALAMP_SAMPLE=<file>` (from `METALAMP_FROM` s, 60 by default): its sound in place of the made-up music, `visual::tests::real_file_mono`); `fireworks_frames` likewise
```

Trying a change: `cargo test` and `cargo clippy --all-targets`, then `uv run build.py` and run `target\release\metalamp.exe`; a test that plays sound should lower `volume` in `settings.json` first and put it back (it is the user's music on the user's speakers). When the change concerns the installed copy (installer, associations, portable mode) or the user runs the program all day, `python tools/make_release.py --no-tests --install` replaces the installed copy silently (it closes the running one first, so its settings are saved).

## GUI checks

- GUI checks: the OpenGL client area cannot be captured with `PrintWindow` on an NVIDIA machine (the caption can, from a DPI-aware process); keys can be posted with `PostMessage(WM_KEYDOWN/WM_KEYUP)`, but each posted key reaches egui three times (seen in a trace of `raw_input_hook`: an arrow moved three rows), and posted `WM_CHAR` and Ctrl do not reach it (winit reads the keyboard's state), so typing and counts of steps are checked by the user; the screen can be captured with `CopyFromScreen` from a DPI-aware process (this machine's OpenGL area is captured). A launched window takes the focus: warn the user before launching test windows and before any simulated global mouse or keyboard input, and close test windows with `taskkill /PID` (WM_CLOSE, which saves the settings; `taskkill` sends nothing to a minimized window, so a minimized one is closed by posting WM_CLOSE to its windows, as `build.py`'s `post_close` does). A test copy runs apart from the installed one with a `metalamp.json` next to `target\release\metalamp.exe` (portable mode: its own settings, index and covers; remove them after, the portable zip must not pick them up); `SendKeys` (SendInput) does reach egui, unlike posted messages, but simulated keys and clicks go to whatever window is in front (once the user's editor): do not simulate input; to look at a settings page, start the copy with `METALAMP_SETTINGS=sound:420` (`SettingsDialog::from_env`: the page and a scroll offset) and only capture the screen. A tooltip open when a modal opens by key stays until the pointer leaves its widget: egui keeps an open tooltip while the pointer is in its rect, whatever lies over it.
