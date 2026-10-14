//! `pinch-ui`: the GPUI desktop app, launched on demand by the engine when the
//! menu bar icon is clicked. Never resident: it closes when it loses focus.
//! See ARCHITECTURE.md.

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
    let initial = view::fetch();

    application().run(move |cx: &mut App| {
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

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // No titlebar: a titled, full-size-content window with
                // native rounded corners and desktop vibrancy behind it.
                titlebar: None,
                window_background: WindowBackgroundAppearance::Blurred,
                kind: WindowKind::Normal,
                is_resizable: false,
                is_movable: false,
                is_minimizable: false,
                focus: true,
                show: true,
                ..Default::default()
            },
            |window, cx| cx.new(|cx| view::Popover::new(initial, window, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
}
