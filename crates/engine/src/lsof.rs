//! Port discovery through `lsof`, which macOS ships and which, unlike
//! parsing `netstat`, reports the owning pid.

use std::collections::HashSet;
use std::process::Command;

fn run(state: &str) -> String {
    Command::new("/usr/sbin/lsof")
        .args(["-nP", "-iTCP", &format!("-sTCP:{state}"), "-Fpn"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// `(pid, port)` for every TCP listener.
pub fn listeners() -> Vec<(u32, u16)> {
    let mut out = Vec::new();
    let mut pid = 0u32;
    for line in run("LISTEN").lines() {
        if let Some(p) = line.strip_prefix('p') {
            pid = p.parse().unwrap_or(0);
        } else if let Some(name) = line.strip_prefix('n')
            && let Some(port) = name.rsplit(':').next().and_then(|p| p.parse().ok())
        {
            out.push((pid, port));
        }
    }
    out
}

/// Local ports that currently have an established connection. A listener's
/// accepted connections carry its own port locally, so this tells us whether
/// anyone (a browser tab, a test run) is talking to a server.
pub fn established_local_ports() -> HashSet<u16> {
    run("ESTABLISHED")
        .lines()
        .filter_map(|line| line.strip_prefix('n'))
        .filter_map(|name| {
            let local = name.split("->").next()?;
            local.rsplit(':').next()?.parse().ok()
        })
        .collect()
}
