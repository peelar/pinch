//! Terminal onboarding: offer to install the pinch skill into detected harnesses.

use std::io::{BufRead, IsTerminal, Write};

use anyhow::Result;
use pinch_client::onboarding;

fn ask(question: &str) -> bool {
    print!("{question} [Y/n] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return false;
    }
    !matches!(line.trim().to_lowercase().as_str(), "n" | "no")
}

/// `pinch setup`. With `yes`, installs for every detected harness without asking.
pub fn run(yes: bool) -> Result<()> {
    let found = onboarding::detect();
    if found.is_empty() {
        println!("No agent harness detected (looked for Claude Code and Codex).");
        println!("The skill lives in skill/SKILL.md; copy it where your agent reads skills.");
    } else {
        let names: Vec<_> = found.iter().map(|h| h.name).collect();
        println!("Hi! pinch found: {}.", names.join(", "));
        println!("The pinch skill teaches agents to start servers through pinch.\n");
        for h in &found {
            let verb = if h.installed { "Update" } else { "Install" };
            if yes || ask(&format!("{verb} the pinch skill for {}?", h.name)) {
                println!("  ✓ {}", onboarding::install(h)?.display());
            }
        }
    }
    onboarding::mark_done();
    Ok(())
}

/// First interactive use: offer setup once. Silent when piped, in JSON mode,
/// or already done, so agents are never blocked on a prompt.
pub fn first_run(json: bool) {
    if json
        || !onboarding::needed()
        || !std::io::stdin().is_terminal()
        || !std::io::stdout().is_terminal()
    {
        return;
    }
    if let Err(e) = run(false) {
        eprintln!("pinch: setup skipped: {e:#}");
    }
    println!();
}
