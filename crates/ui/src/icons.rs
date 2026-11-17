use gpui::{AnyElement, AssetSource, IntoElement, SharedString, Styled, img, px};
use pinch_proto::{Harness, ProcessKind};
use std::borrow::Cow;

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if let Some(bundle) = path
            .strip_prefix(APP_ICON_PREFIX)
            .and_then(|p| p.strip_suffix(".png"))
        {
            return Ok(app_icon_png(bundle, 64).map(Cow::Owned));
        }
        let bytes: Option<&'static [u8]> = match path {
            "icons/settings.svg" => Some(include_bytes!("../assets/icons/settings.svg")),
            "icons/x.svg" => Some(include_bytes!("../assets/icons/x.svg")),
            "icons/undo.svg" => Some(include_bytes!("../assets/icons/undo.svg")),
            "icons/check.svg" => Some(include_bytes!("../assets/icons/check.svg")),
            "brand/daemon.svg" => Some(include_bytes!("../../../assets/brand/daemon.svg")),
            "brand/wordmark.svg" => Some(include_bytes!("../../../assets/brand/wordmark.svg")),
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
            "harnesses/claude-mark.svg" => {
                Some(include_bytes!("../assets/harnesses/claude-mark.svg"))
            }
            "harnesses/openai-mark.svg" => {
                Some(include_bytes!("../assets/harnesses/openai-mark.svg"))
            }
            "harnesses/cursor-mark.svg" => {
                Some(include_bytes!("../assets/harnesses/cursor-mark.svg"))
            }
            "harnesses/opencode-mark.svg" => {
                Some(include_bytes!("../assets/harnesses/opencode-mark.svg"))
            }
            "harnesses/zeron-logo.svg" => {
                Some(include_bytes!("../assets/harnesses/zeron-logo.svg"))
            }
            "harnesses/terminal.svg" => Some(include_bytes!("../assets/harnesses/terminal.svg")),
            "harnesses/file-code.svg" => Some(include_bytes!("../assets/harnesses/file-code.svg")),
            _ => None,
        };
        Ok(bytes.map(Cow::Borrowed))
    }
    fn list(&self, _: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

const APP_ICON_PREFIX: &str = "app-icons";

/// The icon Finder shows for an app bundle, falling back to the runtime
/// icon for process groups without one.
pub fn app(bundle: Option<&str>, name: &str) -> AnyElement {
    match bundle {
        Some(bundle) => img(SharedString::from(format!("{APP_ICON_PREFIX}{bundle}.png")))
            .w(px(20.))
            .h(px(20.))
            .flex_none()
            .into_any_element(),
        None => runtime(&ProcessKind::from_exe(name)),
    }
}

/// Render the bundle's Finder icon into a square PNG. AppKit allows this off
/// the main thread: the bitmap context is current only on this thread.
fn app_icon_png(bundle: &str, size: isize) -> Option<Vec<u8>> {
    use objc2::AllocAnyThread;
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace,
        NSGraphicsContext, NSWorkspace,
    };
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

    // Finder's icon for a missing path is a blank document.
    if !std::path::Path::new(bundle).exists() {
        return None;
    }
    let image = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(bundle));
    // SAFETY: a null plane pointer asks AppKit to allocate the pixel buffer.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            size,
            size,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            0,
        )
    }?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    let side = size as f64;
    image.drawInRect_fromRect_operation_fraction(
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(side, side)),
        NSRect::ZERO,
        NSCompositingOperation::SourceOver,
        1.0,
    );
    context.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();
    // SAFETY: an empty dictionary is valid PNG properties.
    let png = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    Some(png.to_vec())
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

pub fn harness(harness: Option<Harness>, color: gpui::Rgba) -> AnyElement {
    let name = match harness {
        Some(Harness::ClaudeCode) => "claude-mark",
        Some(Harness::Codex) => "openai-mark",
        Some(Harness::Cursor) => "cursor-mark",
        Some(Harness::OpenCode) => "opencode-mark",
        Some(Harness::Zeron) => "zeron-logo",
        Some(Harness::Terminal) | None => "terminal",
        Some(Harness::GeminiCli | Harness::Copilot) => "file-code",
    };
    gpui::svg()
        .path(SharedString::from(format!("harnesses/{name}.svg")))
        .size(px(14.))
        .text_color(color)
        .flex_none()
        .into_any_element()
}
