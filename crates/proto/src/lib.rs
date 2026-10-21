//! Types shared by the engine and its clients (CLI, UI), and the RPC messages
//! they exchange over the engine's Unix socket as newline-delimited JSON.
//! One request per connection: the client writes a line, the engine answers
//! with a line and closes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Bumped on any breaking change to [`Request`] or [`Response`]; clients and
/// the engine refuse to talk across versions.
pub const PROTOCOL_VERSION: u32 = 5;

/// Stable id of a managed process, assigned by the engine.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProcessId(pub String);

/// What launched the process, inferred from its command line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessKind {
    Pnpm,
    Npm,
    Yarn,
    Bun,
    Node,
    Python,
    Cargo,
    Other(String),
}

impl ProcessKind {
    /// Infer the kind from an executable name (`pnpm`, `/usr/bin/python3`, …).
    pub fn from_exe(exe: &str) -> Self {
        let base = exe.rsplit('/').next().unwrap_or(exe);
        match base.to_ascii_lowercase().as_str() {
            "pnpm" | "pnpx" => Self::Pnpm,
            "npm" | "npx" => Self::Npm,
            "yarn" => Self::Yarn,
            "bun" | "bunx" => Self::Bun,
            "node" | "deno" => Self::Node,
            "cargo" => Self::Cargo,
            b if b.starts_with("python") || b == "uv" || b == "uvicorn" => Self::Python,
            _ => Self::Other(base.to_string()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Pnpm => "pnpm",
            Self::Npm => "npm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
            Self::Node => "node",
            Self::Python => "python",
            Self::Cargo => "cargo",
            Self::Other(name) => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Status {
    Running,
    /// Several staleness signals agree; a decision is pending.
    Stale,
    Exited {
        code: Option<i32>,
    },
}

impl Status {
    pub fn is_alive(&self) -> bool {
        !matches!(self, Self::Exited { .. })
    }
}

/// Who started the process; managed ones have logs, unmanaged ones were
/// discovered listening on a port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Managed,
    Unmanaged,
}

/// Explicit attribution supplied by the caller, never inferred from runtime names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Codex,
    Cursor,
    GeminiCli,
    Copilot,
    #[serde(rename = "opencode")]
    OpenCode,
    Zeron,
    Terminal,
}

impl Harness {
    pub const VALUES: &'static [&'static str] = &[
        "claude-code",
        "codex",
        "cursor",
        "gemini-cli",
        "copilot",
        "opencode",
        "zeron",
        "terminal",
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::Cursor => "Cursor",
            Self::GeminiCli => "Gemini CLI",
            Self::Copilot => "Copilot",
            Self::OpenCode => "OpenCode",
            Self::Zeron => "Zeron",
            Self::Terminal => "Terminal",
        }
    }
}

impl std::str::FromStr for Harness {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "claude-code" => Ok(Self::ClaudeCode),
            "codex" => Ok(Self::Codex),
            "cursor" => Ok(Self::Cursor),
            "gemini-cli" => Ok(Self::GeminiCli),
            "copilot" => Ok(Self::Copilot),
            "opencode" => Ok(Self::OpenCode),
            "zeron" => Ok(Self::Zeron),
            "terminal" => Ok(Self::Terminal),
            _ => Err(format!(
                "unknown harness '{value}'; use {}",
                Self::VALUES.join(", ")
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AgentMetadata {
    pub harness: Harness,
    pub thread_id: Option<String>,
    pub agent_id: Option<String>,
}

/// Full paths and IDs form the key so identically named worktrees cannot merge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProcessGroup {
    pub agent: Option<AgentMetadata>,
    pub workspace: String,
    pub branch: Option<String>,
}

impl ProcessGroup {
    pub fn label(&self) -> String {
        let mut parts = vec![
            self.agent
                .as_ref()
                .map_or("Unattributed", |a| a.harness.label())
                .to_string(),
            self.workspace.clone(),
        ];
        if let Some(branch) = &self.branch {
            parts.push(branch.clone());
        }
        if let Some(agent) = &self.agent {
            if let Some(thread) = &agent.thread_id {
                parts.push(format!("thread {thread}"));
            }
            if let Some(id) = &agent.agent_id {
                parts.push(format!("agent {id}"));
            }
        }
        parts.join(" · ")
    }
}

impl ProcessInfo {
    pub fn group(&self) -> ProcessGroup {
        ProcessGroup {
            agent: self.agent.clone(),
            workspace: self.repo.clone().unwrap_or_else(|| self.cwd.clone()),
            branch: self.branch.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    #[serde(default)]
    pub agent: Option<AgentMetadata>,
    pub id: ProcessId,
    pub name: String,
    pub kind: ProcessKind,
    pub origin: Origin,
    pub status: Status,
    pub command: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub executable: Option<String>,
    /// App bundle containing the executable or a command-line path.
    #[serde(default)]
    pub app_bundle: Option<String>,
    /// Nearest ancestor app, distinct from the app containing this executable.
    #[serde(default)]
    pub launcher_app: Option<String>,
    /// Repository root the process belongs to; the scope of `ensure`.
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub owner: Option<String>,
    pub pid: Option<u32>,
    pub ports: Vec<u16>,
    pub started_at_ms: u64,
    /// Last time the process wrote to its log (managed processes only).
    pub last_output_ms: Option<u64>,
    pub log_path: Option<String>,
    pub metrics: Option<Metrics>,
}

/// Summed over the whole process tree (e.g. pnpm → node).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Metrics {
    pub cpu_percent: f32,
    pub rss_bytes: u64,
}

/// Cached whole-computer counters, sampled by the engine independently of clients.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub cpu_percent: Option<f32>,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub swap_used_bytes: u64,
    pub sampled_at_ms: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationStatus {
    #[default]
    Unavailable,
    NotDetermined,
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub enabled: bool,
    pub can_check: bool,
    pub automatic_checks: bool,
    pub available_version: Option<String>,
}

/// Something the engine wants a human to confirm, shown as a notification,
/// in the UI's inbox, and in `pinch inbox`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub process: ProcessId,
    pub process_name: String,
    pub reason: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAnswer {
    Kill,
    /// Keep running and don't ask about this process again.
    Keep,
    /// Ask again in an hour.
    Snooze,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spawn {
    pub agent: AgentMetadata,
    pub name: String,
    pub command: Vec<String>,
    pub cwd: String,
    pub owner: Option<String>,
    /// The caller's environment: the engine runs under launchd or a bare
    /// shell, so it can't rely on its own PATH.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Request {
    Hello {
        version: u32,
    },
    /// Start unless an equivalent process is already running in this repo.
    Ensure(Spawn),
    Run(Spawn),
    /// Processes and pending decisions in one round trip.
    Snapshot,
    CheckForUpdates,
    SetAutomaticUpdateChecks {
        enabled: bool,
    },
    /// Stop by name, id, or port number.
    Stop {
        name: String,
    },
    /// Explicitly confirmed termination of one discovered PID, never its group.
    StopDiscovered {
        process: Box<ProcessInfo>,
    },
    /// Remove a stopped managed process from the registry.
    Clear {
        name: String,
    },
    Restart {
        name: String,
    },
    Logs {
        name: String,
        tail: Option<usize>,
    },
    Answer {
        decision: String,
        answer: DecisionAnswer,
    },
    /// Quit the engine. Managed processes keep running.
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Response {
    Hello {
        version: u32,
    },
    Ensured {
        process: ProcessInfo,
        already_running: bool,
    },
    Process {
        process: ProcessInfo,
    },
    Snapshot {
        processes: Vec<ProcessInfo>,
        decisions: Vec<Decision>,
        #[serde(default)]
        system: SystemMetrics,
        #[serde(default)]
        notifications: NotificationStatus,
        #[serde(default)]
        updates: UpdateStatus,
    },
    Logs {
        text: String,
    },
    Ok,
    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_require_a_known_harness() {
        let spawn = serde_json::json!({
            "name": "web", "command": ["pnpm", "dev"], "cwd": "/tmp", "owner": null
        });
        assert!(serde_json::from_value::<Spawn>(spawn.clone()).is_err());
        let mut spawn = spawn;
        spawn["agent"] = serde_json::json!({"harness": "made-up"});
        assert!(serde_json::from_value::<Spawn>(spawn.clone()).is_err());
        spawn["agent"] = serde_json::json!({"harness": "claude-code", "thread_id": "abc"});
        let parsed: Spawn = serde_json::from_value(spawn).unwrap();
        assert_eq!(parsed.agent.harness, Harness::ClaudeCode);
        assert_eq!(parsed.agent.thread_id.as_deref(), Some("abc"));
        for value in Harness::VALUES {
            let harness: Harness = value.parse().unwrap();
            assert_eq!(serde_json::to_value(harness).unwrap(), *value);
        }
    }

    #[test]
    fn legacy_records_load_and_group_keys_keep_workspaces_and_sessions_separate() {
        let legacy = serde_json::json!({
            "id": "old", "name": "web", "kind": "pnpm", "origin": "managed",
            "status": {"state": "running"}, "command": ["pnpm", "dev"],
            "cwd": "/repos/a/web", "repo": "/repos/a/web", "branch": "main",
            "owner": null, "pid": 1, "ports": [], "started_at_ms": 1,
            "last_output_ms": null, "log_path": null, "metrics": null
        });
        let mut process: ProcessInfo = serde_json::from_value(legacy).unwrap();
        assert!(process.agent.is_none());
        assert!(process.group().label().starts_with("Unattributed"));
        process.agent = Some(AgentMetadata {
            harness: Harness::Codex,
            thread_id: Some("one".into()),
            agent_id: None,
        });
        let original = process.group();
        let restored: ProcessInfo =
            serde_json::from_str(&serde_json::to_string(&process).unwrap()).unwrap();
        assert_eq!(original, restored.group());
        process.repo = Some("/repos/b/web".into());
        assert_ne!(original, process.group());
        process.repo = Some("/repos/a/web".into());
        process.agent.as_mut().unwrap().thread_id = Some("two".into());
        assert_ne!(original, process.group());
        process.agent.as_mut().unwrap().thread_id = Some("one".into());
        process.branch = Some("feature".into());
        assert_ne!(original, process.group());
    }

    #[test]
    fn snapshots_from_older_engines_have_unavailable_system_metrics() {
        let response: Response =
            serde_json::from_str(r#"{"type":"snapshot","processes":[],"decisions":[]}"#).unwrap();
        let Response::Snapshot {
            system,
            notifications,
            ..
        } = response
        else {
            panic!("expected snapshot");
        };
        assert_eq!(notifications, NotificationStatus::Unavailable);
        assert!(system.cpu_percent.is_none());
        assert_eq!(system.memory_total_bytes, 0);
        assert_eq!(system.sampled_at_ms, 0);
    }

    #[test]
    fn snapshots_preserve_cached_system_readings() {
        let response = Response::Snapshot {
            processes: vec![],
            decisions: vec![],
            notifications: NotificationStatus::Enabled,
            updates: UpdateStatus::default(),
            system: SystemMetrics {
                cpu_percent: Some(42.0),
                memory_used_bytes: 12_000,
                memory_total_bytes: 16_000,
                memory_available_bytes: 4_000,
                swap_used_bytes: 500,
                sampled_at_ms: 123_456,
            },
        };
        let json = serde_json::to_string(&response).unwrap();
        let Response::Snapshot { system, .. } = serde_json::from_str(&json).unwrap() else {
            panic!("expected snapshot");
        };
        assert_eq!(system.cpu_percent, Some(42.0));
        assert_eq!(system.memory_used_bytes, 12_000);
        assert_eq!(system.memory_total_bytes, 16_000);
        assert_eq!(system.memory_available_bytes, 4_000);
        assert_eq!(system.swap_used_bytes, 500);
        assert_eq!(system.sampled_at_ms, 123_456);
    }

    #[test]
    fn messages_round_trip_as_tagged_json() {
        let request = Request::Ensure(Spawn {
            agent: AgentMetadata {
                harness: Harness::Codex,
                thread_id: Some("thread-1".into()),
                agent_id: None,
            },
            name: "web".into(),
            command: vec!["pnpm".into(), "dev".into()],
            cwd: "/tmp".into(),
            owner: None,
            env: BTreeMap::new(),
        });
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.starts_with(r#"{"type":"ensure""#));
        serde_json::from_str::<Request>(&json).unwrap();

        let response = Response::Logs {
            text: "ready".into(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(json, r#"{"type":"logs","text":"ready"}"#);
        serde_json::from_str::<Response>(&json).unwrap();
    }

    #[test]
    fn kind_from_exe() {
        assert_eq!(
            ProcessKind::from_exe("/opt/homebrew/bin/pnpm"),
            ProcessKind::Pnpm
        );
        assert_eq!(ProcessKind::from_exe("python3.12"), ProcessKind::Python);
        assert_eq!(
            ProcessKind::from_exe("/Library/Frameworks/Python.framework/Versions/3.13/Python"),
            ProcessKind::Python
        );
        assert_eq!(
            ProcessKind::from_exe("vite"),
            ProcessKind::Other("vite".into())
        );
    }
}
