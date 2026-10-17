# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

MetalAMP is a Windows-only audio player written in Rust with egui/eframe 0.36 (glow), a replacement for Winamp whose main part is a Winamp-style media library: artists on the left, a grid of albums with their covers, the tracks below, the player bar on top, the playlist on the right, a status line. Playback is Windows' own: Media Foundation decodes (MP3, AAC/M4A, FLAC, WMA, WAV) and WASAPI plays. Priorities: start-up speed, a library that shows at once and stays smooth with thousands of tracks, gapless playback, no external runtime or DLLs. The implementation plan agreed with the user is `local/PLAN.md` (Russian, not in the repository); deferred work is in `TODO.md`. `README.md` / `README.ru.md` are the user-facing docs and must be kept in sync (both languages, cross-linked, formal tone, no imperatives).

The name is written `MetalAMP` wherever people read it (UI, window title, READMEs, installer, `%APPDATA%\MetalAMP`, `%LOCALAPPDATA%\Programs\MetalAMP`, `MetalAMP_<ver>_Setup.exe`) and `metalamp` wherever it is an identifier (Cargo package, `metalamp.exe`, the GitHub repo `Fan4Metal/metalamp`).

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
```

Releases: pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`, which runs `make_release.py` and publishes both files; the tag must equal the version in `Cargo.toml`. Creating or pushing a tag publishes a public release, so do it only when the user asks. `tools/setup.iss` installs per user. `.cargo/config.toml` links the CRT statically, so no VCRUNTIME140.dll is needed. These scripts mirror those of disk_flashlight, qview, qrec and metalmd; keep them in step.

Trying a change: `cargo test` and `cargo clippy --all-targets`, then `uv run build.py` and run `target\release\metalamp.exe`; when the change concerns the installed copy (installer, associations, portable mode) or the user runs the program all day, `python tools/make_release.py --no-tests --install` replaces the installed copy silently (it closes the running one first, so its settings are saved).

Toolchain: stable MSVC Rust plus VS 2022 Build Tools. In the Bash tool, `cargo` is not on PATH; prefix commands with `export PATH="$HOME/.cargo/bin:$PATH"`. A running `metalamp.exe` locks the release binary; `build.py` closes it first (WM_CLOSE via `taskkill`, so settings are saved; killed after 3 s). Heredocs containing Rust code break the Bash tool (lifetime apostrophes); write source files with the Write tool or a Python script.

## Architecture

- **`main.rs`**: arguments (`--export-icon`, `--version`, `--help`; the release exe has no console, so the command line modes call `win::attach_parent_console`), the panic hook, `NativeOptions` (glow, `centered` only without saved settings, `persistence_path` for a portable copy), `eframe::run_native`.
- **`app.rs`**: the eframe `App` (egui 0.36: `fn ui(&mut self, ui, frame)` with `egui::Panel::top/left/bottom(...).show(ui, ...)`, not the older `update(ctx)` / `SidePanel` API). `App::new` reads the settings, sets the language and the theme and the dark caption while the window is hidden; `follow_theme` repaints the caption in the frame after a theme change; `save` writes the settings.
- **`icon.rs`** draws the app icon procedurally with no dependencies; `build.rs` includes it via `#[path]` to write the `.ico` embedded by `winresource` and `app_icon_{64,128,256}.rgba` into `OUT_DIR`; `main::embedded_icon` and `ui::app_icon` use those, so nothing is rasterised at start-up.
- **`settings.rs`**: plain strings in eframe's storage (`%APPDATA%\MetalAMP\data\app.ron`), or `app.ron` next to the exe when that file exists and can be written (portable mode, `settings::location`, decided once per run; the portable zip ships it empty). About shows which.
- **`update.rs`**: the opt-in update check (off by default, once a day at start-up, "Check now" in About): `win::https_get` (WinHTTP) asks `api.github.com/repos/…/releases/latest`, `tag_name` is picked out of the JSON by hand, a newer release shows `ui::update_button` beside About, opening the release page built from `CARGO_PKG_REPOSITORY`. `cargo test update -- --ignored` asks GitHub for real.
- **`caption.rs`**: `DWMWA_USE_IMMERSIVE_DARK_MODE` for the caption, `WM_NCACTIVATE` off and on to repaint it (Windows 10 repaints it only on the next non-client repaint).
- **`win.rs`**: `error_box` (user32 `MessageBoxW` declared by hand), `attach_parent_console`, `shell_open` (`ShellExecuteW`; eframe's `links` feature is off), `https_get`.
- **`ui/`**: `app_icon`, toolbar buttons with a painted icon (`icon_button`), `theme_button` (as in Windows → light → dark), `about_button`, `update_button`, `close_cross`; `ui/about.rs` the About modal; `ui/start.rs` the start screen while nothing is open (icon, name, version, one line on how to begin).
- **`ui/icons.rs`**: the painted icons shared by the family (the default fonts lack many symbols), on a 14-point grid with 1.3-point lines. A new icon is added there on the same grid and checked next to the others with `cargo run --example icons`, not drawn ad hoc in a panel.
- **`icon.rs`** (the drawing): an electric guitar (single-cutaway body in cherry red with cream binding, rosewood neck, black headstock with a light edge so it shows on a dark taskbar) lying from the bottom left to the top right over a yellow lightning bolt, lit from the top with a soft shadow; layers of 24 px and less drop the strings, frets, knobs, tuners and binding and get a wider neck. `sample(x, y, px)` smooths edges over one pixel of the layer (signed distance fields: circles, a smooth union, a polygon for the bolt).
- <the app's own modules, one bullet each: what it does, why it is done that way, the numbers measured>

## Conventions

- Every user-visible string goes through `tr!("English", "Русский")` (`i18n.rs`); the language follows the Windows UI language (`i18n::system_lang`, `GetUserDefaultUILanguage`) unless chosen in About (`i18n::LangChoice`, `system`/`en`/`ru` in the settings; applied in `App::new` before any text is made and at once when changed). Strings are built every frame, so nothing caches translated text. Log messages, CLI output and technical errors stay English.
- Persisted state: `theme` (`system`/`light`/`dark`), `language`, `check_updates`, `last_update_check` (Unix seconds), <the app's own keys>; eframe itself persists the window geometry and egui memory.
- The version comes from `Cargo.toml` (`main::VERSION`, shown in About). Between releases it is the next version with `-dev`, the last number raised by one (after `0.1.0`, `0.1.1-dev`), and `build.rs` appends `git describe --tags --always` (`0.1.1-dev (v0.1.0-2-g4bd494d)`); a release version is shown as is. Release: drop `-dev`, commit `Version X.Y.Z`, tag `vX.Y.Z` (release.yml checks the tag against `Cargo.toml`), then set the next `-dev` version (`X.Y.(Z+1)-dev`).
- The release profile has `panic = "abort"` and no console: a panic shows a message box (`main::install_panic_hook`) and ends the program. `f32::clamp` panics when its bounds cross; sizes read from files may be 0.
- Commits in this repository end with a `Co-Authored-By:` trailer naming the Claude model doing the work, e.g. `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (the user's decision; it overrides the global "no Co-Authored-By" rule for this repo only).
- GUI checks: the OpenGL client area cannot be captured with `PrintWindow` on an NVIDIA machine (the caption can, from a DPI-aware process); keys can be posted with `PostMessage(WM_KEYDOWN/WM_KEYUP)`. A launched window takes the focus: warn the user before launching test windows and before any simulated global mouse or keyboard input, and close test windows with `taskkill /PID` (WM_CLOSE, which saves the settings).
