//! Talking to the engine: where its files live, one blocking request per
//! connection, and starting the engine on demand when nobody is home.

pub mod installation;
pub mod onboarding;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use pinch_proto::{PROTOCOL_VERSION, Request, Response};

/// Everything pinch keeps on disk: socket, state, logs. `PINCH_HOME`
/// overrides the default `~/.pinch` (handy for tests).
pub fn home() -> PathBuf {
    if let Some(dir) = std::env::var_os("PINCH_HOME") {
        return PathBuf::from(dir);
    }
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join(installation::state_directory())
}

pub fn socket_path() -> PathBuf {
    home().join("pinchd.sock")
}

pub fn logs_dir() -> PathBuf {
    home().join("logs")
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A sibling executable of the running one (`pinchd` next to `pinch`, both
/// inside `Pinch.app/Contents/MacOS` once bundled).
pub fn sibling_exe(name: &str) -> Result<PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let path = exe.parent().context("executable has no parent")?.join(name);
    if !path.exists() {
        bail!("can't find {name} next to {}", exe.display());
    }
    Ok(path)
}

/// Send one request to a running engine.
pub fn request(req: &Request) -> Result<Response> {
    let mut stream = UnixStream::connect(socket_path()).context("engine is not running")?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    if reply.is_empty() {
        bail!("engine closed the connection without replying");
    }
    Ok(serde_json::from_str(&reply)?)
}

fn engine_alive() -> bool {
    matches!(
        request(&Request::Hello {
            version: PROTOCOL_VERSION
        }),
        Ok(Response::Hello { version }) if version == PROTOCOL_VERSION
    )
}

/// Like [`request`], but starts the engine first if it isn't running.
pub fn request_autostart(req: &Request) -> Result<Response> {
    ensure_engine()?;
    request(req)
}

pub fn ensure_engine() -> Result<()> {
    if engine_alive() {
        return Ok(());
    }
    start_engine(&sibling_exe("pinchd")?)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if engine_alive() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    bail!(
        "engine did not come up; see {}",
        home().join("pinchd.log").display()
    )
}

fn start_engine(exe: &Path) -> Result<()> {
    std::fs::create_dir_all(home())?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(home().join("pinchd.log"))?;
    let mut cmd = Command::new(exe);
    cmd.stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // SAFETY: setsid is async-signal-safe, which is all pre_exec may call. It
    // detaches the engine from this terminal so it outlives the caller.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().context("failed to launch pinchd")?;
    Ok(())
}
