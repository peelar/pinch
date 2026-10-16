//! Identity shared by the binaries inside one installed app bundle.

use std::io::ErrorKind;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};

static VARIANT: OnceLock<Option<String>> = OnceLock::new();

pub fn variant() -> Option<&'static str> {
    VARIANT
        .get_or_init(|| {
            let exe = std::env::current_exe()
                .and_then(|p| p.canonicalize())
                .expect("cannot locate the Pinch executable");
            read_variant(&exe).expect("invalid Pinch installation metadata")
        })
        .as_deref()
}

pub fn display_name() -> String {
    match variant() {
        Some(variant) => format!("Pinch {}", title(variant)),
        None => "Pinch".into(),
    }
}

pub fn cli_name() -> String {
    match variant() {
        Some(variant) => format!("pinch-{variant}"),
        None => "pinch".into(),
    }
}

pub fn state_directory() -> String {
    match variant() {
        Some(variant) => format!(".pinch-{variant}"),
        None => ".pinch".into(),
    }
}

fn read_variant(exe: &Path) -> Result<Option<String>> {
    let Some(contents) = exe.parent().and_then(Path::parent) else {
        return Ok(None);
    };
    if contents.file_name().is_none_or(|name| name != "Contents")
        || exe
            .parent()
            .and_then(Path::file_name)
            .is_none_or(|name| name != "MacOS")
    {
        return Ok(None);
    }
    let path = contents.join("Resources/pinch-variant");
    let value = match std::fs::read_to_string(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    let value = value.trim_end_matches('\n');
    if !valid_variant(value) {
        bail!("invalid variant in {}", path.display());
    }
    Ok(Some(value.to_owned()))
}

fn valid_variant(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn title(variant: &str) -> String {
    variant
        .split('-')
        .map(|part| format!("{}{}", part[..1].to_ascii_uppercase(), &part[1..]))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_cannot_escape_their_state_or_bundle_paths() {
        for value in ["proactive", "ui-test", "pr-123"] {
            assert!(valid_variant(value));
        }
        for value in [
            "",
            "../stable",
            "Stable",
            "a/b",
            "a--b",
            "a-",
            "-a",
            "1test",
            "a b",
            "ü",
            "abcdefghijklmnopqrstuvwxyz1234567",
        ] {
            assert!(!valid_variant(value), "{value}");
        }
        assert_eq!(title("ui-test"), "Ui Test");
    }

    #[test]
    fn reads_bundle_metadata_and_rejects_invalid_metadata() {
        let root = std::env::temp_dir().join(format!("pinch-installation-{}", std::process::id()));
        let resources = root.join("Pinch.app/Contents/Resources");
        let exe = root.join("Pinch.app/Contents/MacOS/pinch");
        std::fs::create_dir_all(&resources).unwrap();
        assert_eq!(read_variant(&exe).unwrap(), None);
        let metadata = resources.join("pinch-variant");
        std::fs::write(&metadata, "proactive\n").unwrap();
        assert_eq!(read_variant(&exe).unwrap().as_deref(), Some("proactive"));
        std::fs::write(&metadata, "../stable").unwrap();
        assert!(read_variant(&exe).is_err());
        assert_eq!(
            read_variant(Path::new("/tmp/target/debug/pinch")).unwrap(),
            None
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
