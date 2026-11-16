//! `pinch-ui`: the GPUI desktop app, prewarmed by the engine.
//! The runtime stays ready between menu bar clicks.
//! See DEVELOPMENT.md.

mod icons;
mod panel;
mod process_list;
mod resources;
mod theme;
mod view;

use gpui::{
    App, AppContext as _, Bounds, WindowBackgroundAppearance, WindowBounds, WindowKind,
    WindowOptions, point, px, size,
};
use gpui_platform::application;

/// `--anchor x,y`: where the menu bar icon is (centre x, bottom y), in points
/// from the top-left of the screen.
fn parse_anchor() -> Option<(f32, f32)> {
    let mut args = std::env::args().skip_while(|a| a != "--anchor");
    args.next()?;
    let value = args.next()?;
    let (x, y) = value.split_once(',')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

fn main() {
    let anchor = parse_anchor();
    let resident = std::env::args().any(|a| a == "--resident");
    let initial = (!resident).then(view::fetch);
    let app = application().with_assets(icons::Assets);
    app.run(move |cx: &mut App| {
        if !resident {
            open(anchor, initial.unwrap(), true, cx);
            return;
        }
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            use std::io::BufRead as _;
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if tx.send_blocking(line).is_err() {
                    break;
                }
            }
        });
        // One hidden panel for the life of the process. It refreshes its own
        // snapshot every second, so it is current whenever it is shown.
        let popover = open(None, view::cached(), false, cx);
        cx.spawn(async move |cx| {
            loop {
                match rx.recv().await {
                    Ok(line) => {
                        // `toggle` comes from the menu bar icon, `show` from
                        // notifications and first launch; either may carry `x,y`.
                        let (verb, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
                        let show_only = verb == "show";
                        let anchor = rest
                            .split_once(',')
                            .and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)));
                        cx.update(|cx| {
                            let _ = popover.update(cx, |this, window, cx| {
                                if panel::is_visible(window) {
                                    // Close only a popover the user is using;
                                    // otherwise bring it forward again.
                                    if window.is_window_active() && !show_only {
                                        panel::hide(window);
                                        return;
                                    }
                                } else if !show_only && view::recently_closed() {
                                    // The click that blurred and closed it: stay closed.
                                    return;
                                }
                                this.reopen(window, cx);
                                panel::show(window, anchor);
                            });
                        });
                    }
                    Err(_) => {
                        cx.update(|cx| cx.quit());
                        break;
                    }
                }
            }
        })
        .detach();
    });
}

fn open(
    anchor: Option<(f32, f32)>,
    initial: view::Snapshot,
    show: bool,
    cx: &mut App,
) -> gpui::WindowHandle<view::Popover> {
    let resident = std::env::args().any(|a| a == "--resident");
    let height = view::height_for(&initial);
    let window_size = size(px(view::WIDTH), px(height));
    let bounds = match anchor {
        Some((x, y)) => {
            let screen_width = cx
                .primary_display()
                .map_or(1440.0, |d| f32::from(d.bounds().size.width));
            let left = (x - view::WIDTH / 2.0).clamp(8.0, screen_width - view::WIDTH - 8.0);
            Bounds::new(point(px(left), px(y + 6.0)), window_size)
        }
        None => Bounds::centered(None, window_size, cx),
    };

    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // No titlebar: a titled, full-size-content window with
                // native rounded corners and desktop vibrancy behind it.
                titlebar: None,
                window_background: WindowBackgroundAppearance::Blurred,
                // From the menu bar, a non-activating panel takes keyboard
                // focus without making pinch-ui the active app, so opening
                // never depends on macOS agreeing to hand activation over
                // from pinchd. Previews stay ordinary windows.
                kind: if resident {
                    WindowKind::PopUp
                } else {
                    WindowKind::Normal
                },
                is_resizable: false,
                is_movable: false,
                is_minimizable: false,
                focus: show,
                show,
                ..Default::default()
            },
            |window, cx| cx.new(|cx| view::Popover::new(initial, window, cx)),
        )
        .unwrap();
    if show && !resident {
        activate();
    }
    handle
}

/// Bring a preview window forward. GPUI's `activate` uses
/// `activateIgnoringOtherApps`, which macOS 14+ ignores.
fn activate() {
    if let Some(mtm) = objc2_foundation::MainThreadMarker::new() {
        objc2_app_kit::NSApplication::sharedApplication(mtm).activate();
    }
}
