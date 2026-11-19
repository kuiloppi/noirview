//! NoirView patch: what the native integration saw of the root window, for
//! diagnosing resize problems (Settings → Diagnostics in NoirView).

use std::sync::Mutex;

/// Sizes in physical pixels unless noted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowDiagnostics {
    /// `WindowEvent::Resized` events received.
    pub resized_events: u64,
    /// The size in the last `Resized` event.
    pub last_resized: [u32; 2],
    /// `WindowEvent::ScaleFactorChanged` events received.
    pub scale_events: u64,
    /// The scale factor in the last `ScaleFactorChanged` event.
    pub last_scale_event: f64,
    /// The size the surface was last configured to.
    pub surface: [u32; 2],
    /// How many times the surface was reconfigured just before painting
    /// because it no longer matched the window.
    pub resyncs: u64,
    /// `window.inner_size()` when the last frame was painted.
    pub paint_inner: [u32; 2],
    /// `window.outer_size()` when the last frame was painted.
    pub paint_outer: [u32; 2],
    /// `window.scale_factor()` when the last frame was painted.
    pub paint_scale: f64,
    /// Points to pixels used to paint the last frame.
    pub paint_ppp: f32,
    /// The window's client rectangle from Windows (GetClientRect), when known.
    pub client_rect: Option<[i32; 4]>,
    /// The window's rectangle on screen from Windows (GetWindowRect), when known.
    pub window_rect: Option<[i32; 4]>,
    /// The window's DPI from Windows (GetDpiForWindow), when known.
    pub dpi: Option<u32>,
    /// The last `CursorMoved` position.
    pub cursor: Option<[f64; 2]>,
    /// Frames painted.
    pub frames: u64,
}

static DIAG: Mutex<Option<WindowDiagnostics>> = Mutex::new(None);

/// A copy of what has been seen so far.
pub fn window_diagnostics() -> WindowDiagnostics {
    DIAG.lock().map(|d| d.clone().unwrap_or_default()).unwrap_or_default()
}

pub(crate) fn update(f: impl FnOnce(&mut WindowDiagnostics)) {
    if let Ok(mut d) = DIAG.lock() {
        f(d.get_or_insert_with(Default::default));
    }
}

/// The client and window rectangles and the DPI of a native window.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
pub(crate) fn native_rects(hwnd: isize) -> (Option<[i32; 4]>, Option<[i32; 4]>, Option<u32>) {
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect};
    let hwnd = hwnd as HWND;
    let mut client = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let mut window = client;
    // SAFETY: hwnd is the live window being painted; the rects are written in place.
    let (c, w, dpi) = unsafe {
        (
            GetClientRect(hwnd, &mut client),
            GetWindowRect(hwnd, &mut window),
            GetDpiForWindow(hwnd),
        )
    };
    let rect = |r: RECT| [r.left, r.top, r.right, r.bottom];
    (
        (c != 0).then(|| rect(client)),
        (w != 0).then(|| rect(window)),
        (dpi != 0).then_some(dpi),
    )
}

/// Records what the root window looks like when a frame is painted.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn record_paint(window: &winit::window::Window, pixels_per_point: f32) {
    let inner = window.inner_size();
    let outer = window.outer_size();
    let scale = window.scale_factor();
    #[cfg(target_os = "windows")]
    let native = {
        use raw_window_handle::HasWindowHandle as _;
        match window.window_handle().map(|h| h.as_raw()) {
            Ok(raw_window_handle::RawWindowHandle::Win32(h)) => native_rects(h.hwnd.get()),
            _ => (None, None, None),
        }
    };
    #[cfg(not(target_os = "windows"))]
    let native = (None, None, None);
    update(|d| {
        d.frames += 1;
        d.paint_inner = [inner.width, inner.height];
        d.paint_outer = [outer.width, outer.height];
        d.paint_scale = scale;
        d.paint_ppp = pixels_per_point;
        (d.client_rect, d.window_rect, d.dpi) = native;
    });
}

/// Records the root window's size, scale and cursor events.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn record_event(event: &winit::event::WindowEvent) {
    use winit::event::WindowEvent;
    match event {
        WindowEvent::Resized(size) => update(|d| {
            d.resized_events += 1;
            d.last_resized = [size.width, size.height];
            if size.width > 0 && size.height > 0 {
                d.surface = d.last_resized;
            }
        }),
        WindowEvent::ScaleFactorChanged { scale_factor, .. } => update(|d| {
            d.scale_events += 1;
            d.last_scale_event = *scale_factor;
        }),
        WindowEvent::CursorMoved { position, .. } => {
            update(|d| d.cursor = Some([position.x, position.y]));
        }
        _ => {}
    }
}

/// Paint times, split by whether the frame uploaded textures.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaintStats {
    /// Frames that uploaded textures.
    pub upload_frames: u64,
    /// Bytes of texture data uploaded.
    pub upload_bytes: u64,
    /// Paint time of those frames (ms, without waiting for the display).
    pub upload_ms: f64,
    /// The longest of them (ms).
    pub upload_max_ms: f64,
    /// Frames without texture uploads.
    pub plain_frames: u64,
    /// Their paint time (ms).
    pub plain_ms: f64,
}

static PAINT: Mutex<Option<PaintStats>> = Mutex::new(None);

/// Paint statistics since the last call.
pub fn take_paint_stats() -> PaintStats {
    PAINT
        .lock()
        .map(|mut p| p.take().unwrap_or_default())
        .unwrap_or_default()
}

/// Records one painted frame.
pub(crate) fn record_paint_time(delta: &egui::TexturesDelta, ms: f64) {
    let bytes: u64 = delta
        .set
        .iter()
        .map(|(_, d)| (d.image.width() * d.image.height() * 4) as u64)
        .sum();
    if let Ok(mut p) = PAINT.lock() {
        let p = p.get_or_insert_with(Default::default);
        if bytes > 0 {
            p.upload_frames += 1;
            p.upload_bytes += bytes;
            p.upload_ms += ms;
            p.upload_max_ms = p.upload_max_ms.max(ms);
        } else {
            p.plain_frames += 1;
            p.plain_ms += ms;
        }
    }
}
