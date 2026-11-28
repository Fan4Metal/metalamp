# CLAUDE.md

Guidance for Claude Code in this repository. The details are in `docs/dev/`; read the part that concerns the task before starting it, and keep it current when the code changes.

## Project

MetalAMP is a Windows-only audio player in Rust with egui/eframe 0.36 (glow), built around a media library: artists, a grid of album covers, tracks, the player bar on top, the playlist on the right. Media Foundation decodes (MP3, AAC/M4A, FLAC, WMA, WAV), WASAPI plays. Priorities: start-up speed, a library shown at once and smooth with thousands of tracks, gapless playback, no external runtime or DLLs. The plan agreed with the user is `local/PLAN.md`, deferred work `local/TODO.md` (neither in the repository). `README.md` / `README.ru.md` are the user-facing docs: kept in sync, both languages, cross-linked, formal tone, no imperatives. `docs/enhancer.md` / `docs/enhancer.ru.md` change with the enhancer's law, its measurements or its output stage.

The name is `MetalAMP` wherever people read it (UI, window title, READMEs, installer, folders) and `metalamp` wherever it is an identifier (Cargo package, `metalamp.exe`, the GitHub repo `Fan4Metal/metalamp`).

## Where the details are

- `docs/dev/architecture.md`: every module, what it does and why, with measurements. Read a module's entry before changing it; update the entry with the change.
- `docs/dev/testing.md`: all commands, benches, ignored tests with real files and devices, GUI checks.
- `docs/dev/settings.md`: every key of `settings.json`; a new setting is added there.
- `docs/dev/keys.md`: the default keys.
- `docs/dev/conventions.md`: the conventions below in full, the toolchain and the release steps.

Map: `main.rs` (arguments, start), `app.rs` (the eframe `App`), `settings.rs`, `palette.rs`, `win.rs` (Win32 helpers), `library/` (scan, index, tags, edit, prepare, filter, watch), `audio/` (engine, decoder, output, `dsp/`), `ui/` (panels, windows, visualizations), `player.rs` + `queue.rs` (the playlist and the engine), `covers.rs`, `stats.rs`, `loudness.rs`, `lyrics.rs`.

## Commands

```
uv run build.py                  # closes a metalamp.exe running from target\, then cargo build --release
cargo test                       # unit tests
cargo clippy --all-targets       # must stay warning-free
python tools/make_release.py --no-tests --install   # release build, installer, installs over the installed copy
python tools/notices.py          # after the dependencies change (a test fails until then)
RUST_LOG=metalamp=debug          # debug log
```

Toolchain: stable MSVC Rust plus VS 2022 Build Tools. In the Bash tool `cargo` is not on PATH: prefix `export PATH="$HOME/.cargo/bin:$PATH"`. Write source files with the Write tool or a Python script, not heredocs (Rust's apostrophes break the Bash tool). The crate is not rustfmt-clean: no `cargo fmt`.

Trying a change: `cargo test` and `cargo clippy --all-targets`, then `uv run build.py` and run `target\release\metalamp.exe`; a test that plays sound lowers `volume` in `settings.json` first and puts it back. Then `make_release.py --no-tests --install` (it closes the running copy, which saves its settings).

Releases: a pushed tag `vX.Y.Z` publishes a public release: only when the user asks. Between releases the version is the next one with `-dev` (details in `conventions.md`).

## Conventions

- Every user-visible string goes through `tr!("English", "Русский")`; logs, CLI output and technical errors stay English. Nothing caches translated text.
- Settings are `#[serde(default)]` and read field by field, so a new field needs a default and nothing else.
- A right-button menu on a widget with a tooltip gets an id of its own; inside a modal it is also made the modal's sublayer each frame (why: `conventions.md`).
- Parsers of the user's files (lofty, covers) run under `guard::catch`. `f32::clamp` panics when its bounds cross; sizes read from files may be 0.
- The default fonts lack many symbols (`→`, `▾`): use `»`, words, or a painted icon from `ui/icons.rs`.
- Commits end with `Co-Authored-By: Claude <model> <noreply@anthropic.com>` (the user's decision for this repo).
- GUI checks: warn the user before launching a window (it takes the focus); never simulate input; capture with `CopyFromScreen` from a DPI-aware process; use a portable test copy (`metalamp.json` next to `target\release\metalamp.exe`) and remove it and its data after. Details and the `METALAMP_*` switches: `testing.md`.
