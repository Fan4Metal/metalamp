//! MetalAMP: a fast audio player for Windows with a Winamp-style media library.

// The release build is a GUI program without a console; the debug build
// keeps it, so `eprintln!`, logs and panics are seen while developing.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// First, so that `tr!` is in scope in the modules below.
#[macro_use]
mod i18n;

mod app;
mod assoc;
mod audio;
mod bench;
mod caption;
mod covers;
mod filetypes;
mod format;
mod frameless;
mod icon;
mod instance;
mod library;
mod palette;
mod player;
mod queue;
mod settings;
mod ui;
mod update;
mod win;

use std::path::PathBuf;
use std::sync::Arc;

/// The display name, as people read it: the window title, About, the
/// panic message, the User-Agent of the update check. The identifier
/// (`metalamp`) is the Cargo package, the exe and the repository.
pub const APP_NAME: &str = "MetalAMP";
/// eframe app id; also names the settings folder in `%APPDATA%` (unless the
/// copy is portable, see `settings::location`).
pub const APP_ID: &str = "MetalAMP";
/// Version from Cargo.toml, shown in About; a development version
/// ("0.1.0-dev") carries the commit it was built from (`build.rs`).
pub const VERSION: &str = env!("APP_VERSION");
/// The window's size in points on the first run (and the restored size of
/// a window first shown maximized).
pub const DEFAULT_SIZE: [f32; 2] = [1280.0, 800.0];
const MIN_SIZE: [f32; 2] = [900.0, 560.0];

fn main() -> eframe::Result {
    env_logger::init();
    install_panic_hook();

    // The command line modes. `args_os`: `args` panics on an argument that
    // is not valid Unicode.
    let mut files = Vec::new();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--export-icon") => {
                // For tools/make_release.py: the installer's icon, then exit.
                win::attach_parent_console();
                let out = args.next().unwrap_or_else(|| "app.ico".into());
                if let Err(e) = std::fs::write(&out, icon::ico(&icon::ICO_SIZES)) {
                    eprintln!("cannot write {}: {e}", out.to_string_lossy());
                    std::process::exit(1);
                }
                return Ok(());
            }
            Some("-V" | "--version") => {
                win::attach_parent_console();
                println!("{APP_NAME} {VERSION}");
                return Ok(());
            }
            Some(flag @ ("--register" | "--unregister")) => {
                // For the installer: register or unregister the file types,
                // then exit; an error is shown (the installer runs it
                // hidden). The types are named in the language of Windows.
                i18n::set_lang(i18n::system_lang());
                let result = if flag == "--register" {
                    std::env::current_exe()
                        .map_err(|e| e.to_string())
                        .and_then(|exe| assoc::register(&exe))
                } else {
                    // Uninstalling this copy leaves another's registration
                    // (a portable copy registered since) alone.
                    match std::env::current_exe().map(|exe| assoc::status(&exe)) {
                        Ok(assoc::Status::OtherCopy(other)) => {
                            log::info!("the file types are another copy's ({other}): kept");
                            Ok(())
                        }
                        _ => assoc::unregister(),
                    }
                };
                if let Err(e) = &result {
                    log::error!("{flag}: {e}");
                    win::error_box(APP_NAME, e);
                    std::process::exit(1);
                }
                return Ok(());
            }
            Some("--bench") => {
                win::attach_parent_console();
                let rest: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
                std::process::exit(bench::run(&rest));
            }
            Some("-h" | "--help") => {
                win::attach_parent_console();
                println!("{APP_NAME} {VERSION}");
                println!("metalamp [FILE...]");
                println!("metalamp --version | --help | --export-icon FILE.ico");
                println!(
                    "metalamp --register | --unregister   (the file types, for the installer)"
                );
                println!("metalamp --bench scan FOLDER... | --bench gapless FILE...");
                return Ok(());
            }
            _ => {
                let path = normalize(&arg.to_string_lossy());
                if !path.as_os_str().is_empty() {
                    files.push(path);
                }
            }
        }
    }

    // A copy already running from this exe takes the files and comes
    // forward; this one ends.
    if instance::forward(&files) {
        return Ok(());
    }

    let settings = settings::Settings::load();
    // The index is read and the folders checked while eframe makes the
    // window (~150 ms), so the first frame can show the library.
    let wake = library::scan::Wake::default();
    let scan = library::scan::Scan::start(
        settings.library_folders.clone(),
        None,
        true,
        std::sync::Arc::clone(&wake),
    );
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            // The title row is the app's own (`frameless`, `ui/title.rs`).
            .with_decorations(false)
            .with_resizable(true)
            .with_drag_and_drop(true)
            .with_inner_size(DEFAULT_SIZE)
            .with_min_inner_size(MIN_SIZE)
            .with_icon(window_icon()),
        // OpenGL starts fastest: a context in ~130 ms against ~240 ms for
        // wgpu on DX12 (measured in metalmd).
        renderer: eframe::Renderer::Glow,
        // Centre only on the first run; later runs put the window where it
        // was (`App::new`, `win::place`).
        centered: settings.window.is_none(),
        ..Default::default()
    };
    eframe::run_native(
        APP_ID,
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc, settings, scan, wake, files)))),
    )
}

/// An argument as a path: Explorer passes a drive's root as `"C:\"`, which
/// turns into `C:"`; the trailing quote is turned back into a backslash. A
/// bare `C:` means the root, not the current directory of C. Relative paths
/// become absolute (the running copy may have another current directory).
/// metalmpv's.
fn normalize(arg: &str) -> PathBuf {
    let arg = match arg.strip_suffix('"') {
        // A lone quote is not a drive's root (it became the current one).
        Some(stripped) if stripped.trim_end_matches('\\').is_empty() => return PathBuf::new(),
        Some(stripped) => format!("{}\\", stripped.trim_end_matches('\\')),
        None => arg.to_string(),
    };
    let b = arg.as_bytes();
    let path = if b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        PathBuf::from(format!("{arg}\\"))
    } else {
        PathBuf::from(arg)
    };
    std::path::absolute(&path).unwrap_or(path)
}

/// The icon of the window, from the embedded 64-pixel rendering.
pub fn window_icon() -> Arc<egui::IconData> {
    Arc::new(egui::IconData {
        rgba: embedded_icon(64).to_vec(),
        width: 64,
        height: 64,
    })
}

/// The app icon rasterised at build time (`build.rs`) as straight RGBA,
/// `size` x `size` pixels for 64, 128 and 256, so that nothing is drawn at
/// start-up (drawing it took 6 ms before the window could be created).
pub fn embedded_icon(size: u32) -> &'static [u8] {
    match size {
        64 => include_bytes!(concat!(env!("OUT_DIR"), "/app_icon_64.rgba")),
        128 => include_bytes!(concat!(env!("OUT_DIR"), "/app_icon_128.rgba")),
        _ => include_bytes!(concat!(env!("OUT_DIR"), "/app_icon_256.rgba")),
    }
}

/// The release build aborts on a panic and has no console, so the window
/// would vanish without a word: say what happened in a message box. The
/// default hook still prints it (seen in a debug build). A stack overflow
/// is not a panic and still ends the process silently. disk_flashlight also
/// appends the report to `crash.log` next to the settings file.
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let text = tr!(
            format!("{APP_NAME} stopped because of an internal error.\n\n{info}"),
            format!("{APP_NAME} остановлен из-за внутренней ошибки.\n\n{info}")
        );
        win::error_box(APP_NAME, &text);
    }));
}
