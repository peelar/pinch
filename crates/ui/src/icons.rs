use gpui::{AnyElement, AssetSource, IntoElement, SharedString, Styled, img, px};
use pinch_proto::ProcessKind;
use std::borrow::Cow;

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "runtimes/node.svg" => Some(include_bytes!("../assets/runtimes/node.svg")),
            "runtimes/python.svg" => Some(include_bytes!("../assets/runtimes/python.svg")),
            "runtimes/npm.svg" => Some(include_bytes!("../assets/runtimes/npm.svg")),
            "runtimes/pnpm.svg" => Some(include_bytes!("../assets/runtimes/pnpm.svg")),
            "runtimes/yarn.svg" => Some(include_bytes!("../assets/runtimes/yarn.svg")),
            "runtimes/bun.svg" => Some(include_bytes!("../assets/runtimes/bun.svg")),
            "runtimes/rust.svg" => Some(include_bytes!("../assets/runtimes/rust.svg")),
            "runtimes/cloudflared.svg" => {
                Some(include_bytes!("../assets/runtimes/cloudflared.svg"))
            }
            _ => None,
        };
        Ok(bytes.map(Cow::Borrowed))
    }
    fn list(&self, _: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

pub fn runtime(kind: &ProcessKind) -> AnyElement {
    let name = match kind {
        ProcessKind::Node => "node",
        ProcessKind::Python => "python",
        ProcessKind::Npm => "npm",
        ProcessKind::Pnpm => "pnpm",
        ProcessKind::Yarn => "yarn",
        ProcessKind::Bun => "bun",
        ProcessKind::Cargo => "rust",
        ProcessKind::Other(name) if name.eq_ignore_ascii_case("cloudflared") => "cloudflared",
        ProcessKind::Other(_) => return gpui::div().w(px(20.)).h(px(20.)).into_any_element(),
    };
    img(SharedString::from(format!("runtimes/{name}.svg")))
        .w(px(20.))
        .h(px(20.))
        .flex_none()
        .into_any_element()
}
