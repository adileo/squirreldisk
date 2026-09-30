//! Where the window opens: centred on the screen the user is working on (the
//! one under the mouse), sized to most of it. Best effort: when the platform
//! can't tell, the app falls back to the primary screen (see
//! `App::place_window`).

/// Window size for a screen area of `avail` (same unit as `scale`, which is
/// the size of one point): most of a small screen, but on a big one no more
/// than a little wider than the home column (820 points), so the layout
/// doesn't get sparse; never less than the window's minimum size.
pub fn fit(avail: (f64, f64), scale: f64) -> (f64, f64) {
    let (w, h) = avail;
    ((w * 0.8).clamp(860.0 * scale, 1080.0 * scale).min(w), (h * 0.85).clamp(560.0 * scale, 800.0 * scale).min(h))
}

pub enum Placed {
    Done,
    /// Moved to another screen: size it next frame, once its DPI applies.
    #[cfg_attr(not(windows), allow(dead_code))]
    Again,
    /// The platform can't say: use the fallback.
    Unknown,
}

#[cfg(target_os = "macos")]
pub fn on_active_screen(frame: &eframe::Frame) -> Placed {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSScreen, NSView};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Some(mtm) = MainThreadMarker::new() else { return Placed::Unknown };
    let Ok(handle) = frame.window_handle() else { return Placed::Unknown };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return Placed::Unknown };
    // SAFETY: winit hands out a valid NSView for the lifetime of the window,
    // and eframe calls us on the main thread.
    let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
    let Some(window) = view.window() else { return Placed::Unknown };
    // AppKit coordinates: points, origin at the bottom left of the main screen
    let mouse = NSEvent::mouseLocation();
    let under_mouse = NSScreen::screens(mtm).iter().find(|s| {
        let f = s.frame();
        mouse.x >= f.origin.x && mouse.x < f.origin.x + f.size.width && mouse.y >= f.origin.y && mouse.y < f.origin.y + f.size.height
    });
    let Some(screen) = under_mouse.or_else(|| NSScreen::mainScreen(mtm)) else { return Placed::Unknown };
    let area = screen.visibleFrame(); // without the menu bar and the Dock
    let (w, h) = fit((area.size.width, area.size.height), 1.0);
    let origin = NSPoint::new(area.origin.x + (area.size.width - w) / 2.0, area.origin.y + (area.size.height - h) / 2.0);
    window.setFrame_display(NSRect::new(origin, NSSize::new(w, h)), true);
    Placed::Done
}

#[cfg(windows)]
pub fn on_active_screen(frame: &eframe::Frame) -> Placed {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::{HWND, POINT};
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL};
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER};
    let Ok(handle) = frame.window_handle() else { return Placed::Unknown };
    let RawWindowHandle::Win32(h) = handle.as_raw() else { return Placed::Unknown };
    let hwnd = h.hwnd.get() as HWND;
    // SAFETY: plain Win32 calls on our own window, with valid out-pointers.
    unsafe {
        let mut cursor = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut cursor) == 0 {
            return Placed::Unknown;
        }
        let target = MonitorFromPoint(cursor, MONITOR_DEFAULTTONULL);
        if target.is_null() {
            return Placed::Unknown;
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(target, &mut info) == 0 {
            return Placed::Unknown;
        }
        let work = info.rcWork; // physical pixels, without the taskbar
        if MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) != target {
            // Another screen, maybe with another scale: move there first; the
            // size is worked out next frame with that screen's DPI.
            SetWindowPos(hwnd, std::ptr::null_mut(), work.left + 40, work.top + 40, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            return Placed::Again;
        }
        let scale = match GetDpiForWindow(hwnd) {
            0 => 1.0,
            dpi => dpi as f64 / 96.0,
        };
        let (aw, ah) = ((work.right - work.left) as f64, (work.bottom - work.top) as f64);
        let (w, h) = fit((aw, ah), scale);
        let (x, y) = (work.left + ((aw - w) / 2.0) as i32, work.top + ((ah - h) / 2.0) as i32);
        if SetWindowPos(hwnd, std::ptr::null_mut(), x, y, w as i32, h as i32, SWP_NOZORDER | SWP_NOACTIVATE) == 0 {
            return Placed::Unknown;
        }
    }
    Placed::Done
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn on_active_screen(_frame: &eframe::Frame) -> Placed {
    // X11 would allow it, Wayland doesn't let apps place windows at all
    Placed::Unknown
}
