//! The registry: managed processes, their metrics, and the decisions the
//! staleness rule raises about them.
//!
//! Managed processes are started in their own session (`setsid`) with stdout
//! and stderr pointing straight at a log file. That is what lets them outlive
//! the engine: after a restart the engine re-adopts them from `state.json`,
//! checking pid *and* start time so a recycled pid is never mistaken for one.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use pinch_client::{home, logs_dir, now_ms};
use pinch_proto::{
    Decision, DecisionAnswer, Metrics, Origin, ProcessId, ProcessInfo, ProcessKind, Spawn, Status,
    SystemMetrics,
};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::discovery;

pub struct Config {
    /// How long a server may be silent and unconnected before we ask about it.
    pub stale_after: Duration,
}

impl Config {
    pub fn from_env() -> Self {
        let secs = std::env::var("PINCH_STALE_AFTER_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2 * 60 * 60);
        Self {
            stale_after: Duration::from_secs(secs),
        }
    }
}

/// Callbacks to consumers of engine decisions. They are invoked while the registry lock
/// is held, so they must only enqueue work.
type DecisionsChanged = Box<dyn Fn(&[Decision]) + Send + Sync>;

pub struct Hooks {
    pub decisions_changed: DecisionsChanged,
}

impl Default for Hooks {
    fn default() -> Self {
        Self {
            decisions_changed: Box::new(|_| {}),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    info: ProcessInfo,
    /// sysinfo's start time (seconds) of the root pid: the pid-reuse guard.
    start_time_s: u64,
    kept: bool,
    #[serde(default)]
    last_activity_ms: u64,
    #[serde(default)]
    pending_decision: Option<Decision>,
    /// Not persisted: environments hold secrets.
    #[serde(skip)]
    env: BTreeMap<String, String>,
}

struct Inner {
    entries: Vec<Entry>,
    decisions: Vec<Decision>,
    unmanaged: Vec<ProcessInfo>,
    sys: System,
    system_metrics: SystemMetrics,
    cpu_totals: HashMap<u32, u64>,
    system_sampled_at: Option<Instant>,
    counter: u64,
    last_decisions: Vec<String>,
    last_activity_saved_ms: u64,
}

pub struct Engine {
    inner: Mutex<Inner>,
    cfg: Config,
    hooks: Hooks,
}

fn refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
        .with_cwd(UpdateKind::OnlyIfNotSet)
}

fn refresh_system_metrics(inner: &mut Inner) {
    inner.sys.refresh_cpu_usage();
    inner.sys.refresh_memory();
    let cpu_percent = inner
        .system_sampled_at
        .filter(|at| at.elapsed() >= sysinfo::MINIMUM_CPU_UPDATE_INTERVAL)
        .map(|_| inner.sys.global_cpu_usage().clamp(0.0, 100.0));
    inner.system_sampled_at = Some(Instant::now());
    inner.system_metrics = SystemMetrics {
        cpu_percent,
        memory_used_bytes: inner.sys.used_memory(),
        memory_total_bytes: inner.sys.total_memory(),
        memory_available_bytes: inner.sys.available_memory(),
        swap_used_bytes: inner.sys.used_swap(),
        sampled_at_ms: now_ms(),
    };
}

impl Engine {
    pub fn new(cfg: Config, hooks: Hooks) -> Result<Arc<Self>> {
        fs::create_dir_all(logs_dir())?;
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind());

        let mut entries: Vec<Entry> = fs::read(state_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let now = now_ms();
        for entry in &mut entries {
            if entry.info.status.is_alive() && !is_same_process(&sys, entry) {
                entry.info.status = Status::Exited { code: None };
            }
            if entry.pending_decision.is_none() {
                entry.last_activity_ms = now;
            }
        }
        let decisions: Vec<Decision> = entries
            .iter_mut()
            .filter_map(|entry| {
                let decision = entry.pending_decision.take()?;
                (entry.info.origin == Origin::Managed
                    && entry.info.status.is_alive()
                    && !entry.kept
                    && decision.process == entry.info.id)
                    .then_some(decision)
            })
            .collect();

        let mut inner = Inner {
            entries,
            last_decisions: decisions.iter().map(|d| d.id.clone()).collect(),
            decisions,
            unmanaged: Vec::new(),
            sys,
            system_metrics: SystemMetrics::default(),
            cpu_totals: HashMap::new(),
            system_sampled_at: None,
            counter: 0,
            last_activity_saved_ms: now,
        };
        refresh_system_metrics(&mut inner);
        let engine = Arc::new(Self {
            inner: Mutex::new(inner),
            cfg,
            hooks,
        });
        save(&engine.lock());
        Ok(engine)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- requests ---------------------------------------------------------

    /// Start unless an equivalent process is already running in this repo.
    /// Everything happens under one lock, so concurrent callers get one process.
    pub fn ensure(self: &Arc<Self>, spawn: Spawn) -> Result<(ProcessInfo, bool)> {
        let mut inner = self.lock();
        let key = repo_key(&spawn.cwd);
        let existing = inner.entries.iter().position(|e| {
            e.info.status.is_alive()
                && repo_key_of(&e.info) == key
                && (e.info.name == spawn.name || e.info.command == spawn.command)
        });
        if let Some(idx) = existing {
            let entry = &mut inner.entries[idx];
            if entry.info.command != spawn.command {
                bail!(
                    "'{}' is already running in this repo with a different command ({}); \
                     use `{} restart {}` or pick another name",
                    entry.info.name,
                    entry.info.command.join(" "),
                    pinch_client::installation::cli_name(),
                    entry.info.name
                );
            }
            entry.last_activity_ms = now_ms();
            entry.info.status = Status::Running;
            let info = entry.info.clone();
            drop_decisions_for(&mut inner, &info.id);
            sync_attention(&self.hooks, &mut inner);
            save(&inner);
            return Ok((info, true));
        }
        let info = self.spawn_locked(&mut inner, spawn)?;
        Ok((info, false))
    }

    pub fn run(self: &Arc<Self>, spawn: Spawn) -> Result<ProcessInfo> {
        let mut inner = self.lock();
        if inner
            .entries
            .iter()
            .any(|e| e.info.status.is_alive() && e.info.name == spawn.name)
        {
            bail!(
                "'{}' is already running; use `ensure` or `restart`",
                spawn.name
            );
        }
        self.spawn_locked(&mut inner, spawn)
    }

    pub fn snapshot(&self) -> (Vec<ProcessInfo>, Vec<Decision>, SystemMetrics) {
        let inner = self.lock();
        let mut processes: Vec<ProcessInfo> =
            inner.entries.iter().map(|e| e.info.clone()).collect();
        processes.extend(inner.unmanaged.iter().cloned());
        (processes, inner.decisions.clone(), inner.system_metrics)
    }

    pub fn stop(&self, target: &str) -> Result<ProcessInfo> {
        let mut inner = self.lock();
        if let Some(idx) = find_managed(&inner, target) {
            let entry = &mut inner.entries[idx];
            if let (true, Some(pid)) = (entry.info.status.is_alive(), entry.info.pid) {
                terminate_group(pid);
                entry.info.status = Status::Exited { code: None };
                entry.info.ports.clear();
                entry.info.metrics = None;
            }
            let info = entry.info.clone();
            drop_decisions_for(&mut inner, &info.id);
            sync_attention(&self.hooks, &mut inner);
            save(&inner);
            return Ok(info);
        }
        if let Some(info) = find_unmanaged(&inner, target) {
            bail!(
                "'{}' was discovered, not registered through Pinch; discovered processes are read-only",
                info.name
            );
        }
        bail!("no process matching '{target}'")
    }

    pub fn stop_discovered(&self, confirmed: &ProcessInfo) -> Result<ProcessInfo> {
        let inner = self.lock();
        let info = inner
            .unmanaged
            .iter()
            .find(|p| p.id == confirmed.id)
            .ok_or_else(|| anyhow!("discovered process is no longer available; review it again"))?
            .clone();
        let pid = info.pid.ok_or_else(|| anyhow!("process has no PID"))?;
        if pid <= 1 || pid == std::process::id() || confirmed.origin != Origin::Unmanaged {
            bail!("refusing to terminate a protected process");
        }
        let mut fresh = System::new();
        fresh.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
            refresh_kind(),
        );
        let process = fresh
            .process(Pid::from_u32(pid))
            .ok_or_else(|| anyhow!("process has already exited"))?;
        let command: Vec<String> = process
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        let executable = process.exe().map(|p| p.to_string_lossy().into_owned());
        let cwd = process
            .cwd()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if info.id.0 != format!("pid:{pid}:{}", process.start_time())
            || confirmed.pid != Some(pid)
            || confirmed.command != command
            || confirmed.executable != executable
            || confirmed.cwd != cwd
            || confirmed.started_at_ms != process.start_time() * 1000
        {
            bail!("process identity changed; review it again before stopping");
        }
        // Discovered listeners can share a session with unrelated applications.
        if unsafe { libc::kill(pid as i32, libc::SIGTERM) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(info)
    }

    pub fn clear(&self, target: &str) -> Result<()> {
        let mut inner = self.lock();
        let idx = find_managed(&inner, target)
            .ok_or_else(|| anyhow!("no managed process matching '{target}'"))?;
        if inner.entries[idx].info.status.is_alive() {
            bail!("stop '{}' before clearing it", inner.entries[idx].info.name);
        }
        let entry = inner.entries.remove(idx);
        drop_decisions_for(&mut inner, &entry.info.id);
        sync_attention(&self.hooks, &mut inner);
        save(&inner);
        Ok(())
    }

    pub fn restart(self: &Arc<Self>, target: &str) -> Result<ProcessInfo> {
        let (spawn, pid) = {
            let inner = self.lock();
            let idx = find_managed(&inner, target)
                .ok_or_else(|| anyhow!("no managed process matching '{target}'"))?;
            let entry = &inner.entries[idx];
            let agent = entry.info.agent.clone().context(
                "this legacy process has no harness; stop it and use run/ensure with --harness",
            )?;
            let spawn = Spawn {
                agent,
                name: entry.info.name.clone(),
                command: entry.info.command.clone(),
                cwd: entry.info.cwd.clone(),
                owner: entry.info.owner.clone(),
                env: entry.env.clone(),
            };
            (
                spawn,
                entry
                    .info
                    .status
                    .is_alive()
                    .then_some(entry.info.pid)
                    .flatten(),
            )
        };
        if let Some(pid) = pid {
            terminate_group(pid);
            // Wait (without holding the lock) so the new process can have the port.
            let deadline = std::time::Instant::now() + Duration::from_secs(6);
            while pid_alive(pid) && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        let mut inner = self.lock();
        if let Some(idx) = find_managed(&inner, &spawn.name) {
            inner.entries[idx].info.status = Status::Exited { code: None };
        }
        self.spawn_locked(&mut inner, spawn)
    }

    pub fn logs(&self, target: &str, tail: usize) -> Result<String> {
        let path = {
            let inner = self.lock();
            let idx = find_managed(&inner, target).ok_or_else(|| {
                anyhow!("no managed process matching '{target}' (unmanaged processes have no logs)")
            })?;
            inner.entries[idx]
                .info
                .log_path
                .clone()
                .context("no log file")?
        };
        Ok(tail_lines(Path::new(&path), tail)?)
    }

    pub fn answer(&self, decision_id: &str, answer: DecisionAnswer) -> Result<()> {
        let mut inner = self.lock();
        let decision = inner
            .decisions
            .iter()
            .find(|d| d.id == decision_id)
            .cloned()
            .ok_or_else(|| anyhow!("decision {decision_id} is no longer pending"))?;
        let idx = inner
            .entries
            .iter()
            .position(|e| e.info.id == decision.process);
        match (answer, idx) {
            (DecisionAnswer::Kill, Some(idx)) => {
                let pid = inner.entries[idx].info.pid.context("process has no PID")?;
                inner.sys.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
                    true,
                    refresh_kind(),
                );
                if inner.entries[idx].info.origin != Origin::Managed
                    || !is_same_process(&inner.sys, &inner.entries[idx])
                {
                    drop_decisions_for(&mut inner, &decision.process);
                    sync_attention(&self.hooks, &mut inner);
                    save(&inner);
                    bail!("process identity changed; the stale suggestion was dismissed");
                }
                terminate_group(pid);
                inner.entries[idx].info.status = Status::Exited { code: None };
                inner.entries[idx].info.metrics = None;
                inner.entries[idx].info.ports.clear();
            }
            (DecisionAnswer::Keep, Some(idx)) => {
                inner.entries[idx].kept = true;
                inner.entries[idx].info.status = Status::Running;
            }
            (_, None) => {}
        }
        inner.decisions.retain(|d| d.id != decision_id);
        sync_attention(&self.hooks, &mut inner);
        save(&inner);
        Ok(())
    }

    // ---- spawning ---------------------------------------------------------

    fn spawn_locked(self: &Arc<Self>, inner: &mut Inner, spawn: Spawn) -> Result<ProcessInfo> {
        let program = spawn.command.first().context("empty command")?.clone();
        let key = repo_key(&spawn.cwd);
        inner.entries.retain(|e| {
            let replaced = !e.info.status.is_alive()
                && e.info.name == spawn.name
                && repo_key_of(&e.info) == key;
            if replaced && let Some(log) = &e.info.log_path {
                let _ = fs::remove_file(log);
            }
            !replaced
        });
        inner.counter += 1;
        let id = format!("{:x}{:x}", now_ms() & 0xffff_ffff, inner.counter);
        let log_path = logs_dir().join(format!("{}-{id}.log", sanitize(&spawn.name)));
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;

        let mut cmd = Command::new(&program);
        cmd.args(&spawn.command[1..])
            .current_dir(&spawn.cwd)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        if !spawn.env.is_empty() {
            cmd.env_clear().envs(&spawn.env);
        }
        cmd.env("PINCH_PROCESS", &spawn.name);
        // SAFETY: setsid is async-signal-safe, which is all pre_exec may call.
        // The child becomes a session and group leader, so `killpg(pid)`
        // reaches everything it spawns and it survives the engine.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to start `{}`", spawn.command.join(" ")))?;
        let pid = child.id();

        inner.sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
            refresh_kind(),
        );
        let start_time_s = inner
            .sys
            .process(Pid::from_u32(pid))
            .map_or(0, |p| p.start_time());

        let (repo, branch) = find_repo(Path::new(&spawn.cwd));
        let info = ProcessInfo {
            agent: Some(spawn.agent.clone()),
            id: ProcessId(id.clone()),
            name: spawn.name.clone(),
            kind: ProcessKind::from_exe(&program),
            origin: Origin::Managed,
            status: Status::Running,
            command: spawn.command.clone(),
            cwd: spawn.cwd.clone(),
            executable: inner
                .sys
                .process(Pid::from_u32(pid))
                .and_then(|p| p.exe())
                .map(|p| p.to_string_lossy().into_owned()),
            app_bundle: None,
            launcher_app: None,
            repo: repo.map(|r| r.to_string_lossy().into_owned()),
            branch,
            owner: spawn.owner.clone(),
            pid: Some(pid),
            ports: Vec::new(),
            started_at_ms: now_ms(),
            last_output_ms: None,
            log_path: Some(log_path.to_string_lossy().into_owned()),
            metrics: None,
        };
        inner.entries.push(Entry {
            info: info.clone(),
            start_time_s,
            kept: false,
            last_activity_ms: now_ms(),
            pending_decision: None,
            env: spawn.env,
        });
        save(inner);

        // Reap the child so it never lingers as a zombie, and learn its exit code.
        let engine = Arc::clone(self);
        let process_id = ProcessId(id);
        std::thread::spawn(move || {
            let code = child.wait().ok().and_then(|s| s.code());
            engine.mark_exited(&process_id, code);
        });
        Ok(info)
    }

    fn mark_exited(&self, id: &ProcessId, code: Option<i32>) {
        let mut inner = self.lock();
        if let Some(entry) = inner.entries.iter_mut().find(|e| &e.info.id == id)
            && entry.info.status.is_alive()
        {
            entry.info.status = Status::Exited { code };
            entry.info.metrics = None;
            entry.info.ports.clear();
        }
        drop_decisions_for(&mut inner, id);
        sync_attention(&self.hooks, &mut inner);
        save(&inner);
    }

    // ---- sampling ---------------------------------------------------------

    /// One sampling pass: refresh metrics and ports, discover unmanaged
    /// listeners, and evaluate the staleness rule.
    pub fn tick(&self) {
        // Socket discovery can be slow; run it before taking the lock.
        let listeners = discovery::listeners();
        let connected_ports = discovery::established_local_ports();

        let mut inner = self.lock();
        refresh_system_metrics(&mut inner);
        inner
            .sys
            .refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind());
        let cpu_totals: HashMap<u32, u64> = inner
            .sys
            .processes()
            .iter()
            .map(|(pid, process)| (pid.as_u32(), process.accumulated_cpu_time()))
            .collect();
        let cpu_advanced: HashSet<u32> = cpu_totals
            .iter()
            .filter(|(pid, total)| {
                inner
                    .cpu_totals
                    .get(pid)
                    .is_some_and(|previous| *total > previous)
            })
            .map(|(pid, _)| *pid)
            .collect();
        inner.cpu_totals = cpu_totals;
        let children = children_map(&inner.sys);
        let now = now_ms();
        let mut managed_pids: HashSet<u32> = HashSet::new();
        let mut changed = false;
        let mut activity_changed = false;

        for i in 0..inner.entries.len() {
            if !inner.entries[i].info.status.is_alive() {
                continue;
            }
            if !is_same_process(&inner.sys, &inner.entries[i]) {
                // Adopted after an engine restart, so there is no waiter thread.
                let id = inner.entries[i].info.id.clone();
                inner.entries[i].info.status = Status::Exited { code: None };
                inner.entries[i].info.metrics = None;
                inner.entries[i].info.ports.clear();
                drop_decisions_for(&mut inner, &id);
                changed = true;
                continue;
            }
            let Some(root) = inner.entries[i].info.pid else {
                continue;
            };
            let tree = descendants(&children, root);
            managed_pids.extend(&tree);

            let (cpu, rss) = tree_usage(&inner.sys, &tree, &cpu_advanced);
            let mut ports: Vec<u16> = listeners
                .iter()
                .filter(|(pid, _)| tree.contains(pid))
                .map(|(_, port)| *port)
                .collect();
            ports.sort_unstable();
            ports.dedup();

            let entry = &mut inner.entries[i];
            entry.info.metrics = Some(Metrics {
                cpu_percent: cpu,
                rss_bytes: rss,
            });
            entry.info.ports = ports;
            entry.info.last_output_ms = entry
                .info
                .log_path
                .as_deref()
                .and_then(|p| fs::metadata(p).ok())
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64);
        }

        // Recent use holds off suggestions even after a connection closes.
        for entry in &mut inner.entries {
            if entry.info.status.is_alive()
                && (entry.info.ports.iter().any(|p| connected_ports.contains(p))
                    || entry.info.metrics.is_some_and(|m| m.cpu_percent >= 5.0))
            {
                entry.last_activity_ms = now;
                activity_changed = true;
            }
        }

        // Require a full quiet interval across output, connections, CPU and ensure.
        let stale_after_ms = self.cfg.stale_after.as_millis() as u64;
        for i in 0..inner.entries.len() {
            let entry = &inner.entries[i];
            if !entry.info.status.is_alive() || entry.info.origin != Origin::Managed {
                continue;
            }
            let last_activity = entry
                .info
                .last_output_ms
                .unwrap_or(0)
                .max(entry.info.started_at_ms)
                .max(entry.last_activity_ms);
            let silent_ms = now.saturating_sub(last_activity);
            let connected = entry.info.ports.iter().any(|p| connected_ports.contains(p));
            let stale = silent_ms >= stale_after_ms && !connected && !entry.kept;

            let id = entry.info.id.clone();
            let has_decision = inner.decisions.iter().any(|d| d.process == id);
            if stale && !has_decision {
                let name = entry.info.name.clone();
                let seq = inner.counter + 1;
                let port = entry
                    .info
                    .ports
                    .first()
                    .map(|p| format!(" (:{p})"))
                    .unwrap_or_default();
                let decision = Decision {
                    id: format!("d{:x}{:x}", now & 0xffff_ffff, seq),
                    process: id,
                    process_name: name.clone(),
                    reason: format!(
                        "{name}{port}: no activity observed for {}. {}",
                        human_duration(silent_ms),
                        memory_estimate(entry.info.metrics)
                    ),
                    created_at_ms: now,
                    quiet_since_ms: Some(last_activity),
                };
                inner.counter = seq;
                inner.entries[i].info.status = Status::Stale;
                inner.decisions.push(decision);
                changed = true;
            } else if !stale && has_decision {
                inner.entries[i].info.status = Status::Running;
                drop_decisions_for(&mut inner, &id);
                changed = true;
            } else if !stale && inner.entries[i].info.status == Status::Stale {
                inner.entries[i].info.status = Status::Running;
                changed = true;
            }
        }

        // Forget exited processes after a day, and cap how many we remember.
        let before = inner.entries.len();
        let day_ago = now.saturating_sub(24 * 60 * 60 * 1000);
        inner.entries.retain(|e| {
            e.info.status.is_alive()
                || e.info.last_output_ms.unwrap_or(0).max(e.info.started_at_ms) >= day_ago
        });
        let dead = inner
            .entries
            .iter()
            .filter(|e| !e.info.status.is_alive())
            .count();
        for _ in 0..dead.saturating_sub(30) {
            if let Some(i) = inner.entries.iter().position(|e| !e.info.status.is_alive()) {
                inner.entries.remove(i);
            }
        }
        changed |= inner.entries.len() != before;

        // Unmanaged: dev-looking listeners that we did not start.
        let own_pid = std::process::id();
        let mut by_pid: HashMap<u32, Vec<u16>> = HashMap::new();
        for (pid, port) in &listeners {
            if !managed_pids.contains(pid) && *pid != own_pid {
                by_pid.entry(*pid).or_default().push(*port);
            }
        }
        let mut unmanaged = Vec::new();
        for (pid, mut ports) in by_pid {
            let Some(proc_) = inner.sys.process(Pid::from_u32(pid)) else {
                continue;
            };
            let exe = proc_.name().to_string_lossy().to_string();
            if !looks_like_dev_server(&exe) {
                continue;
            }
            ports.sort_unstable();
            ports.dedup();
            let tree = descendants(&children, pid);
            let (cpu, rss) = tree_usage(&inner.sys, &tree, &cpu_advanced);
            let cwd = proc_
                .cwd()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let kind = ProcessKind::from_exe(&exe);
            let (repo, branch) = find_repo(Path::new(&cwd));
            let executable = proc_.exe().map(|p| p.to_string_lossy().into_owned());
            let command: Vec<String> = proc_
                .cmd()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            let app_bundle = executable
                .as_deref()
                .and_then(app_bundle_path)
                .or_else(|| command.iter().find_map(|arg| app_bundle_path(arg)));
            let launcher_app = ancestor_app(&inner.sys, Pid::from_u32(pid));
            let label = app_bundle
                .as_deref()
                .and_then(|path| Path::new(path).file_stem())
                .map(|name| name.to_string_lossy().into_owned())
                .or_else(|| {
                    repo.as_deref()
                        .and_then(Path::file_name)
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| kind.label().to_string());
            unmanaged.push(ProcessInfo {
                agent: None,
                id: ProcessId(format!("pid:{pid}:{}", proc_.start_time())),
                name: format!("{label}:{}", ports[0]),
                kind,
                origin: Origin::Unmanaged,
                status: Status::Running,
                command,
                cwd,
                executable,
                app_bundle,
                launcher_app,
                repo: repo.map(|r| r.to_string_lossy().into_owned()),
                branch,
                owner: None,
                pid: Some(pid),
                ports,
                started_at_ms: proc_.start_time() * 1000,
                last_output_ms: None,
                log_path: None,
                metrics: Some(Metrics {
                    cpu_percent: cpu,
                    rss_bytes: rss,
                }),
            });
        }
        unmanaged.sort_by(|a, b| a.ports.cmp(&b.ports));
        inner.unmanaged = unmanaged;

        sync_attention(&self.hooks, &mut inner);
        if changed
            || (activity_changed && now.saturating_sub(inner.last_activity_saved_ms) >= 30_000)
        {
            inner.last_activity_saved_ms = now;
            save(&inner);
        }
    }
}

// ---- helpers --------------------------------------------------------------

fn memory_estimate(metrics: Option<Metrics>) -> String {
    match metrics {
        Some(m) if m.rss_bytes > 0 => format!(
            "About {:.0} MB RAM in its process tree could be freed by stopping it.",
            m.rss_bytes as f64 / 1_048_576.0
        ),
        _ => "Stop it to release its resources?".into(),
    }
}

fn state_path() -> PathBuf {
    home().join("state.json")
}

fn save(inner: &Inner) {
    let path = state_path();
    let tmp = path.with_extension("json.tmp");
    let mut entries = inner.entries.clone();
    for entry in &mut entries {
        entry.pending_decision = inner
            .decisions
            .iter()
            .find(|d| d.process == entry.info.id)
            .cloned();
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&entries)
        && fs::write(&tmp, bytes).is_ok()
    {
        let _ = fs::rename(&tmp, &path);
    }
}

fn sync_attention(hooks: &Hooks, inner: &mut Inner) {
    let ids: Vec<String> = inner.decisions.iter().map(|d| d.id.clone()).collect();
    if ids != inner.last_decisions {
        inner.last_decisions = ids;
        (hooks.decisions_changed)(&inner.decisions);
    }
}

fn drop_decisions_for(inner: &mut Inner, id: &ProcessId) {
    inner.decisions.retain(|d| &d.process != id);
}

fn find_managed(inner: &Inner, target: &str) -> Option<usize> {
    let port: Option<u16> = target.parse().ok();
    let matches = |e: &Entry| {
        e.info.name == target
            || e.info.id.0 == target
            || port.is_some_and(|p| e.info.ports.contains(&p))
    };
    // Prefer a live match so a dead entry never shadows a running one.
    inner
        .entries
        .iter()
        .rposition(|e| e.info.status.is_alive() && matches(e))
        .or_else(|| inner.entries.iter().rposition(matches))
}

fn find_unmanaged(inner: &Inner, target: &str) -> Option<ProcessInfo> {
    let port: Option<u16> = target.parse().ok();
    inner
        .unmanaged
        .iter()
        .find(|u| {
            u.name == target || u.id.0 == target || port.is_some_and(|p| u.ports.contains(&p))
        })
        .cloned()
}

fn repo_key(cwd: &str) -> String {
    find_repo(Path::new(cwd))
        .0
        .map_or_else(|| cwd.to_string(), |r| r.to_string_lossy().into_owned())
}

fn repo_key_of(info: &ProcessInfo) -> String {
    info.repo.clone().unwrap_or_else(|| info.cwd.clone())
}

/// Walk up to the directory holding `.git` and read the branch from HEAD
/// (also works in worktrees, where `.git` is a file pointing at the real one).
fn find_repo(start: &Path) -> (Option<PathBuf>, Option<String>) {
    for dir in start.ancestors() {
        let git = dir.join(".git");
        if !git.exists() {
            continue;
        }
        let git_dir = if git.is_file() {
            fs::read_to_string(&git)
                .ok()
                .and_then(|s| {
                    s.trim()
                        .strip_prefix("gitdir:")
                        .map(|p| PathBuf::from(p.trim()))
                })
                .map(|p| if p.is_absolute() { p } else { dir.join(p) })
        } else {
            Some(git)
        };
        let branch = git_dir
            .and_then(|g| fs::read_to_string(g.join("HEAD")).ok())
            .and_then(|head| {
                head.trim()
                    .strip_prefix("ref: refs/heads/")
                    .map(str::to_owned)
            });
        return (Some(dir.to_path_buf()), branch);
    }
    (None, None)
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn is_same_process(sys: &System, entry: &Entry) -> bool {
    let Some(pid) = entry.info.pid else {
        return false;
    };
    match sys.process(Pid::from_u32(pid)) {
        Some(p) => entry.start_time_s == 0 || p.start_time() == entry.start_time_s,
        None => false,
    }
}

fn children_map(sys: &System) -> HashMap<u32, Vec<u32>> {
    let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
    for (pid, process) in sys.processes() {
        if let Some(parent) = process.parent() {
            map.entry(parent.as_u32()).or_default().push(pid.as_u32());
        }
    }
    map
}

fn descendants(children: &HashMap<u32, Vec<u32>>, root: u32) -> HashSet<u32> {
    let mut seen = HashSet::from([root]);
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        for &child in children.get(&pid).into_iter().flatten() {
            if seen.insert(child) {
                stack.push(child);
            }
        }
    }
    seen
}

fn tree_usage(sys: &System, tree: &HashSet<u32>, cpu_advanced: &HashSet<u32>) -> (f32, u64) {
    tree.iter()
        .filter_map(|pid| sys.process(Pid::from_u32(*pid)))
        .fold((0.0, 0), |(cpu, rss), p| {
            (
                cpu + if cpu_advanced.contains(&p.pid().as_u32()) {
                    p.cpu_usage()
                } else {
                    0.0
                },
                rss + p.memory(),
            )
        })
}

fn looks_like_dev_server(exe: &str) -> bool {
    let lower = exe.to_lowercase();
    lower.starts_with("python")
        || matches!(
            lower.as_str(),
            "node"
                | "bun"
                | "deno"
                | "ruby"
                | "php"
                | "uvicorn"
                | "gunicorn"
                | "next-server"
                | "vite"
        )
}

fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// SIGTERM the whole process group, and SIGKILL it if it hasn't gone in 5s.
fn terminate_group(pid: u32) {
    // SAFETY: plain signal delivery to a group we created with setsid.
    unsafe { libc::killpg(pid as i32, libc::SIGTERM) };
    std::thread::spawn(move || {
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(50));
            if !pid_alive(pid) {
                return;
            }
        }
        // SAFETY: as above; the process ignored SIGTERM for 5s.
        unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
    });
}

fn tail_lines(path: &Path, n: usize) -> std::io::Result<String> {
    const WINDOW: u64 = 256 * 1024;
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(WINDOW);
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // probably cut mid-line
    }
    let skip = lines.len().saturating_sub(n);
    Ok(lines[skip..].join("\n"))
}

pub fn human_duration(ms: u64) -> String {
    if ms < 60_000 {
        return format!("{}s", ms / 1000);
    }
    let mins = ms / 60_000;
    match (mins / 60, mins % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

fn ancestor_app(sys: &System, pid: Pid) -> Option<String> {
    let mut current = sys.process(pid)?.parent()?;
    for _ in 0..64 {
        let process = sys.process(current)?;
        if let Some(bundle) = process
            .exe()
            .and_then(|p| app_bundle_path(&p.to_string_lossy()))
        {
            return Some(bundle);
        }
        let parent = process.parent()?;
        if parent == current {
            break;
        }
        current = parent;
    }
    None
}

fn app_bundle_path(path: &str) -> Option<String> {
    if !Path::new(path).is_absolute() {
        return None;
    }
    Path::new(path)
        .ancestors()
        .filter(|p| p.extension().is_some_and(|ext| ext == "app"))
        .last()
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn discovered_processes_cannot_be_stopped_by_name_id_or_port() {
        let info: ProcessInfo = serde_json::from_value(serde_json::json!({
            "id": "pid:999999:123", "name": "node:3000", "kind": "node",
            "origin": "unmanaged", "status": {"state": "running"},
            "command": [], "cwd": "", "repo": null, "branch": null,
            "owner": null, "pid": 999999, "ports": [3000], "started_at_ms": 123000,
            "last_output_ms": null, "log_path": null, "metrics": null
        }))
        .unwrap();
        let engine = Engine {
            inner: Mutex::new(Inner {
                entries: vec![],
                decisions: vec![],
                unmanaged: vec![info],
                sys: System::new(),
                system_metrics: SystemMetrics::default(),
                cpu_totals: HashMap::new(),
                system_sampled_at: None,
                counter: 0,
                last_decisions: Vec::new(),
                last_activity_saved_ms: 0,
            }),
            cfg: Config {
                stale_after: Duration::from_secs(3600),
            },
            hooks: Hooks {
                decisions_changed: Box::new(|_| {}),
            },
        };
        for target in ["node:3000", "pid:999999:123", "3000"] {
            assert!(
                engine
                    .stop(target)
                    .unwrap_err()
                    .to_string()
                    .contains("read-only")
            );
        }
        let confirmed = engine.snapshot().0[0].clone();
        assert!(engine.stop_discovered(&confirmed).is_err());
        let mut wrong_id = confirmed;
        wrong_id.id = ProcessId("node:3000".into());
        assert!(engine.stop_discovered(&wrong_id).is_err());
        assert_eq!(engine.snapshot().0.len(), 1);

        let mut child = Command::new("sleep").arg("60").spawn().unwrap();
        let pid = child.id();
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
            refresh_kind(),
        );
        let live = sys.process(Pid::from_u32(pid)).unwrap();
        let mut reviewed = engine.snapshot().0[0].clone();
        reviewed.id = ProcessId(format!("pid:{pid}:{}", live.start_time()));
        reviewed.pid = Some(pid);
        reviewed.started_at_ms = live.start_time() * 1000;
        reviewed.command = live
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        reviewed.executable = live.exe().map(|p| p.to_string_lossy().into_owned());
        reviewed.cwd = live
            .cwd()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        engine.lock().unmanaged = vec![reviewed.clone()];
        let mut changed = reviewed.clone();
        changed.command.push("changed".into());
        assert!(engine.stop_discovered(&changed).is_err());
        assert!(child.try_wait().unwrap().is_none());
        engine.stop_discovered(&reviewed).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("confirmed process did not terminate");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn identifies_outer_app_bundle_including_helpers() {
        assert_eq!(
            app_bundle_path("/Applications/Raycast.app/Contents/MacOS/node"),
            Some("/Applications/Raycast.app".into())
        );
        assert_eq!(
            app_bundle_path(
                "/Applications/Example.app/Contents/Frameworks/Helper.app/Contents/MacOS/node"
            ),
            Some("/Applications/Example.app".into())
        );
        assert_eq!(app_bundle_path("/opt/homebrew/bin/node"), None);
        assert_eq!(app_bundle_path("--app=/Applications/Raycast.app"), None);
    }
}
