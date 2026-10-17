//! Generates the application icon and embeds it (plus version info) into the
//! Windows executable, and passes the version shown in the UI to the program.

#[path = "src/filetypes.rs"]
mod filetypes;
#[path = "src/icon.rs"]
mod icon;
#[path = "src/type_icon.rs"]
mod type_icon;

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=src/filetypes.rs");
    println!("cargo:rerun-if-changed=src/type_icon.rs");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=APP_VERSION={}", version());

    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    // The icon as raw RGBA for the window icon, the start page and About
    // (see `main::embedded_icon`), so that nothing is rasterised at start-up.
    for size in [64u32, 128, 256] {
        std::fs::write(
            out_dir.join(format!("app_icon_{size}.rgba")),
            icon::rgba(size),
        )
        .expect("write an icon");
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ico_path = out_dir.join("app.ico");
    std::fs::write(&ico_path, ico_with_png(&icon::rgba)).expect("write app.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        // The icons of the registered file types, referenced by the
        // registry as `"metalamp.exe",-<id>` (assoc.rs). The app icon keeps
        // ID 1, the lowest, which Explorer shows for the executable.
        for (i, t) in filetypes::FILE_TYPES.iter().enumerate() {
            let path = out_dir.join(format!("type_{}.ico", t.id));
            std::fs::write(&path, ico_with_png(&|s| type_icon::rgba(t, s)))
                .expect("write a file type icon");
            res.set_icon_with_id(path.to_str().unwrap(), &filetypes::icon_id(i).to_string());
        }
        res.set_icon(ico_path.to_str().unwrap())
            .set(
                "FileDescription",
                "MetalAMP - audio player with a media library",
            )
            .set("ProductName", "MetalAMP")
            .set("LegalCopyright", "Copyright (C) 2026 Fan4_Metal");
        if let Err(e) = res.compile() {
            // A missing resource compiler must not break the build.
            println!("cargo:warning=icon not embedded: {e}");
        }
    }
}

/// The version shown in the UI: Cargo's, plus the commit for a development
/// version ("0.1.1-dev (v0.1.0-2-g4bd494d)"), so that builds between
/// releases can be told apart. A release version is shown as is.
fn version() -> String {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    if !version.contains('-') {
        return version;
    }
    // `--always` falls back to the abbreviated hash when no tag is reachable
    // (a shallow checkout fetches none). No `--dirty`: this script is not rerun
    // when only sources change, so the mark would be stale.
    match git(&["describe", "--tags", "--always"]) {
        Some(describe) => {
            watch_git();
            format!("{version} ({describe})")
        }
        None => version,
    }
}

/// Reruns this script when a commit is made, the branch is switched or a tag
/// is added: HEAD, the branch it points to, packed refs and the tags.
fn watch_git() {
    let Some(dirs) = git(&["rev-parse", "--git-dir", "--git-common-dir"]) else {
        return;
    };
    let mut dirs = dirs.lines().map(std::path::PathBuf::from);
    let (Some(git_dir), Some(common)) = (dirs.next(), dirs.next()) else {
        return;
    };
    let head = git_dir.join("HEAD");
    let mut paths = vec![
        head.clone(),
        common.join("packed-refs"),
        common.join("refs").join("tags"),
    ];
    if let Some(branch) = std::fs::read_to_string(&head)
        .ok()
        .and_then(|h| h.strip_prefix("ref: ").map(|r| r.trim().to_owned()))
    {
        paths.push(common.join(branch));
    }
    // A path that does not exist would make Cargo rerun the script every time.
    for path in paths.iter().filter(|p| p.exists()) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

/// Output of a git command, trimmed; `None` when git is missing or fails.
fn git(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// An `.ico` of every size in `icon::ICO_SIZES` drawn by `draw`, the 256 px
/// layer as PNG (~50 KB instead of ~260 KB of BMP), as qview stores its.
fn ico_with_png(draw: &dyn Fn(u32) -> Vec<u8>) -> Vec<u8> {
    let layers: Vec<(u32, Vec<u8>)> = icon::ICO_SIZES
        .iter()
        .map(|&s| {
            let px = draw(s);
            let image = if s >= 256 {
                png(&px, s)
            } else {
                icon::bmp(&px, s)
            };
            (s, image)
        })
        .collect();
    icon::ico_of(&layers)
}

/// Straight RGBA pixels `px` of `size` x `size` as a PNG file.
fn png(px: &[u8], size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header().expect("PNG header");
    writer.write_image_data(px).expect("PNG data");
    writer.finish().expect("PNG end");
    out
}
