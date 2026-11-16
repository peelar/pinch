//! The resident popover is one native panel, hidden and shown again instead of
//! recreated: every GPUI window removed leaks several megabytes, and a panel
//! that already exists opens without delay.

use gpui::Window;
use objc2::rc::Retained;
use objc2_app_kit::{NSScreen, NSView, NSWindow};
use objc2_foundation::{MainThreadMarker, NSPoint};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

fn native(window: &Window) -> Option<Retained<NSWindow>> {
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // SAFETY: GPUI's AppKit handle points at the window's content view, which
    // lives as long as the window, and we are on the main thread.
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    view.window()
}

pub fn is_visible(window: &Window) -> bool {
    native(window).is_some_and(|w| w.isVisible())
}

pub fn hide(window: &Window) {
    if let Some(w) = native(window) {
        w.orderOut(None);
    }
}

/// Show under the menu bar icon (`x` centre, `y` bottom, top-left-origin
/// points), or centred without an anchor, and take keyboard focus. The panel
/// is non-activating, so this does not activate the app.
pub fn show(window: &Window, anchor: Option<(f32, f32)>) {
    let Some(w) = native(window) else { return };
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    match anchor {
        Some((x, y)) => {
            // Cocoa's origin is the bottom-left of the screen with the menu bar.
            let screen = NSScreen::screens(mtm).firstObject().map(|s| s.frame());
            if let Some(screen) = screen {
                let width = w.frame().size.width;
                let left = (f64::from(x) - width / 2.0).clamp(8.0, screen.size.width - width - 8.0);
                w.setFrameTopLeftPoint(NSPoint::new(left, screen.size.height - f64::from(y) - 6.0));
            }
        }
        None => w.center(),
    }
    w.makeKeyAndOrderFront(None);
}
