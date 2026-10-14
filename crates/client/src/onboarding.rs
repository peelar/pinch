//! Onboarding shared by the CLI and the desktop app: detect agent harnesses
//! and install the pinch skill into them.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const SKILL: &str = include_str!("../../../skill/SKILL.md");

struct Harness {
    name: &'static str,
    binary: &'static str,
    config_dir: &'static str,
    skills_dir: &'static str,
}

const HARNESSES: &[Harness] = &[
    Harness {
        name: "Claude Code",
        binary: "claude",
        config_dir: ".claude",
        skills_dir: ".claude/skills",
    },
    Harness {
        name: "Codex",
        binary: "codex",
        config_dir: ".codex",
        skills_dir: ".codex/skills",
    },
];

/// An agent harness found on this machine.
#[derive(Debug, Clone)]
pub struct Detected {
    pub name: &'static str,
    /// The pinch skill is already in place.
    pub installed: bool,
    skills_dir: &'static str,
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("$HOME is not set")
}

fn on_path(binary: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(binary).is_file()))
}

fn skill_path(skills_dir: &str, home: &Path) -> PathBuf {
    home.join(skills_dir).join("pinch").join("SKILL.md")
}

/// Harnesses whose config dir exists or whose binary is on `$PATH`.
pub fn detect() -> Vec<Detected> {
    let Ok(home) = home_dir() else {
        return Vec::new();
    };
    HARNESSES
        .iter()
        .filter(|h| home.join(h.config_dir).is_dir() || on_path(h.binary))
        .map(|h| Detected {
            name: h.name,
            installed: skill_path(h.skills_dir, &home).exists(),
            skills_dir: h.skills_dir,
        })
        .collect()
}

/// Write (or refresh) the skill for one harness; returns where it went.
pub fn install(target: &Detected) -> Result<PathBuf> {
    let path = skill_path(target.skills_dir, &home_dir()?);
    std::fs::create_dir_all(path.parent().expect("skill path has a parent"))?;
    std::fs::write(&path, SKILL)?;
    Ok(path)
}

fn marker() -> PathBuf {
    crate::home().join("onboarded")
}

/// True until the user has installed or dismissed the offer once.
pub fn needed() -> bool {
    !marker().exists()
}

pub fn mark_done() {
    let path = marker();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, "");
}
