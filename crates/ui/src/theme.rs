//! Crisp neutral surfaces with acid accents; chrome stays dark in both appearances.

use gpui::{Rgba, WindowAppearance, rgb, rgba};

#[derive(Clone, Copy)]
pub struct Theme {
    /// Wash over the vibrant window background.
    pub tint: Rgba,
    pub text: Rgba,
    pub accent: Rgba,
    pub on_accent: Rgba,
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
                tint: rgb(0x080a09),
                accent: rgb(0xd7ff3f),
                on_accent: rgb(0x10120d),
                text: rgb(0xf7f8f7),
                muted: rgb(0xc5c9c7),
                faint: rgb(0xa4aaa7),
                hover: rgba(0xd7ff3f20),
                card: rgb(0x171b19),
                tooltip: rgb(0x171b19),
                border: rgb(0x3d4440),
                running: rgb(0xd7ff3f),
                stale: rgb(0xffb000),
                stopped: rgb(0x8b9290),
                danger: rgb(0xff5269),
                on_danger: rgb(0x10120d),
            },
            _ => Self {
                tint: rgb(0xf6f7f7),
                accent: rgb(0xd7ff3f),
                on_accent: rgb(0x10120d),
                text: rgb(0x10120d),
                muted: rgb(0x343a37),
                faint: rgb(0x4b534f),
                hover: rgba(0xc8ff0040),
                card: rgb(0xffffff),
                tooltip: rgb(0xf7f8f7),
                border: rgb(0xb9c1bc),
                running: rgb(0x287900),
                stale: rgb(0x925300),
                stopped: rgb(0x69736c),
                danger: rgb(0xc11538),
                on_danger: rgb(0xffffff),
            },
        }
    }
}
