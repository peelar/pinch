//! Colours, following the system appearance. Surfaces are translucent so the
//! window's vibrancy shows through; text and accents are opaque.

use gpui::{Rgba, WindowAppearance, rgb, rgba};

#[derive(Clone, Copy)]
pub struct Theme {
    /// Wash over the vibrant window background.
    pub tint: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub faint: Rgba,
    pub hover: Rgba,
    pub card: Rgba,
    pub tooltip: Rgba,
    pub border: Rgba,
    pub running: Rgba,
    pub stale: Rgba,
    pub stopped: Rgba,
    pub danger: Rgba,
    pub on_danger: Rgba,
}

impl Theme {
    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self {
                tint: rgba(0x1c1c1e8c),
                text: rgb(0xf5f5f7),
                muted: rgb(0xc7c7cc),
                faint: rgb(0xaeaeb5),
                hover: rgba(0xffffff12),
                card: rgba(0xffffff0f),
                tooltip: rgb(0x29292c),
                border: rgba(0xffffff1a),
                running: rgb(0x32d74b),
                stale: rgb(0xffb340),
                stopped: rgb(0x6e6e76),
                danger: rgb(0xff453a),
                on_danger: rgb(0xffffff),
            },
            _ => Self {
                tint: rgba(0xf5f5f78c),
                text: rgb(0x1d1d1f),
                muted: rgb(0x303034),
                faint: rgb(0x313135),
                hover: rgba(0x00000010),
                card: rgba(0xffffff80),
                tooltip: rgb(0xfafafa),
                border: rgba(0x0000001a),
                running: rgb(0x28cd41),
                stale: rgb(0xf5a000),
                stopped: rgb(0xaeaeb2),
                danger: rgb(0xff3b30),
                on_danger: rgb(0xffffff),
            },
        }
    }
}
