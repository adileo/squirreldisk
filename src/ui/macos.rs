//! macOS window chrome helpers.

/// Vertical center (points from the top of the window) and right edge of the
/// traffic-light buttons, read from AppKit so the custom top bar lines up
/// exactly on every macOS version.
pub fn traffic_lights(frame: &eframe::Frame) -> Option<(f32, f32)> {
    use objc2_app_kit::{NSView, NSWindowButton};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = frame.window_handle().ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return None };
    // SAFETY: winit hands out a valid NSView for the lifetime of the window,
    // and eframe calls us on the main thread.
    let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
    let window = view.window()?;
    let content = window.contentView()?;
    let height = content.frame().size.height;
    let close = window.standardWindowButton(NSWindowButton::CloseButton)?;
    let zoom = window.standardWindowButton(NSWindowButton::ZoomButton)?;
    let c = close.convertRect_toView(close.bounds(), None);
    let z = zoom.convertRect_toView(zoom.bounds(), None);
    let center_from_top = height - (c.origin.y + c.size.height / 2.0);
    Some((center_from_top as f32, (z.origin.x + z.size.width) as f32))
}

/// Replaces the Dock tile image (PNG bytes). Must run on the main thread.
pub fn set_dock_icon(png: &[u8]) {
    use objc2::MainThreadMarker;
    use objc2::AllocAnyThread;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;
    let Some(mtm) = MainThreadMarker::new() else { return };
    let data = NSData::with_bytes(png);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else { return };
    let app = NSApplication::sharedApplication(mtm);
    unsafe { app.setApplicationIconImage(Some(&image)) };
}
