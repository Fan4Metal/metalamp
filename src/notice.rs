//! The track notice, Winamp's: a small card by the notification area
//! naming each track as it starts (its place in the playlist, the title
//! with its length, the artist, the album) with its cover, for a few
//! seconds (`settings.track_notice`, `track_notice_seconds`).
//!
//! A window of its own on a "notice" thread, not an egui viewport: a
//! minimized eframe window runs a frame only when one is due, which would
//! make the fades stutter. The card is laid out with egui and drawn by
//! hand (`ui::raster`) at the primary monitor's density, its rounded
//! corners and shadow put on as the alpha of a layered window
//! (`UpdateLayeredWindow`, premultiplied BGRA); it slides in, stays, and
//! fades out (`SourceConstantAlpha`). It never takes the focus
//! (`WS_EX_NOACTIVATE`); the pointer over it keeps it, a click brings the
//! player's window back (`win::hook_show`), the right button sends it
//! away. A notice while another shows replaces it in place.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use egui::epaint::RectShape;
use egui::{Color32, ColorImage, FontId, Rect, Stroke, StrokeKind, Vec2, pos2, vec2};

use crate::library::Track;
use crate::palette;
use crate::settings::CoverPriority;
use crate::ui::raster::{Picture, Renderer};

/// The card, without its shadow (points).
const CARD: Vec2 = vec2(380.0, 104.0);
/// The room round the card for its shadow (points).
const SHADOW: f32 = 16.0;
/// How far the shadow falls under the card (points).
const SHADOW_DROP: f32 = 3.0;
const RADIUS: f32 = 10.0;
const COVER_RADIUS: f32 = 6.0;
const PAD: f32 = 12.0;
/// The cover's side (points): the card's height less its padding.
const COVER: f32 = CARD.y - 2.0 * PAD;
/// The card from the work area's edges (points).
const MARGIN: f32 = 12.0;
/// How far it slides in (points).
const SLIDE: f32 = 14.0;
const IN_S: f32 = 0.22;
const OUT_S: f32 = 0.5;
const TICK_MS: u32 = 15;

/// The thread's wake-up: something came down the channel.
const WM_NOTICE: u32 = 0x8000 + 0x52;
const TIMER: usize = 1;

/// A track starting, as the app knows it.
pub struct Notice {
    pub path: PathBuf,
    /// The library's track (or the file's as read); `None`: its tags are
    /// read on the notice's thread.
    pub track: Option<Track>,
    /// Its place in the playlist and the playlist's length.
    pub place: Option<(usize, usize)>,
    /// The length as the player has it (0: not known yet).
    pub length_ms: u64,
    pub priority: CoverPriority,
    /// How long it stays before it fades.
    pub seconds: u32,
}

enum Msg {
    Show(Box<Notice>),
}

/// The app's end: the thread started with the first notice.
#[derive(Default)]
pub struct Notifier {
    tx: Option<Sender<Msg>>,
    hwnd: Arc<AtomicIsize>,
}

impl Notifier {
    pub fn show(&mut self, notice: Notice) {
        if self.tx.is_none() {
            let (tx, rx) = channel();
            let hwnd = self.hwnd.clone();
            let spawned = std::thread::Builder::new()
                .name("notice".into())
                .spawn(move || run(rx, hwnd));
            if let Err(e) = spawned {
                log::warn!("notice thread: {e}");
                return;
            }
            self.tx = Some(tx);
        }
        let Some(tx) = &self.tx else { return };
        if tx.send(Msg::Show(Box::new(notice))).is_err() {
            // The thread could not make its window: no notices this run.
            return;
        }
        let hwnd = self.hwnd.load(Relaxed);
        if hwnd != 0 {
            // SAFETY: the notice's window, alive as long as its thread.
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    hwnd as _, WM_NOTICE, 0, 0,
                )
            };
        }
    }
}

/// Whether Windows lets a notice show now: not over a full-screen
/// program, a presentation or in quiet hours.
pub fn may_show() -> bool {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHQueryUserNotificationState(state: *mut i32) -> i32;
    }
    let mut state = 0;
    // SAFETY: an out parameter of ours.
    if unsafe { SHQueryUserNotificationState(&mut state) } != 0 {
        return true;
    }
    // QUNS_BUSY, QUNS_RUNNING_D3D_FULL_SCREEN, QUNS_PRESENTATION_MODE,
    // QUNS_QUIET_TIME.
    !matches!(state, 2 | 3 | 4 | 6)
}

/// What the card says.
#[derive(Clone, Debug, PartialEq)]
struct Card {
    place: String,
    heading: String,
    title: String,
    length: String,
    artist: String,
    album: String,
}

impl Card {
    fn of(notice: &Notice, track: Option<&Track>) -> Card {
        let place = match notice.place {
            Some((i, n)) => format!("{i} {} {n}", tr!("of", "из")),
            None => "MetalAMP".to_owned(),
        };
        let stem = || {
            notice
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let length_ms = match notice.length_ms {
            0 => track.map_or(0, |t| u64::from(t.duration_ms)),
            ms => ms,
        };
        let (title, artist, album) = match track {
            Some(t) => {
                let title = if t.title.is_empty() { stem() } else { t.title.clone() };
                let artist = if t.artist.is_empty() {
                    crate::library::unknown_artist().to_owned()
                } else {
                    t.artist.clone()
                };
                let mut parts = Vec::new();
                if !t.album.is_empty() {
                    parts.push(t.album.clone());
                }
                if t.year > 0 {
                    parts.push(t.year.to_string());
                }
                if t.track > 0 {
                    parts.push(format!("{} {}", tr!("track", "трек"), t.track));
                }
                (title, artist, parts.join(" · "))
            }
            None => (stem(), String::new(), String::new()),
        };
        Card {
            place,
            heading: tr!("Now playing", "Сейчас играет").to_owned(),
            title,
            length: if length_ms > 0 {
                crate::format::time(length_ms)
            } else {
                String::new()
            },
            artist,
            album,
        }
    }
}

/// A picture for a layered window: premultiplied BGRA, top row first.
struct Layered {
    size: [usize; 2],
    bgra: Vec<u8>,
}

/// The card's picture at `scale` pixels a point, with its shadow.
fn render(renderer: &mut Renderer, scale: f32, card: &Card, cover: Option<ColorImage>) -> Layered {
    let full = CARD + Vec2::splat(2.0 * SHADOW);
    let [w, h] = renderer.pixels(full);
    let shadow = (SHADOW * scale).round();
    // The card on whole pixels.
    let card_px = [shadow, shadow, w as f32 - shadow, h as f32 - shadow];
    let at = |px: f32| px / scale;
    let card_rect = Rect::from_min_max(
        pos2(at(card_px[0]), at(card_px[1])),
        pos2(at(card_px[2]), at(card_px[3])),
    );
    let ground = palette::panel_bg();
    let side = (COVER * scale).round();
    let cover_rect = Rect::from_min_size(
        pos2(
            at((card_px[0] + PAD * scale).round()),
            at((card_px[1] + PAD * scale).round()),
        ),
        Vec2::splat(at(side)),
    );
    // Loaded before the frame and kept past it: the raster takes the
    // textures egui hands over at the frame's end.
    let texture = cover.map(|c| {
        renderer
            .ctx
            .load_texture("notice cover", c, egui::TextureOptions::LINEAR)
    });
    let rect = Rect::from_min_size(egui::Pos2::ZERO, full);
    let picture = renderer.frame(rect, ground, |p| {
        p.rect_filled(card_rect, RADIUS, ground);
        p.rect_stroke(
            card_rect,
            RADIUS,
            Stroke::new(1.0 / scale, palette::line()),
            StrokeKind::Inside,
        );
        match &texture {
            Some(t) => {
                p.add(
                    RectShape::filled(cover_rect, COVER_RADIUS, Color32::WHITE).with_texture(
                        t.id(),
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    ),
                );
            }
            None => {
                let well = palette::list_bg();
                p.rect_filled(cover_rect, COVER_RADIUS, well);
                let mark = well.lerp_to_gamma(palette::text_weak(), 0.3);
                crate::ui::icons::paint(
                    p,
                    crate::ui::icons::Icon::Notes,
                    cover_rect.center(),
                    cover_rect.width() * 0.5,
                    mark,
                    well,
                );
            }
        }
        let x = cover_rect.right() + PAD;
        let right = card_rect.right() - PAD;
        let width = right - x;
        let top = card_rect.top() + PAD + 3.0;
        // The place on the left, what happened on the right.
        let small = FontId::proportional(11.5);
        let heading = p.layout_no_wrap(card.heading.clone(), small.clone(), palette::text_weak());
        let heading_w = heading.size().x;
        p.galley(pos2(right - heading_w, top - 1.0), heading, palette::text_weak());
        let place = crate::ui::elided(
            p,
            card.place.clone(),
            small,
            palette::accent(),
            width - heading_w - 8.0,
        );
        p.galley(pos2(x, top - 1.0), place, palette::accent());
        // The title with its length after it, on one base line.
        let length = p.layout_no_wrap(card.length.clone(), FontId::proportional(13.0), palette::text_weak());
        let length_w = if card.length.is_empty() { 0.0 } else { length.size().x + 8.0 };
        let title = crate::ui::elided(
            p,
            card.title.clone(),
            FontId::proportional(16.5),
            palette::text_bright(),
            width - length_w,
        );
        let title_y = top + 16.0;
        let title_size = title.size();
        if !card.length.is_empty() {
            let y = title_y + title_size.y - length.size().y - 1.0;
            p.galley(pos2(x + title_size.x + 8.0, y), length, palette::text_weak());
        }
        p.galley(pos2(x, title_y), title, palette::text_bright());
        let artist = crate::ui::elided(
            p,
            card.artist.clone(),
            FontId::proportional(13.5),
            palette::text(),
            width,
        );
        p.galley(pos2(x, top + 41.0), artist, palette::text());
        let album = crate::ui::elided(
            p,
            card.album.clone(),
            FontId::proportional(12.5),
            palette::text_weak(),
            width,
        );
        p.galley(pos2(x, top + 61.0), album, palette::text_weak());
    });
    drop(texture);
    let dark = palette::current().dark;
    let bgra = layered(
        &picture,
        [w, h],
        card_px,
        RADIUS * scale,
        shadow,
        SHADOW_DROP * scale,
        if dark { 0.55 } else { 0.28 },
    );
    Layered { size: [w, h], bgra }
}

/// The signed distance from `(x, y)` to a rectangle `r` (left, top,
/// right, bottom) with corners of `radius`: negative inside.
fn rounded_distance(x: f32, y: f32, r: [f32; 4], radius: f32) -> f32 {
    let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
    let (hx, hy) = ((r[2] - r[0]) / 2.0 - radius, (r[3] - r[1]) / 2.0 - radius);
    let (qx, qy) = ((x - cx).abs() - hx, (y - cy).abs() - hy);
    let outside = qx.max(0.0).hypot(qy.max(0.0));
    outside + qx.max(qy).min(0.0) - radius
}

/// `picture` cut to the rounded card and laid over its soft shadow, as a
/// layered window's premultiplied BGRA.
fn layered(
    picture: &Picture,
    [w, h]: [usize; 2],
    card: [f32; 4],
    radius: f32,
    blur: f32,
    drop: f32,
    darkness: f32,
) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    let shadow_card = [card[0], card[1] + drop, card[2], card[3] + drop];
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let cover = (0.5 - rounded_distance(px, py, card, radius)).clamp(0.0, 1.0);
            let d = rounded_distance(px, py, shadow_card, radius + blur * 0.5) + blur * 0.5;
            let shade = darkness * (1.0 - (d / blur).clamp(0.0, 1.0)).powi(2);
            let alpha = cover + shade * (1.0 - cover);
            let c = picture[y * w + x];
            let i = (y * w + x) * 4;
            let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            out[i] = byte(c[2] * cover);
            out[i + 1] = byte(c[1] * cover);
            out[i + 2] = byte(c[0] * cover);
            out[i + 3] = byte(alpha);
        }
    }
    out
}

/// The cover read for the card: its middle square at `side` pixels.
fn cover_of(notice: &Notice, side: u32) -> Option<ColorImage> {
    let path = &notice.path;
    let image = crate::guard::catch(path, || {
        crate::covers::large(path, side.max(256) * 2, notice.priority)
    })
    .flatten()?;
    let [w, h] = image.size;
    let n = w.min(h);
    if n == 0 {
        return None;
    }
    let (x0, y0) = ((w - n) / 2, (h - n) / 2);
    let mut rgba = Vec::with_capacity(n * n * 4);
    for y in y0..y0 + n {
        for x in x0..x0 + n {
            rgba.extend_from_slice(&image.pixels[y * w + x].to_srgba_unmultiplied());
        }
    }
    let square = image::RgbaImage::from_raw(n as u32, n as u32, rgba)?;
    let small = image::imageops::resize(&square, side, side, image::imageops::FilterType::CatmullRom);
    Some(ColorImage::from_rgba_unmultiplied(
        [side as usize, side as usize],
        small.as_raw(),
    ))
}

/// Where the window goes (its top left, pixels) on a monitor of `monitor`
/// with the work area `work`, `size` pixels large, its card `margin` from
/// the edges; and the way it slides in: 1 up from below, −1 down from
/// above (the taskbar at the top).
fn corner(monitor: [i32; 4], work: [i32; 4], size: [i32; 2], margin: i32) -> ([i32; 2], i32) {
    let x = work[2] - margin - size[0];
    let taskbar_on_top = work[1] > monitor[1] && work[3] >= monitor[3];
    if taskbar_on_top {
        ([x, work[1] + margin], -1)
    } else {
        ([x, work[3] - margin - size[1]], 1)
    }
}

/// The primary monitor's rectangle, work area and pixels a point.
fn screen() -> Option<([i32; 4], [i32; 4], f32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
    };
    #[link(name = "shcore")]
    unsafe extern "system" {
        fn GetDpiForMonitor(monitor: isize, kind: i32, x: *mut u32, y: *mut u32) -> i32;
    }
    // SAFETY: plain queries with out parameters of ours.
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let (mut dx, mut dy) = (96, 96);
        // MDT_EFFECTIVE_DPI
        if GetDpiForMonitor(monitor as isize, 0, &mut dx, &mut dy) != 0 || dx == 0 {
            dx = 96;
        }
        let r = |r: windows_sys::Win32::Foundation::RECT| [r.left, r.top, r.right, r.bottom];
        Some((r(info.rcMonitor), r(info.rcWork), dx as f32 / 96.0))
    }
}

/// Where a shown card is in its life.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    In(Instant),
    Held(Instant),
    Out(Instant),
}

/// How a phase looks `elapsed` into it: the opacity and the share of the
/// slide still to go; `None` once a fade out is done.
fn look(phase: Phase, now: Instant) -> Option<(f32, f32)> {
    let ease = |t: f32| 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3);
    match phase {
        Phase::In(start) => {
            let e = ease(now.duration_since(start).as_secs_f32() / IN_S);
            Some((e, 1.0 - e))
        }
        Phase::Held(_) => Some((1.0, 0.0)),
        Phase::Out(start) => {
            let t = now.duration_since(start).as_secs_f32() / OUT_S;
            (t < 1.0).then(|| {
                let t = t.clamp(0.0, 1.0);
                (1.0 - t * t * (3.0 - 2.0 * t), 0.0)
            })
        }
    }
}

/// The DIB the window shows, selected into a memory DC.
struct Bitmap {
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    dib: windows_sys::Win32::Graphics::Gdi::HBITMAP,
    old: windows_sys::Win32::Graphics::Gdi::HGDIOBJ,
    size: [i32; 2],
}

impl Bitmap {
    fn of(picture: &Layered) -> Option<Bitmap> {
        use windows_sys::Win32::Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, SelectObject,
        };
        let [w, h] = picture.size;
        // SAFETY: a DC and a DIB of ours, the bits written within the
        // size asked for.
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            if dc.is_null() {
                return None;
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits = std::ptr::null_mut();
            let dib = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if dib.is_null() || bits.is_null() {
                DeleteDC(dc);
                return None;
            }
            std::ptr::copy_nonoverlapping(picture.bgra.as_ptr(), bits as *mut u8, w * h * 4);
            let old = SelectObject(dc, dib);
            Some(Bitmap {
                dc,
                dib,
                old,
                size: [w as i32, h as i32],
            })
        }
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        use windows_sys::Win32::Graphics::Gdi::{DeleteDC, DeleteObject, SelectObject};
        // SAFETY: made in `of`, let go once.
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.dib);
            DeleteDC(self.dc);
        }
    }
}

struct Shown {
    bitmap: Bitmap,
    /// The top left where it rests (pixels).
    at: [i32; 2],
    /// The slide's length and way (pixels; positive: from below).
    slide: i32,
    phase: Phase,
    hold: Duration,
}

struct State {
    hwnd: isize,
    rx: Receiver<Msg>,
    /// The renderer at the density it was made for.
    renderer: Option<(f32, Renderer)>,
    shown: Option<Shown>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// The notice's thread: its window and message loop.
fn run(rx: Receiver<Msg>, out: Arc<AtomicIsize>) {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, GetMessageW, IDC_HAND, LoadCursorW, MSG,
        RegisterClassW, TranslateMessage, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    };
    let class = crate::win::wide("MetalAMP.Notice");
    // SAFETY: a class and a window of this thread, its messages pumped
    // here.
    let hwnd = unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_HAND),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            log::warn!("notice: RegisterClassW failed: {}", std::io::Error::last_os_error());
            return;
        }
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        log::warn!("notice: CreateWindowExW failed: {}", std::io::Error::last_os_error());
        return;
    }
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            hwnd: hwnd as isize,
            rx,
            renderer: None,
            shown: None,
        })
    });
    out.store(hwnd as isize, Relaxed);
    // What was sent before the window was there.
    with_state(take);
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: the thread's own message loop.
    while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn with_state(f: impl FnOnce(&mut State)) {
    STATE.with(|s| {
        // Not while it is borrowed: a message Windows sends from within
        // a call made while handling another.
        if let Ok(mut s) = s.try_borrow_mut()
            && let Some(state) = s.as_mut()
        {
            f(state);
        }
    });
}

unsafe extern "system" fn window_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, MA_NOACTIVATE, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_RBUTTONUP, WM_TIMER,
    };
    match msg {
        WM_NOTICE => with_state(take),
        WM_TIMER if wparam == TIMER => with_state(tick),
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as _,
        WM_LBUTTONUP => {
            crate::win::hook_show();
            with_state(hide);
        }
        WM_RBUTTONUP => with_state(|s| {
            if let Some(shown) = &mut s.shown
                && !matches!(shown.phase, Phase::Out(_))
            {
                shown.phase = Phase::Out(Instant::now());
            }
        }),
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    0
}

/// The notices sent: the last one shown.
fn take(state: &mut State) {
    let mut last = None;
    while let Ok(Msg::Show(n)) = state.rx.try_recv() {
        last = Some(n);
    }
    if let Some(notice) = last {
        show(state, &notice);
    }
}

fn show(state: &mut State, notice: &Notice) {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetTimer;
    let Some((monitor, work, scale)) = screen() else { return };
    let read;
    let track = match &notice.track {
        Some(t) => Some(t),
        None => {
            read = crate::guard::catch(&notice.path, || {
                crate::library::outside::read_track(&notice.path)
            })
            .flatten();
            read.as_ref()
        }
    };
    let card = Card::of(notice, track);
    let cover = cover_of(notice, (COVER * scale).round() as u32);
    if state.renderer.as_ref().is_none_or(|(s, _)| *s != scale) {
        state.renderer = Some((scale, Renderer::new(scale)));
    }
    let Some((_, renderer)) = &mut state.renderer else { return };
    let picture = render(renderer, scale, &card, cover);
    let Some(bitmap) = Bitmap::of(&picture) else { return };
    let shadow = (SHADOW * scale).round() as i32;
    let margin = (MARGIN * scale).round() as i32 - shadow;
    let (at, way) = corner(monitor, work, bitmap.size, margin);
    let now = Instant::now();
    // Another while one shows: in its place at once, held anew.
    let phase = match &state.shown {
        Some(s) if !matches!(s.phase, Phase::Out(_)) => Phase::Held(now),
        _ => Phase::In(now),
    };
    state.shown = Some(Shown {
        bitmap,
        at,
        slide: way * (SLIDE * scale).round() as i32,
        phase,
        hold: Duration::from_secs(u64::from(notice.seconds.max(1))),
    });
    draw(state, true);
    // SAFETY: a timer of the notice's window.
    unsafe { SetTimer(state.hwnd as _, TIMER, TICK_MS, None) };
}

/// The window drawn as its phase looks now; it hides once faded out.
fn draw(state: &mut State, raise: bool) {
    use windows_sys::Win32::Foundation::{POINT, SIZE};
    use windows_sys::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
        ULW_ALPHA, UpdateLayeredWindow,
    };
    let Some(shown) = &state.shown else { return };
    let Some((opacity, slide)) = look(shown.phase, Instant::now()) else {
        hide(state);
        return;
    };
    let hwnd = state.hwnd as _;
    let dst = POINT {
        x: shown.at[0],
        y: shown.at[1] + (shown.slide as f32 * slide).round() as i32,
    };
    let size = SIZE {
        cx: shown.bitmap.size[0],
        cy: shown.bitmap.size[1],
    };
    let src = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: (opacity * 255.0).round() as u8,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    // SAFETY: the notice's window and its bitmap's DC.
    unsafe {
        UpdateLayeredWindow(
            hwnd,
            std::ptr::null_mut(),
            &dst,
            &size,
            shown.bitmap.dc,
            &src,
            0,
            &blend,
            ULW_ALPHA,
        );
        if raise {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
        }
    }
}

fn hide(state: &mut State) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, SW_HIDE, ShowWindow};
    // SAFETY: the notice's window and timer.
    unsafe {
        KillTimer(state.hwnd as _, TIMER);
        ShowWindow(state.hwnd as _, SW_HIDE);
    }
    state.shown = None;
}

/// Whether the pointer is over the window.
fn pointer_over(hwnd: isize) -> bool {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: an out parameter of ours.
    if unsafe { GetCursorPos(&mut p) } == 0 {
        return false;
    }
    crate::win::window_rect(hwnd)
        .is_some_and(|r| p.x >= r[0] && p.x < r[2] && p.y >= r[1] && p.y < r[3])
}

fn tick(state: &mut State) {
    let over = pointer_over(state.hwnd);
    let Some(shown) = &mut state.shown else {
        hide(state);
        return;
    };
    let now = Instant::now();
    match shown.phase {
        Phase::In(start) if now.duration_since(start).as_secs_f32() >= IN_S => {
            shown.phase = Phase::Held(now);
        }
        Phase::Held(_) if over => shown.phase = Phase::Held(now),
        Phase::Held(since) if now.duration_since(since) >= shown.hold => {
            shown.phase = Phase::Out(now);
        }
        // Held back as it fades.
        Phase::Out(_) if over => shown.phase = Phase::Held(now),
        Phase::Held(_) => return,
        _ => {}
    }
    draw(state, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(track: Option<Track>) -> Notice {
        Notice {
            path: PathBuf::from(r"D:\Music\Lacrimosa\(1995) Inferno\04 - No Blind Eyes Can See.mp3"),
            track,
            place: Some((4, 8)),
            length_ms: 556_000,
            priority: CoverPriority::Tags,
            seconds: 4,
        }
    }

    fn track() -> Track {
        Track {
            title: "No Blind Eyes Can See".into(),
            artist: "Lacrimosa".into(),
            album: "Inferno".into(),
            year: 1995,
            track: 4,
            ..Default::default()
        }
    }

    #[test]
    fn the_card_says_what_plays() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let t = track();
        let card = Card::of(&notice(Some(t.clone())), Some(&t));
        assert_eq!(card.place, "4 of 8");
        assert_eq!(card.title, "No Blind Eyes Can See");
        assert_eq!(card.length, "9:16");
        assert_eq!(card.artist, "Lacrimosa");
        assert_eq!(card.album, "Inferno · 1995 · track 4");
        // Without tags: the file's name.
        let card = Card::of(&notice(None), None);
        assert_eq!(card.title, "04 - No Blind Eyes Can See");
        assert_eq!(card.album, "");
    }

    #[test]
    fn the_card_goes_by_the_taskbar() {
        let monitor = [0, 0, 1920, 1080];
        // At the bottom: the bottom right corner, sliding up.
        let (at, way) = corner(monitor, [0, 0, 1920, 1040], [400, 120], 10);
        assert_eq!((at, way), ([1510, 910], 1));
        // At the top: the top right corner, sliding down.
        let (at, way) = corner(monitor, [0, 40, 1920, 1080], [400, 120], 10);
        assert_eq!((at, way), ([1510, 50], -1));
        // On the right: by it, at the bottom.
        let (at, _) = corner(monitor, [0, 0, 1860, 1080], [400, 120], 10);
        assert_eq!(at, [1450, 950]);
    }

    #[test]
    fn it_slides_in_stays_and_fades() {
        let start = Instant::now();
        let at = |s: f32| start + Duration::from_secs_f32(s);
        let (o, slide) = look(Phase::In(start), start).unwrap();
        assert_eq!((o, slide), (0.0, 1.0));
        let (o, slide) = look(Phase::In(start), at(IN_S)).unwrap();
        assert!((o - 1.0).abs() < 1e-6 && slide.abs() < 1e-6);
        assert_eq!(look(Phase::Held(start), at(100.0)), Some((1.0, 0.0)));
        let (o, _) = look(Phase::Out(start), at(OUT_S / 2.0)).unwrap();
        assert!((o - 0.5).abs() < 1e-3, "{o}");
        assert_eq!(look(Phase::Out(start), at(OUT_S)), None);
    }

    #[test]
    fn the_shadow_falls_round_a_rounded_card() {
        let r = [10.0, 10.0, 90.0, 50.0];
        assert!(rounded_distance(50.0, 30.0, r, 6.0) < -10.0);
        assert!((rounded_distance(50.0, 10.0, r, 6.0)).abs() < 1e-4);
        // The corner is cut: its very point lies outside.
        assert!(rounded_distance(10.5, 10.5, r, 6.0) > 0.0);
        let picture = vec![[1.0, 1.0, 1.0]; 100 * 60];
        let out = layered(&picture, [100, 60], r, 6.0, 8.0, 2.0, 0.5);
        let px = |x: usize, y: usize| &out[(y * 100 + x) * 4..(y * 100 + x) * 4 + 4];
        // Inside: the card, opaque.
        assert_eq!(px(50, 30), [255, 255, 255, 255]);
        // Under it: shadow, black and partly clear.
        let below = px(50, 52);
        assert_eq!(&below[..3], [0, 0, 0]);
        assert!(below[3] > 0 && below[3] < 128, "{below:?}");
        // Far off: nothing.
        assert_eq!(px(0, 0), [0, 0, 0, 0]);
    }

    /// The notice's window on the screen for a few seconds, two notices
    /// one after another (`METALAMP_SAMPLE` for a cover); it takes no
    /// focus.
    #[test]
    #[ignore]
    fn notice_on_screen() {
        let mut notifier = Notifier::default();
        let sample = std::env::var_os("METALAMP_SAMPLE").map(PathBuf::from);
        let mut first = notice(Some(track()));
        if let Some(p) = &sample {
            first.path = p.clone();
        }
        notifier.show(first);
        std::thread::sleep(Duration::from_millis(2500));
        let mut second = notice(None);
        second.place = Some((5, 8));
        second.seconds = 2;
        notifier.show(second);
        std::thread::sleep(Duration::from_millis(3500));
    }

    /// The card drawn into a PNG to look at (`METALAMP_FRAMES=<file.png>`,
    /// `METALAMP_THEME=light`), on a checked ground for its alpha.
    #[test]
    #[ignore]
    fn notice_card() {
        let Ok(out) = std::env::var("METALAMP_FRAMES") else { return };
        crate::i18n::set_lang(crate::i18n::Lang::Ru);
        if std::env::var("METALAMP_THEME").as_deref() == Ok("light") {
            palette::set(palette::Look {
                theme: palette::Theme::Light,
                ..Default::default()
            });
        }
        let scale = 1.5;
        let mut renderer = Renderer::new(scale);
        let mut t = track();
        t.title = "Kabinett der Sinne (Ein Lied von Liebe und Tod)".into();
        let card = Card::of(&notice(Some(t.clone())), Some(&t));
        let cover = std::env::var_os("METALAMP_SAMPLE").and_then(|p| {
            let mut n = notice(None);
            n.path = PathBuf::from(p);
            cover_of(&n, (COVER * scale).round() as u32)
        });
        let picture = render(&mut renderer, scale, &card, cover);
        let [w, h] = picture.size;
        // Over a checkerboard, as the window would lie over the desktop.
        let mut rgb = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let ground = if (x / 8 + y / 8) % 2 == 0 { 200.0 } else { 150.0 };
                let a = picture.bgra[i + 3] as f32 / 255.0;
                for c in [2, 1, 0] {
                    rgb.push((picture.bgra[i + c] as f32 + ground * (1.0 - a)).round() as u8);
                }
            }
        }
        image::save_buffer(&out, &rgb, w as u32, h as u32, image::ColorType::Rgb8).unwrap();
    }
}
