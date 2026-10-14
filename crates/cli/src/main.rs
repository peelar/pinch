//! `pinch`: the CLI agents use to start, find, inspect and stop dev servers.

mod setup;

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use pinch_client::{now_ms, request_autostart};
use pinch_proto::{
    Decision, DecisionAnswer, Origin, ProcessInfo, Request, Response, Spawn, Status,
};

#[derive(Parser)]
#[command(
    name = "pinch",
    version,
    about = "Keeps coding agents from wasting resources on duplicate and forgotten dev servers"
)]
struct Cli {
    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start a process unless an equivalent one is already running in this repo.
    Ensure {
        /// Name to register it under, e.g. `web`.
        #[arg(long)]
        name: String,
        /// Who is starting it (defaults to $PINCH_OWNER).
        #[arg(long)]
        owner: Option<String>,
        /// Block until it is listening on a port.
        #[arg(long)]
        wait: bool,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Start a process unconditionally (fails if the name is taken).
    Run {
        #[arg(long)]
        name: String,
        #[arg(long)]
        owner: Option<String>,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// List managed and discovered processes.
    Ps,
    /// Show a process's logs.
    Logs {
        /// Name, id, or port.
        name: String,
        #[arg(long, default_value_t = 100)]
        tail: usize,
        #[arg(long, short)]
        follow: bool,
        /// Only show lines containing this text.
        #[arg(long)]
        grep: Option<String>,
    },
    /// Stop a registered process and everything it spawned.
    Stop { name: String },
    /// Stop and start again with the same command.
    Restart { name: String },
    /// Block until a process is listening on a port.
    Wait {
        name: String,
        #[arg(long, default_value_t = 60)]
        timeout_secs: u64,
    },
    /// List decisions waiting for confirmation.
    Inbox,
    /// Answer a pending decision.
    Answer {
        decision: String,
        #[arg(value_parser = ["kill", "keep", "snooze"])]
        answer: String,
    },
    /// Quit the engine (managed processes keep running).
    Quit,
    /// Detect agent harnesses and install the pinch skill.
    Setup {
        /// Install for every detected harness without asking.
        #[arg(long, short)]
        yes: bool,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("pinch: {e:#}");
        std::process::exit(1);
    }
}

fn call(req: Request) -> Result<Response> {
    match request_autostart(&req)? {
        Response::Error { message } => bail!("{message}"),
        other => Ok(other),
    }
}

fn spawn_request(name: String, owner: Option<String>, command: Vec<String>) -> Result<Spawn> {
    Ok(Spawn {
        name,
        command,
        cwd: std::env::current_dir()?.to_string_lossy().into_owned(),
        owner: owner.or_else(|| std::env::var("PINCH_OWNER").ok()),
        env: std::env::vars().collect::<BTreeMap<_, _>>(),
    })
}

fn snapshot() -> Result<(Vec<ProcessInfo>, Vec<Decision>)> {
    match call(Request::Snapshot)? {
        Response::Snapshot {
            processes,
            decisions,
        } => Ok((processes, decisions)),
        other => bail!("unexpected reply: {other:?}"),
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let json = cli.json;
    if !matches!(cli.command, Command::Setup { .. }) {
        setup::first_run(json);
    }
    match cli.command {
        Command::Ensure {
            name,
            owner,
            wait,
            command,
        } => {
            let spawn = spawn_request(name, owner, command)?;
            let Response::Ensured {
                mut process,
                already_running,
            } = call(Request::Ensure(spawn))?
            else {
                bail!("unexpected reply");
            };
            if wait {
                process = wait_for_port(&process.name, Duration::from_secs(60))?;
            }
            let status = if already_running {
                "already_running"
            } else {
                "started"
            };
            if json {
                print_json(&serde_json::json!({
                    "status": status,
                    "name": process.name,
                    "port": process.ports.first(),
                    "pid": process.pid,
                    "process": process,
                }));
            } else {
                let verb = if already_running {
                    "already running"
                } else {
                    "started"
                };
                println!("{verb}: {}", describe(&process));
                if !already_running && process.ports.is_empty() {
                    println!(
                        "  logs: pinch logs {0}   ·   wait for port: pinch wait {0}",
                        process.name
                    );
                }
            }
        }
        Command::Run {
            name,
            owner,
            command,
        } => {
            let Response::Process { process } =
                call(Request::Run(spawn_request(name, owner, command)?))?
            else {
                bail!("unexpected reply");
            };
            if json {
                print_json(&process);
            } else {
                println!("started: {}", describe(&process));
            }
        }
        Command::Ps => {
            let (processes, _) = snapshot()?;
            if json {
                print_json(&processes);
            } else {
                for (origin, heading) in [
                    (Origin::Managed, "REGISTERED THROUGH PINCH"),
                    (Origin::Unmanaged, "DISCOVERED (READ ONLY)"),
                ] {
                    let group: Vec<_> = processes
                        .iter()
                        .filter(|p| p.origin == origin)
                        .cloned()
                        .collect();
                    if !group.is_empty() {
                        println!("{heading}");
                        print_table(&group);
                    }
                }
                if processes.is_empty() {
                    println!("no processes");
                }
            }
        }
        Command::Logs {
            name,
            tail,
            follow,
            grep,
        } => logs(&name, tail, follow, grep.as_deref())?,
        Command::Stop { name } => {
            let Response::Process { process } = call(Request::Stop { name })? else {
                bail!("unexpected reply");
            };
            if json {
                print_json(&process);
            } else {
                println!("stopped: {}", process.name);
            }
        }
        Command::Restart { name } => {
            let Response::Process { process } = call(Request::Restart { name })? else {
                bail!("unexpected reply");
            };
            if json {
                print_json(&process);
            } else {
                println!("restarted: {}", describe(&process));
            }
        }
        Command::Wait { name, timeout_secs } => {
            let process = wait_for_port(&name, Duration::from_secs(timeout_secs))?;
            if json {
                print_json(&process);
            } else {
                println!("ready: {}", describe(&process));
            }
        }
        Command::Inbox => {
            let (_, decisions) = snapshot()?;
            if json {
                print_json(&decisions);
            } else if decisions.is_empty() {
                println!("nothing needs your attention");
            } else {
                for d in decisions {
                    println!("{}  {}", d.id, d.reason);
                    println!("    pinch answer {} kill|keep|snooze", d.id);
                }
            }
        }
        Command::Answer { decision, answer } => {
            let answer = match answer.as_str() {
                "kill" => DecisionAnswer::Kill,
                "keep" => DecisionAnswer::Keep,
                _ => DecisionAnswer::Snooze,
            };
            call(Request::Answer { decision, answer })?;
            println!("done");
        }
        Command::Setup { yes } => setup::run(yes)?,
        Command::Quit => {
            if pinch_client::request(&Request::Shutdown).is_ok() {
                println!("engine stopped (managed processes are still running)");
            } else {
                println!("engine was not running");
            }
        }
    }
    Ok(())
}

fn wait_for_port(name: &str, timeout: Duration) -> Result<ProcessInfo> {
    let deadline = Instant::now() + timeout;
    loop {
        let (processes, _) = snapshot()?;
        let found = processes
            .into_iter()
            .rev()
            .find(|p| p.name == name || p.id.0 == name);
        match found {
            Some(p) if matches!(p.status, Status::Exited { .. }) => {
                bail!(
                    "{} exited before listening; see `pinch logs {}`",
                    p.name,
                    p.name
                )
            }
            Some(p) if !p.ports.is_empty() => return Ok(p),
            None => bail!("no process named '{name}'"),
            _ => {}
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for '{name}' to listen on a port");
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn logs(name: &str, tail: usize, follow: bool, grep: Option<&str>) -> Result<()> {
    let keep = |line: &str| grep.is_none_or(|g| line.contains(g));
    let Response::Logs { text } = call(Request::Logs {
        name: name.to_string(),
        tail: Some(tail),
    })?
    else {
        bail!("unexpected reply");
    };
    text.lines()
        .filter(|l| keep(l))
        .for_each(|l| println!("{l}"));
    if !follow {
        return Ok(());
    }

    let (processes, _) = snapshot()?;
    let path = processes
        .iter()
        .rev()
        .find(|p| p.name == name || p.id.0 == name)
        .and_then(|p| p.log_path.clone())
        .context("process has no log file")?;
    let mut file = std::fs::File::open(path)?;
    let mut pos = file.seek(SeekFrom::End(0))?;
    let mut pending = String::new();
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let len = file.metadata()?.len();
        if len < pos {
            pos = 0; // truncated
        }
        if len > pos {
            file.seek(SeekFrom::Start(pos))?;
            let mut chunk = String::new();
            file.by_ref().take(len - pos).read_to_string(&mut chunk)?;
            pos = len;
            pending.push_str(&chunk);
            while let Some(i) = pending.find('\n') {
                let line: String = pending.drain(..=i).collect();
                if keep(line.trim_end()) {
                    print!("{line}");
                }
            }
        }
    }
}

fn print_json(value: &impl serde::Serialize) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

fn describe(p: &ProcessInfo) -> String {
    let ports = if p.ports.is_empty() {
        String::new()
    } else {
        format!(
            " :{}",
            p.ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    format!(
        "{}{} (pid {}, up {})",
        p.name,
        ports,
        p.pid.unwrap_or(0),
        duration(now_ms().saturating_sub(p.started_at_ms))
    )
}

fn print_table(processes: &[ProcessInfo]) {
    if processes.is_empty() {
        println!("no processes");
        return;
    }
    println!(
        "{:<18} {:<8} {:<8} {:>5} {:>8} {:>7}  WHERE",
        "NAME", "PORT", "STATUS", "CPU", "MEM", "UP"
    );
    for p in processes {
        let status = match (&p.origin, &p.status) {
            (Origin::Unmanaged, _) => "foreign".to_string(),
            (_, Status::Running) => "running".to_string(),
            (_, Status::Stale) => "stale".to_string(),
            (_, Status::Exited { code: Some(c) }) => format!("exit {c}"),
            (_, Status::Exited { code: None }) => "stopped".to_string(),
        };
        let (cpu, mem) = p
            .metrics
            .map(|m| (format!("{:.0}%", m.cpu_percent), bytes(m.rss_bytes)))
            .unwrap_or_else(|| ("-".into(), "-".into()));
        let port = p.ports.first().map_or("-".to_string(), |p| format!(":{p}"));
        let repo = p
            .repo
            .as_deref()
            .and_then(|r| r.rsplit('/').next())
            .unwrap_or("");
        let place = match &p.branch {
            Some(b) => format!("{repo}@{b}"),
            None => repo.to_string(),
        };
        let up = if p.status.is_alive() {
            duration(now_ms().saturating_sub(p.started_at_ms))
        } else {
            "-".into()
        };
        println!(
            "{:<18} {:<8} {:<8} {:>5} {:>8} {:>7}  {}",
            p.name, port, status, cpu, mem, up, place
        );
        if p.origin == Origin::Unmanaged {
            println!(
                "  PID {} · {}",
                p.pid.unwrap_or_default(),
                p.executable.as_deref().unwrap_or("executable unknown")
            );
            println!(
                "  cwd: {}",
                if p.cwd.is_empty() { "unknown" } else { &p.cwd }
            );
            println!("  command: {}", p.command.join(" "));
        }
    }
}

fn bytes(n: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = n as f64 / MB;
    if mb >= 1024.0 {
        format!("{:.1}G", mb / 1024.0)
    } else {
        format!("{mb:.0}M")
    }
}

fn duration(ms: u64) -> String {
    let secs = ms / 1000;
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86399 => format!("{}h{}m", secs / 3600, secs % 3600 / 60),
        _ => format!("{}d", secs / 86400),
    }
}
