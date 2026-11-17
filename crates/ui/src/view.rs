//! The popover: a live list of processes with pending decisions on top, and
//! a log view you drill into.

use std::time::Duration;

use gpui::{
    AnyElement, ClickEvent, Context, FocusHandle, FontWeight, IntoElement, KeyDownEvent, Render,
    Rgba, ScrollHandle, SharedString, Window, div, prelude::*, px, size,
};
use pinch_client::{now_ms, onboarding, request, request_autostart};
use pinch_proto::{
    Decision, DecisionAnswer, Origin, ProcessGroup, ProcessInfo, Request, Response, app_name,
};

use crate::process_list::ProcessList;
use crate::resources::{self, Measure};
use crate::theme::Theme;
use pinch_proto::{AppUsage, Close, NotificationStatus, SystemMetrics};

pub const WIDTH: f32 = 400.0;
const LOG_HEIGHT: f32 = 520.0;
const MONO: &str = "Menlo";

#[derive(Default, Clone)]
pub struct Snapshot {
    pub loading: bool,
    pub system: SystemMetrics,
    pub notifications: NotificationStatus,
    pub updates: pinch_proto::UpdateStatus,
    pub processes: Vec<ProcessInfo>,
    pub decisions: Vec<Decision>,
    pub apps: Vec<AppUsage>,
    pub error: Option<String>,
    /// Offer to install the pinch skill into detected agent harnesses.
    pub onboarding: bool,
}

static CACHE: std::sync::Mutex<Option<Snapshot>> = std::sync::Mutex::new(None);

static CLOSED_AT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

pub fn recently_closed() -> bool {
    CLOSED_AT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some_and(|t| t.elapsed() < Duration::from_millis(400))
}

pub fn dismiss(window: &mut Window, cx: &mut Context<Popover>) {
    if std::env::args().any(|a| a == "--resident") {
        // Hiding resigns key status, which calls back here; record only the
        // first close so a following click still sees when it happened.
        if crate::panel::is_visible(window) {
            *CLOSED_AT.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
            crate::panel::hide(window);
        }
    } else {
        cx.quit();
    }
}

pub fn cached() -> Snapshot {
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| Snapshot {
            loading: true,
            ..Default::default()
        })
}

pub fn fetch() -> Snapshot {
    let onboarding = onboarding::needed() && !onboarding::detect().is_empty();
    let snapshot = fetch_engine();
    let snapshot = Snapshot {
        onboarding,
        ..snapshot
    };
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(snapshot.clone());
    snapshot
}

fn fetch_engine() -> Snapshot {
    let response = if std::env::args().any(|arg| arg == "--resident" || arg == "--dev") {
        request(&Request::Snapshot)
    } else {
        request_autostart(&Request::Snapshot)
    };
    match response {
        Ok(Response::Snapshot {
            processes,
            decisions,
            system,
            notifications,
            updates,
            apps,
        }) => Snapshot {
            loading: false,
            system,
            notifications,
            updates,
            processes,
            decisions,
            apps,
            error: None,
            onboarding: false,
        },
        Ok(Response::Error { message }) => Snapshot {
            error: Some(message),
            ..Default::default()
        },
        Ok(_) => Snapshot::default(),
        Err(e) => Snapshot {
            error: Some(format!("{e:#}")),
            ..Default::default()
        },
    }
}

/// Window height that fits the content, within sane bounds.
pub fn height_for(snapshot: &Snapshot) -> f32 {
    list_height(snapshot)
}

fn list_height(snapshot: &Snapshot) -> f32 {
    let list = ProcessList::new(&snapshot.processes, &snapshot.decisions);
    let mut h = 66.0 + 40.0 + 12.0; // header + footer + list margin
    if snapshot.onboarding {
        h += 150.0;
    }
    if snapshot.notifications == NotificationStatus::Disabled {
        h += 84.0;
    }
    let discovered = list.discovered().len();
    let groups = list.managed_groups().len() + usize::from(discovered > 0);
    let servers = list.stale.len() + list.managed.len() + discovered;
    if servers == 0 {
        h += 150.0;
    } else {
        h += 34.0 + groups as f32 * 32.0 + servers as f32 * 52.0;
    }
    let apps = app_entries(snapshot, &list).len();
    if apps > 0 {
        h += 34.0 + apps as f32 * 52.0;
    }
    h.clamp(240.0, 640.0)
}

/// Ports of the servers an app runs itself.
pub struct AppServers {
    ports: Vec<u16>,
}

/// The Apps section: heavy apps, plus each app that runs a server itself,
/// so those servers appear under the app instead of in Servers.
pub fn app_entries(
    snapshot: &Snapshot,
    list: &ProcessList<'_>,
) -> Vec<(AppUsage, Option<AppServers>)> {
    let mut entries: Vec<(AppUsage, Option<AppServers>)> = heavy_apps(&snapshot.apps)
        .into_iter()
        .map(|a| (a.clone(), None))
        .collect();
    let users = |a: &AppUsage, path: &str| {
        a.app_bundle.as_deref() == Some(path)
            && !matches!(a.instance, Some(pinch_proto::Instance::Automated { .. }))
    };
    let mut paths: Vec<&str> = list
        .recognized
        .iter()
        .filter_map(|(_, p)| p.app.as_deref())
        .collect();
    paths.sort_unstable();
    paths.dedup();
    for path in paths {
        let servers: Vec<&ProcessInfo> = list
            .recognized
            .iter()
            .map(|(_, p)| *p)
            .filter(|p| p.app.as_deref() == Some(path))
            .collect();
        let mut ports: Vec<u16> = servers.iter().flat_map(|p| p.ports.clone()).collect();
        ports.sort_unstable();
        ports.dedup();
        let runs = AppServers { ports };
        if let Some(entry) = entries.iter_mut().find(|(a, _)| users(a, path)) {
            entry.1 = Some(runs);
            continue;
        }
        // Light apps are absent from the heavy list; use the full usage
        // reading, or failing that what their servers use.
        let app = snapshot
            .apps
            .iter()
            .find(|a| users(a, path))
            .cloned()
            .unwrap_or_else(|| AppUsage {
                name: app_name(path).to_string(),
                app_bundle: Some(path.to_string()),
                close: None,
                instance: None,
                process_count: servers.len() as u32,
                cpu_percent: servers
                    .iter()
                    .filter_map(|p| p.metrics)
                    .map(|m| m.cpu_percent)
                    .sum(),
                memory_bytes: servers
                    .iter()
                    .filter_map(|p| p.metrics)
                    .map(|m| m.rss_bytes)
                    .sum(),
            });
        entries.push((app, Some(runs)));
    }
    entries
}

/// How many apps the usage screen lists.
const USAGE_ROWS: usize = 10;
/// How many apps the main list's Heavy apps section shows at most.
const HEAVY_ROWS: usize = 4;

/// Apps worth a place beside the dev servers: an agent's browser, or
/// anything using a lot of memory or CPU. Not a full activity monitor.
pub fn heavy_apps(apps: &[AppUsage]) -> Vec<&AppUsage> {
    let mut heavy: Vec<&AppUsage> = apps
        .iter()
        .filter(|a| {
            matches!(a.instance, Some(pinch_proto::Instance::Automated { .. }))
                || a.memory_bytes >= 1 << 30
                || a.cpu_percent >= 25.0
        })
        .collect();
    // Heaviest first, with instances of one app (yours, an agent's) together.
    let app_key = |a: &AppUsage| a.app_bundle.clone().unwrap_or_else(|| a.name.clone());
    let peak = |a: &AppUsage| {
        let key = app_key(a);
        apps.iter()
            .filter(|other| app_key(other) == key)
            .map(|other| other.memory_bytes)
            .max()
            .unwrap_or(0)
    };
    heavy.sort_by_key(|a| {
        (
            std::cmp::Reverse(peak(a)),
            app_key(a),
            std::cmp::Reverse(a.memory_bytes),
        )
    });
    heavy.truncate(HEAVY_ROWS);
    heavy
}

fn usage_height(snapshot: &Snapshot) -> f32 {
    let rows = snapshot.apps.len().clamp(1, USAGE_ROWS) as f32;
    (52.0 + 48.0 + rows * 50.0 + 40.0 + 12.0).clamp(240.0, 640.0)
}

const CONFIGURATION_HEIGHT: f32 = 280.0;

enum Screen {
    List,
    Configuration,
    Logs { name: String, text: String },
    Usage(Measure),
}

pub struct Popover {
    resources: gpui::Entity<SystemMetrics>,
    snapshot: Snapshot,
    screen: Screen,
    focus: FocusHandle,
    log_scroll: ScrollHandle,
    seen_active: bool,
    answering: bool,
    /// The row whose Stop, Quit, or Close is awaiting confirmation.
    confirm: Option<String>,
}

impl Popover {
    pub fn new(snapshot: Snapshot, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);

        // Close on blur, except when the development preview is beside an editor.
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.seen_active = true;
                this.focus.focus(window, cx);
            } else if this.seen_active && !std::env::args().any(|arg| arg == "--dev") {
                dismiss(window, cx);
            }
        })
        .detach();

        // The engine samples system metrics even while the popover is closed.
        // Poll its cached snapshot and the open log once a second.
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let Ok(logs_for) = this.update(cx, |this, _| match &this.screen {
                    Screen::Logs { name, .. } => Some(name.clone()),
                    Screen::List | Screen::Configuration | Screen::Usage(_) => None,
                }) else {
                    break;
                };
                let (snapshot, logs) = cx
                    .background_executor()
                    .spawn(async move {
                        let snapshot = fetch();
                        let logs = logs_for.and_then(|name| fetch_logs(&name));
                        (snapshot, logs)
                    })
                    .await;
                let applied = this.update_in(cx, |this, window, cx| {
                    this.apply(snapshot, logs, window, cx);
                });
                if applied.is_err() {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
        })
        .detach();

        Self {
            resources: cx.new(|_| snapshot.system),
            snapshot,
            screen: Screen::List,
            focus,
            log_scroll: ScrollHandle::new(),
            seen_active: false,
            answering: false,
            confirm: None,
        }
    }

    fn apply(
        &mut self,
        snapshot: Snapshot,
        logs: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let old_height = list_height(&self.snapshot);
        self.resources.update(cx, |current, cx| {
            *current = snapshot.system;
            cx.notify();
        });
        self.snapshot = snapshot;
        match (&mut self.screen, logs) {
            (Screen::Logs { text, .. }, Some(fresh)) if *text != fresh => {
                *text = fresh;
                self.log_scroll.scroll_to_bottom();
            }
            (Screen::List, _) => {
                let height = list_height(&self.snapshot);
                if (height - old_height).abs() > 1.0 {
                    window.resize(size(px(WIDTH), px(height)));
                }
            }
            (Screen::Configuration, _) => {
                window.resize(size(px(WIDTH), px(CONFIGURATION_HEIGHT)));
            }
            (Screen::Usage(_), _) => {
                window.resize(size(px(WIDTH), px(usage_height(&self.snapshot))));
            }
            _ => {}
        }
        cx.notify();
    }

    /// Send a request in the background, then refresh straight away.
    fn send(&mut self, req: Request, window: &mut Window, cx: &mut Context<Self>) {
        let return_to_list = matches!(&req, Request::Stop { .. } | Request::Clear { .. });
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { request(&req) })
                .await;
            let show_list =
                return_to_list && matches!(&result, Ok(Response::Ok | Response::Process { .. }));
            let mut snapshot = cx.background_executor().spawn(async { fetch() }).await;
            match result {
                Ok(Response::Error { message }) => snapshot.error = Some(message),
                Err(error) => snapshot.error = Some(format!("{error:#}")),
                _ => {}
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.apply(snapshot, None, window, cx);
                if show_list {
                    this.back(window, cx);
                }
            });
        })
        .detach();
    }

    /// Install the skill for every detected harness (or just dismiss), then refresh.
    fn finish_onboarding(&mut self, install: bool, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let snapshot = cx
                .background_executor()
                .spawn(async move {
                    if install {
                        for h in onboarding::detect() {
                            let _ = onboarding::install(&h);
                        }
                    }
                    onboarding::mark_done();
                    fetch()
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.apply(snapshot, None, window, cx)
            });
        })
        .detach();
    }

    fn open_logs(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let text = fetch_logs(&name).unwrap_or_default();
        self.screen = Screen::Logs { name, text };
        self.log_scroll.scroll_to_bottom();
        window.resize(size(px(WIDTH), px(LOG_HEIGHT)));
        cx.notify();
    }

    /// Start from the list each time the resident panel is shown again.
    pub fn reopen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm = None;
        self.screen = Screen::List;
        window.resize(size(px(WIDTH), px(list_height(&self.snapshot))));
        cx.notify();
    }

    fn open_usage(&mut self, measure: Measure, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm = None;
        self.screen = Screen::Usage(measure);
        window.resize(size(px(WIDTH), px(usage_height(&self.snapshot))));
        cx.notify();
    }

    fn close_app(&mut self, app: &AppUsage, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm = None;
        match app.close {
            Some(Close::Terminate { pid, started_at_s }) => {
                self.send(Request::TerminateApp { pid, started_at_s }, window, cx);
            }
            _ => {
                if let Err(error) = resources::quit(app) {
                    self.snapshot.error = Some(error);
                }
            }
        }
        cx.notify();
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = Screen::List;
        self.confirm = None;
        window.resize(size(px(WIDTH), px(list_height(&self.snapshot))));
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            if self.confirm.take().is_some() {
                cx.notify();
                return;
            }
            match self.screen {
                Screen::Logs { .. } | Screen::Configuration | Screen::Usage(_) => {
                    self.back(window, cx)
                }
                Screen::List => dismiss(window, cx),
            }
        }
    }

    // ---- pieces -----------------------------------------------------------

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_appearance(gpui::WindowAppearance::Dark);
        let live: Vec<&ProcessInfo> = self
            .snapshot
            .processes
            .iter()
            .filter(|p| p.status.is_alive())
            .collect();
        let rss: u64 = live
            .iter()
            .filter_map(|p| p.metrics)
            .map(|m| m.rss_bytes)
            .sum();
        let summary = if self.snapshot.loading {
            "Loading…".to_string()
        } else if live.is_empty() {
            "all quiet".to_string()
        } else {
            format!("{} running · {} RAM", live.len(), bytes(rss))
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_3()
            .bg(theme.tint)
            .border_b_2()
            .border_color(theme.accent)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(40.))
                    .h(px(40.))
                    .flex_none()
                    .rounded_md()
                    .bg(theme.accent)
                    .child(
                        gpui::svg()
                            .path("brand/daemon.svg")
                            .text_color(theme.on_accent)
                            .w(px(32.))
                            .h(px(32.)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        gpui::svg()
                            .path("brand/wordmark.svg")
                            .w(px(70.))
                            .h(px(19.))
                            .text_color(theme.accent),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .line_height(px(17.))
                            .text_color(theme.muted)
                            .font_family(MONO)
                            .child(summary),
                    ),
            )
            .when_some(pinch_client::installation::variant(), |header, variant| {
                header.child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme.muted)
                        .child(variant),
                )
            })
            .child(self.configuration_button(theme, cx))
    }

    fn onboarding_card(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let found = onboarding::detect();
        let names = found
            .iter()
            .map(|h| h.name)
            .collect::<Vec<_>>()
            .join(" and ");
        div()
            .mx_3()
            .mt_3()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .rounded_lg()
            .bg(theme.card)
            .border_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Your agents start things. Pinch keeps count."),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme.muted)
                    .child(format!(
                        "Found {names}. Install the pinch skill so they start servers through pinch instead of launching duplicates."
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        button("onboard-install", "Install skill", false, theme)
                            .bg(theme.accent).text_color(theme.on_accent).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.finish_onboarding(true, window, cx)
                            }),
                        ),
                    )
                    .child(
                        button("onboard-skip", "Not now", false, theme).on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                this.finish_onboarding(false, window, cx)
                            },
                        )),
                    ),
            )
    }

    fn stop_stale(
        &mut self,
        decisions: Vec<Decision>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.answering {
            return;
        }
        self.answering = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let snapshot = cx
                .background_executor()
                .spawn(async move {
                    let mut errors = Vec::new();
                    for decision in decisions {
                        // A notification or renewed activity may have resolved this item.
                        match request(&Request::Snapshot) {
                            Ok(Response::Snapshot { decisions, .. })
                                if !decisions.iter().any(|d| d.id == decision.id) =>
                            {
                                continue;
                            }
                            _ => {}
                        }
                        let error = match request(&Request::Answer {
                            decision: decision.id,
                            answer: DecisionAnswer::Kill,
                        }) {
                            Ok(Response::Ok) => None,
                            Ok(Response::Error { message }) => Some(message),
                            Err(error) => Some(format!("{error:#}")),
                            _ => Some("Unexpected response".into()),
                        };
                        if let Some(error) = error {
                            errors.push(format!("{}: {error}", decision.process_name));
                        }
                    }
                    let mut snapshot = fetch();
                    if !errors.is_empty() {
                        snapshot.error = Some(errors.join("; "));
                    }
                    snapshot
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.answering = false;
                this.apply(snapshot, None, window, cx);
            });
        })
        .detach();
    }

    fn servers(
        &self,
        list: &ProcessList<'_>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let groups = list.managed_groups();
        let discovered = list.discovered();
        if groups.is_empty() && discovered.is_empty() {
            return Vec::new();
        }
        let decisions: Vec<Decision> = list.stale.iter().map(|(_, d)| (*d).clone()).collect();
        let mut header = section_label("SERVERS", theme)
            .flex()
            .items_center()
            .child(div().flex_1());
        if !decisions.is_empty() {
            let label = format!(
                "Stop {} stale · ~{}",
                decisions.len(),
                bytes(list.stale_ram())
            );
            header = header.child(
                header_action("stop-stale", label, theme.danger, theme)
                    .when(self.answering, |button| button.opacity(0.5))
                    .when(!self.answering, |button| {
                        button.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.stop_stale(decisions.clone(), window, cx);
                        }))
                    }),
            );
        }
        let mut out = vec![header.into_any_element()];
        for (key, entries) in groups {
            out.push(agent_group_label(&key, theme).into_any_element());
            for (p, decision) in entries {
                out.push(self.server_row(p, decision, theme, cx));
            }
        }
        if !discovered.is_empty() {
            out.push(
                div()
                    .px_4()
                    .pt_3()
                    .pb_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.0))
                    .text_color(theme.muted)
                    .child(crate::icons::harness(None, theme.muted))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Started outside Pinch"),
                    )
                    .into_any_element(),
            );
        }
        for p in discovered {
            out.push(self.server_row(p, None, theme, cx));
        }
        out
    }

    /// A server on a port: one Pinch started, or one it discovered.
    fn server_row(
        &self,
        p: &ProcessInfo,
        decision: Option<&Decision>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let managed = p.origin == Origin::Managed;
        let key = format!("server:{}", p.id.0);
        let mut subtitle = Vec::new();
        let (dot, subtitle_color) = if let Some(decision) = decision {
            subtitle.push(decision.quiet_since_ms.map_or_else(
                || "quiet".into(),
                |since| format!("quiet {}", duration(now_ms().saturating_sub(since))),
            ));
            (Dot::Filled(theme.stale), theme.stale)
        } else if managed {
            subtitle.push(p.kind.label().to_string());
            subtitle.push(format!(
                "up {}",
                duration(now_ms().saturating_sub(p.started_at_ms))
            ));
            (Dot::Filled(theme.running), theme.faint)
        } else {
            subtitle.push(p.kind.label().to_string());
            if p.recognized {
                subtitle.push("✓ expected".into());
            }
            if let Some(repo) = p.repo.as_deref().and_then(|r| r.rsplit('/').next()) {
                subtitle.push(match &p.branch {
                    Some(b) => format!("{repo}@{b}"),
                    None => repo.to_string(),
                });
            }
            let color = if p.recognized {
                theme.faint
            } else {
                theme.muted
            };
            (Dot::Hollow(color), theme.faint)
        };
        subtitle.extend(p.ports.first().map(|port| format!(":{port}")));
        let (primary, secondary) = match p.metrics {
            Some(m) => (bytes(m.rss_bytes), format!("{:.0}% CPU", m.cpu_percent)),
            None => ("starting".into(), String::new()),
        };

        let mut actions = Vec::new();
        if !managed {
            let process = p.clone();
            let label = if p.recognized {
                "Unrecognize this process"
            } else {
                "Recognize until this process exits"
            };
            let icon = if p.recognized {
                "icons/undo.svg"
            } else {
                "icons/check.svg"
            };
            actions.push(
                icon_button(
                    SharedString::from(format!("recognize-{}", p.id.0)),
                    icon,
                    label,
                    theme.text,
                    theme,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    let process = Box::new(process.clone());
                    this.send(
                        if process.recognized {
                            Request::Unrecognize { process }
                        } else {
                            Request::Recognize { process }
                        },
                        window,
                        cx,
                    );
                }))
                .into_any_element(),
            );
        }
        let target = p.id.0.clone();
        let confirm_key = key.clone();
        actions.push(
            icon_button(
                SharedString::from(format!("stop-{}", p.id.0)),
                "icons/x.svg",
                if managed { "Stop" } else { "Stop…" },
                theme.danger,
                theme,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                if managed {
                    this.send(
                        Request::Stop {
                            name: target.clone(),
                        },
                        window,
                        cx,
                    );
                } else {
                    this.confirm = Some(confirm_key.clone());
                    cx.notify();
                }
            }))
            .into_any_element(),
        );

        let confirm = (self.confirm.as_deref() == Some(key.as_str())).then(|| {
            let process = p.clone();
            self.confirm_buttons(&key, "Stop", theme, cx, move |this, window, cx| {
                this.send(
                    Request::StopDiscovered {
                        process: Box::new(process.clone()),
                    },
                    window,
                    cx,
                );
            })
        });

        let name = p.name.clone();
        let row = render_row(
            Row {
                id: SharedString::from(key),
                dot,
                icon: crate::icons::runtime(&p.kind),
                title: p.name.clone(),
                subtitle: subtitle.join(" · "),
                subtitle_color,
                details: (!managed).then(|| p.clone()),
                primary,
                secondary,
                share: None,
            },
            actions,
            confirm,
            theme,
        );
        if managed {
            row.cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.open_logs(name.clone(), window, cx);
                }))
                .into_any_element()
        } else {
            row.into_any_element()
        }
    }

    fn apps(
        &self,
        list: &ProcessList<'_>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let entries = app_entries(&self.snapshot, list);
        if entries.is_empty() {
            return Vec::new();
        }
        let header = section_label("APPS", theme)
            .flex()
            .items_center()
            .child(div().flex_1())
            .child(
                header_action("apps-all", "All apps ›".into(), theme.muted, theme).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_usage(Measure::Memory, window, cx)
                    }),
                ),
            );
        let mut out = vec![header.into_any_element()];
        for (index, (app, runs)) in entries.iter().enumerate() {
            out.push(self.app_row(
                "apps",
                index,
                app,
                Measure::Memory,
                None,
                runs.as_ref(),
                theme,
                cx,
            ));
        }
        out
    }

    /// Cancel and a red confirm button, shown in place of a row's numbers.
    fn confirm_buttons(
        &self,
        key: &str,
        label: &'static str,
        theme: Theme,
        cx: &mut Context<Self>,
        on_confirm: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        div()
            .flex()
            .flex_none()
            .gap_1()
            .child(
                button(
                    SharedString::from(format!("{key}-cancel")),
                    "Cancel",
                    false,
                    theme,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.confirm = None;
                    cx.notify();
                })),
            )
            .child(
                button(
                    SharedString::from(format!("{key}-confirm")),
                    label,
                    true,
                    theme,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.confirm = None;
                    on_confirm(this, window, cx);
                })),
            )
            .into_any_element()
    }

    fn list(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let list = ProcessList::new(&self.snapshot.processes, &self.snapshot.decisions);
        let mut body = div()
            .id("list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .mb_3();
        if self.snapshot.onboarding {
            body = body.child(self.onboarding_card(theme, cx));
        }
        if self.snapshot.notifications == NotificationStatus::Disabled {
            body = body.child(
                div().mx_3().mb_2().p_3().rounded_lg().bg(theme.card)
                    .flex().flex_col().gap_2()
                    .child(div().text_size(px(12.0)).text_color(theme.muted)
                        .child("Notifications are off. Stale-process suggestions will appear here."))
                    .child(button("notification-settings", "Notification settings", false, theme)
                        .on_click(|_, _, _| {
                            let _ = std::process::Command::new("open")
                                .arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension")
                                .spawn();
                        })),
            );
        }
        if let Some(error) = &self.snapshot.error {
            body = body.child(
                div()
                    .px_4()
                    .py_2()
                    .text_size(px(12.0))
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }
        let servers = self.servers(&list, theme, cx);
        if servers.is_empty() && self.snapshot.error.is_none() && !self.snapshot.loading {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .py_8()
                    .child(
                        gpui::svg()
                            .path("brand/daemon.svg")
                            .w(px(48.))
                            .h(px(48.))
                            .text_color(theme.muted),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child("Nothing on the clock."),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme.muted)
                            .child("Start a server through Pinch to track it here."),
                    ),
            );
        }
        body.children(servers)
            .children(self.apps(&list, theme, cx))
            .into_any_element()
    }

    fn logs_view(
        &self,
        name: &str,
        text: &str,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let process = self
            .snapshot
            .processes
            .iter()
            .rev()
            .find(|p| p.name == name && p.origin == Origin::Managed);
        let alive = process.is_some_and(|p| p.status.is_alive());
        let port = process
            .and_then(|p| p.ports.first())
            .map(|p| format!(":{p}"));
        let stop_name = name.to_string();
        let restart_name = name.to_string();

        let mut actions = div().flex().flex_none().gap_2();
        if alive {
            actions = actions.child(
                button("log-stop", "Stop", false, theme)
                    .bg(theme.border)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.send(
                            Request::Stop {
                                name: stop_name.clone(),
                            },
                            window,
                            cx,
                        );
                    })),
            );
        }
        if let Some(process) = process.filter(|p| !p.status.is_alive()) {
            let target = process.id.0.clone();
            actions = actions.child(button("log-remove", "Remove", true, theme).on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.send(
                        Request::Clear {
                            name: target.clone(),
                        },
                        window,
                        cx,
                    );
                }),
            ));
        }
        actions = actions.child(button("log-restart", "Restart", false, theme).on_click(
            cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.send(
                    Request::Restart {
                        name: restart_name.clone(),
                    },
                    window,
                    cx,
                );
            }),
        ));

        let lines: Vec<AnyElement> = if text.trim().is_empty() {
            vec![
                div()
                    .text_color(theme.faint)
                    .child("No output yet.")
                    .into_any_element(),
            ]
        } else {
            text.lines()
                .map(|l| {
                    div()
                        .whitespace_nowrap()
                        .child(strip_ansi(l))
                        .into_any_element()
                })
                .collect()
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .pt_3()
                    .pb_2()
                    .child(
                        div()
                            .id("back")
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .rounded_md()
                            .px_2()
                            .py_1()
                            .hover(|s| s.bg(theme.hover))
                            .child(div().text_color(theme.muted).child("‹"))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name.to_string()),
                            )
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(px(12.0))
                                    .text_color(theme.muted)
                                    .child(port.unwrap_or_default()),
                            )
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.back(window, cx)
                            })),
                    )
                    .child(actions),
            )
            .when_some(self.snapshot.error.clone(), |view, error| {
                view.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_size(px(12.))
                        .text_color(theme.danger)
                        .child(error),
                )
            })
            .child(
                div()
                    .id("logs")
                    .flex_1()
                    .min_h_0()
                    .mx_3()
                    .mb_2()
                    .p_2()
                    .rounded_md()
                    .bg(theme.card)
                    .border_1()
                    .border_color(theme.border)
                    .overflow_scroll()
                    .track_scroll(&self.log_scroll)
                    .font_family(MONO)
                    .text_size(px(10.5))
                    .line_height(px(15.0))
                    .children(lines),
            )
            .into_any_element()
    }

    fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match request(&Request::CheckForUpdates) {
            Ok(Response::Ok) => dismiss(window, cx),
            Ok(Response::Error { message }) => {
                self.snapshot.error = Some(message);
                cx.notify();
            }
            Err(error) => {
                self.snapshot.error = Some(error.to_string());
                cx.notify();
            }
            _ => {}
        }
    }

    fn configuration(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let updates = &self.snapshot.updates;
        div().flex().flex_col().flex_1().min_h_0()
            .child(div().px_3().pt_3().pb_2()
                .child(div().id("configuration-back").flex().items_center().gap_2()
                    .cursor_pointer().rounded_md().px_2().py_1().hover(|s| s.bg(theme.hover))
                    .child(div().text_color(theme.muted).child("‹"))
                    .child(div().text_size(px(14.0)).font_weight(FontWeight::SEMIBOLD).child("Configuration"))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.back(window, cx)))))
            .child(div().px_4().py_3().flex().flex_col().gap_3()
                .child(div().flex().flex_col().gap_1()
                .child(div().flex().items_center().justify_between()
                    .child(div().text_size(px(13.0)).child("Automatic updates"))
                    .child(div().id("automatic-update-checks")
                        .w(px(34.0)).h(px(20.0)).rounded_full().px(px(3.0))
                        .flex().items_center()
                        .bg(if updates.automatic_checks { theme.accent } else { theme.border })
                        .when(updates.automatic_checks, |s| s.justify_end())
                        .child(div().size(px(14.0)).rounded_full().bg(theme.text))
                        .when(updates.enabled, |toggle| toggle.cursor_pointer().on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                let enabled = !this.snapshot.updates.automatic_checks;
                                match request(&Request::SetAutomaticUpdateChecks { enabled }) {
                                    Ok(Response::Ok) => this.snapshot.updates.automatic_checks = enabled,
                                    Ok(Response::Error { message }) => this.snapshot.error = Some(message),
                                    Err(error) => this.snapshot.error = Some(error.to_string()),
                                    _ => {},
                                }
                                cx.notify();
                            })))
                        .when(!updates.enabled, |toggle| toggle.opacity(0.4))))
                .child(div().text_size(px(12.0)).text_color(theme.faint)
                    .child(if updates.enabled {
                        "Check for new versions automatically. Installing an update always asks for approval."
                    } else {
                        "Updates are unavailable in this build."
                    })))
                .when(updates.enabled, |body| body.child(
                    div().flex().child(button("configuration-check-updates", "Check for Updates…", false, theme)
                        .when(updates.can_check || updates.available_version.is_some(), |button| button.on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| this.check_for_updates(window, cx))))
                        .when(!updates.can_check && updates.available_version.is_none(), |button| button.opacity(0.4)))))
                .when_some(self.snapshot.error.clone(), |body, error| body.child(
                    div().text_size(px(12.0)).text_color(theme.danger).child(error))))
            .into_any_element()
    }

    /// Whole-computer usage by app: what to look at when the meters are red.
    fn usage(&self, measure: Measure, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let system = &self.snapshot.system;
        let mut apps: Vec<&AppUsage> = self.snapshot.apps.iter().collect();
        match measure {
            Measure::Cpu => {
                // Idle apps are noise when looking for what burns CPU.
                apps.retain(|a| a.cpu_percent >= 0.5);
                apps.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent))
            }
            Measure::Memory => apps.sort_by_key(|a| std::cmp::Reverse(a.memory_bytes)),
        }
        apps.truncate(USAGE_ROWS);
        let empty = apps.is_empty();
        let value = |app: &AppUsage| match measure {
            Measure::Cpu => app.cpu_percent as f64,
            Measure::Memory => app.memory_bytes as f64,
        };
        let max = apps.first().map_or(0.0, |a| value(a)).max(f64::EPSILON);
        let summary = match measure {
            Measure::Cpu => system.cpu_percent.map_or_else(
                || "Measuring…".to_string(),
                |p| format!("{p:.0}% of total CPU capacity in use · 100% per app is one core"),
            ),
            Measure::Memory => {
                let mut parts = vec![format!(
                    "{} of {} used",
                    resources::bytes(system.memory_used_bytes),
                    resources::bytes(system.memory_total_bytes)
                )];
                if system.swap_used_bytes > 0 {
                    parts.push(format!("{} swap", resources::bytes(system.swap_used_bytes)));
                }
                parts.extend(resources::pressure_label(system.memory_pressure).map(str::to_owned));
                parts.join(" · ")
            }
        };
        let level = match measure {
            Measure::Cpu => resources::level_color(system.cpu_percent, None, theme),
            Measure::Memory => resources::level_color(
                resources::memory_percent(system),
                system.memory_pressure,
                theme,
            ),
        };
        let rows: Vec<AnyElement> = apps
            .into_iter()
            .enumerate()
            .map(|(index, app)| {
                let share = (value(app) / max).clamp(0.0, 1.0) as f32;
                self.app_row("usage", index, app, measure, Some(share), None, theme, cx)
            })
            .collect();

        let tab = |id: &'static str, label: &'static str, target: Measure| {
            let active = measure == target;
            div()
                .id(id)
                .px_2()
                .py(px(2.0))
                .rounded_md()
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .when(active, |t| t.bg(theme.accent).text_color(theme.on_accent))
                .when(!active, |t| {
                    t.text_color(theme.muted).hover(|s| s.bg(theme.hover))
                })
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.open_usage(target, window, cx)
                }))
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .id("usage-back")
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .rounded_md()
                            .px_2()
                            .py_1()
                            .hover(|s| s.bg(theme.hover))
                            .child(div().text_color(theme.muted).child("‹"))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("What's using it"),
                            )
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.back(window, cx)
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(tab("usage-cpu", "CPU", Measure::Cpu))
                            .child(tab("usage-memory", "Memory", Measure::Memory)),
                    ),
            )
            .child(
                div()
                    .px_4()
                    .pb_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.0))
                    .text_color(theme.muted)
                    .child(dot(level))
                    .child(div().min_w_0().truncate().child(summary)),
            )
            .when_some(self.snapshot.error.clone(), |view, error| {
                view.child(
                    div()
                        .px_4()
                        .pb_2()
                        .text_size(px(12.))
                        .text_color(theme.danger)
                        .child(error),
                )
            })
            .child(
                div()
                    .id("usage-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb_2()
                    .when(empty, |list| {
                        list.child(
                            div()
                                .px_4()
                                .py_3()
                                .text_size(px(12.0))
                                .text_color(theme.faint)
                                .child(if self.snapshot.apps.is_empty() {
                                    "Measuring…"
                                } else {
                                    "Nothing is using noticeable CPU."
                                }),
                        )
                    })
                    .children(rows),
            )
            .into_any_element()
    }

    /// One app in the usage screen or the main list's Apps section.
    /// `share` draws a bar relative to the heaviest app shown.
    #[allow(clippy::too_many_arguments)]
    fn app_row(
        &self,
        prefix: &'static str,
        index: usize,
        app: &AppUsage,
        measure: Measure,
        share: Option<f32>,
        runs: Option<&AppServers>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = format!("{prefix}-{index}");
        let (primary, secondary) = match measure {
            Measure::Cpu => (
                format!("{:.0}%", app.cpu_percent),
                resources::bytes(app.memory_bytes),
            ),
            Measure::Memory => (
                resources::bytes(app.memory_bytes),
                format!("{:.0}% CPU", app.cpu_percent),
            ),
        };
        let automated = matches!(app.instance, Some(pinch_proto::Instance::Automated { .. }));
        let mut subtitle: Vec<String> = app.instance.iter().map(|i| i.label()).collect();
        subtitle.push(match app.process_count {
            1 => "1 process".to_string(),
            n => format!("{n} processes"),
        });
        if let Some(runs) = runs {
            let ports: Vec<String> = runs.ports.iter().map(|p| format!(":{p}")).collect();
            if !ports.is_empty() {
                subtitle.push(format!("runs {}", ports.join(" ")));
            }
        }
        let dot = if automated {
            Dot::Filled(theme.stale)
        } else if runs.is_some() && heavy_apps(std::slice::from_ref(app)).is_empty() {
            Dot::Filled(theme.faint)
        } else {
            Dot::Filled(theme.running)
        };
        let close_label = match app.close {
            Some(Close::Terminate { .. }) => "Close",
            _ => "Quit",
        };

        let mut actions = Vec::new();
        if app.close.is_some() {
            let confirm_key = key.clone();
            let tooltip = if automated {
                "Close the agent's browser…".to_string()
            } else {
                format!("Quit {}…", app.name)
            };
            actions.push(
                icon_button(
                    SharedString::from(format!("{key}-close")),
                    "icons/x.svg",
                    tooltip,
                    theme.danger,
                    theme,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.confirm = Some(confirm_key.clone());
                    cx.notify();
                }))
                .into_any_element(),
            );
        }
        let confirm = (self.confirm.as_deref() == Some(key.as_str())).then(|| {
            let target = app.clone();
            self.confirm_buttons(&key, close_label, theme, cx, move |this, window, cx| {
                this.close_app(&target, window, cx)
            })
        });
        render_row(
            Row {
                id: SharedString::from(key),
                dot,
                icon: crate::icons::app(app.app_bundle.as_deref(), &app.name),
                title: app.name.clone(),
                subtitle: subtitle.join(" · "),
                subtitle_color: if automated { theme.stale } else { theme.faint },
                details: None,
                primary,
                secondary,
                share,
            },
            actions,
            confirm,
            theme,
        )
        .into_any_element()
    }

    fn configuration_button(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("configuration")
            .size(px(24.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .hover(|s| s.bg(theme.hover))
            .child(
                gpui::svg()
                    .path("icons/settings.svg")
                    .size(px(18.0))
                    .text_color(theme.muted),
            )
            .tooltip(|_, cx| cx.new(|_| TextTooltip("Configuration".into())).into())
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.confirm = None;
                this.screen = Screen::Configuration;
                window.resize(size(px(WIDTH), px(CONFIGURATION_HEIGHT)));
                cx.notify();
            }))
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_appearance(gpui::WindowAppearance::Dark);
        div()
            .bg(theme.tint)
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .px_4()
            .py_2()
            .text_size(px(11.0))
            .text_color(theme.faint)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .id("quit")
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme.text))
                            .child("Quit pinch")
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                let _ = request(&Request::Shutdown);
                                cx.quit();
                            })),
                    )
                    .when_some(
                        self.snapshot.updates.available_version.as_ref(),
                        |actions, _| {
                            actions.child(
                                div()
                                    .id("check-updates")
                                    .cursor_pointer()
                                    .text_color(theme.accent)
                                    .child("Update…")
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.check_for_updates(window, cx);
                                    })),
                            )
                        },
                    ),
            )
            .child({
                let this = cx.entity().downgrade();
                resources::indicators(
                    &self.resources,
                    theme,
                    move |measure, window, cx| {
                        let _ = this.update(cx, |this, cx| this.open_usage(measure, window, cx));
                    },
                    cx,
                )
            })
    }
}

impl Render for Popover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_appearance(window.appearance());
        let content = match &self.screen {
            Screen::List => div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(self.header(cx))
                .child(self.list(theme, cx))
                .into_any_element(),
            Screen::Logs { name, text } => self.logs_view(name, text, theme, cx),
            Screen::Configuration => self.configuration(theme, cx),
            Screen::Usage(measure) => self.usage(*measure, theme, cx),
        };

        div()
            .id("root")
            .key_context("Popover")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.tint)
            .text_color(theme.text)
            .font_family(".SystemUIFont")
            .child(content)
            .child(self.footer(cx))
    }
}

// ---- small helpers --------------------------------------------------------

fn dot(color: Rgba) -> impl IntoElement {
    div().w(px(8.0)).h(px(8.0)).rounded_full().bg(color)
}

/// Filled for something running as expected or needing attention; hollow
/// for a server Pinch did not start.
enum Dot {
    Filled(Rgba),
    Hollow(Rgba),
}

/// What every server and app row shows, in the same places.
struct Row {
    id: SharedString,
    dot: Dot,
    icon: AnyElement,
    title: String,
    subtitle: String,
    subtitle_color: Rgba,
    /// Shown as a tooltip on the subtitle.
    details: Option<ProcessInfo>,
    primary: String,
    secondary: String,
    share: Option<f32>,
}

/// The shared row: dot, icon, name and subtitle, then the numbers, with
/// actions revealed on hover. `confirm` replaces actions and numbers.
fn render_row(
    row: Row,
    actions: Vec<AnyElement>,
    confirm: Option<AnyElement>,
    theme: Theme,
) -> gpui::Stateful<gpui::Div> {
    let group = row.id.clone();
    let dot = div().w(px(8.0)).h(px(8.0)).flex_none().rounded_full();
    let dot = match row.dot {
        Dot::Filled(color) => dot.bg(color),
        Dot::Hollow(color) => dot.border_1().border_color(color),
    };
    let numbers = div()
        .w(px(64.0))
        .flex()
        .flex_col()
        .items_end()
        .font_family(MONO)
        .child(div().text_size(px(12.0)).child(row.primary))
        .child(
            div()
                .text_size(px(10.5))
                .text_color(theme.faint)
                .child(row.secondary),
        );
    let right = confirm.unwrap_or_else(|| {
        let hover_group = group.clone();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .when(!actions.is_empty(), |right| {
                right.child(
                    div()
                        .flex()
                        .gap_1()
                        .invisible()
                        .group_hover(hover_group, |s| s.visible())
                        .children(actions),
                )
            })
            .child(numbers)
            .into_any_element()
    });
    let details = row.details;
    div()
        .id(row.id.clone())
        .group(group)
        .mx_2()
        .px_2()
        .py(px(6.0))
        .rounded_md()
        .flex()
        .flex_col()
        .gap_1()
        .hover(|s| s.bg(theme.hover))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(dot)
                .child(row.icon)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .truncate()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .child(row.title),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("{}-subtitle", row.id)))
                                .truncate()
                                .text_size(px(11.0))
                                .text_color(row.subtitle_color)
                                .child(row.subtitle)
                                .when_some(details, |subtitle, details| {
                                    subtitle.tooltip(move |_, cx| {
                                        cx.new(|_| ProcessDetails(details.clone())).into()
                                    })
                                }),
                        ),
                )
                .child(right),
        )
        // Full row width, so bars compare across rows.
        .when_some(row.share, |el, share| {
            el.child(
                div()
                    .h(px(3.0))
                    .w_full()
                    .rounded_full()
                    .bg(theme.border)
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(share))
                            .rounded_full()
                            .bg(theme.muted),
                    ),
            )
        })
}

fn header_action(
    id: &'static str,
    label: String,
    color: Rgba,
    theme: Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_2()
        .rounded_md()
        .font_family(".SystemUIFont")
        .font_weight(FontWeight::NORMAL)
        .text_size(px(11.0))
        .text_color(color)
        .cursor_pointer()
        .hover(|s| s.bg(theme.hover))
        .child(label)
}

/// Where a discovered server came from, before stopping it.
struct ProcessDetails(ProcessInfo);

impl Render for ProcessDetails {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_appearance(window.appearance());
        let p = &self.0;
        let unknown = |v: Option<&str>| v.unwrap_or("Unknown").to_string();
        let lines = [
            format!("PID {}", p.pid.unwrap_or_default()),
            format!("App: {}", unknown(p.app_bundle.as_deref())),
            format!("Launched via: {}", unknown(p.launcher_app.as_deref())),
            format!("Executable: {}", unknown(p.executable.as_deref())),
            format!("Command: {}", p.command.join(" ")),
            format!("Directory: {}", p.cwd),
        ];
        div()
            .p_3()
            .max_w(px(380.0))
            .rounded_lg()
            .bg(theme.tooltip)
            .border_1()
            .border_color(theme.border)
            .shadow_md()
            .text_size(px(11.0))
            .text_color(theme.text)
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Started outside Pinch"),
            )
            .child(
                div().text_color(theme.muted).child(
                    "Stop sends SIGTERM to this PID only; it may restart or lose unsaved work.",
                ),
            )
            .children(lines.into_iter().map(|line| div().child(line)))
    }
}

fn agent_group_label(group: &ProcessGroup, theme: Theme) -> impl IntoElement {
    let workspace = std::path::Path::new(&group.workspace)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| group.workspace.clone());
    let details = group.clone();
    let mut header = div()
        .id(SharedString::from(group.label()))
        .px_4()
        .pt_3()
        .pb_1()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(11.0))
        .text_color(theme.muted)
        .child(crate::icons::harness(
            group.agent.as_ref().map(|a| a.harness),
            theme.muted,
        ))
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .child(workspace),
        );
    if let Some(thread) = group.agent.as_ref().and_then(|a| a.thread_id.as_ref()) {
        let short = if thread.chars().count() > 20 {
            format!("{}…", thread.chars().take(8).collect::<String>())
        } else {
            thread.clone()
        };
        header = header
            .child(div().flex_none().text_color(theme.faint).child("/"))
            .child(div().min_w_0().truncate().child(short));
    }
    header.tooltip(move |_, cx| cx.new(|_| AgentGroupDetails(details.clone())).into())
}

struct TextTooltip(SharedString);

impl Render for TextTooltip {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_appearance(window.appearance());
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(theme.tooltip)
            .border_1()
            .border_color(theme.border)
            .text_color(theme.text)
            .text_size(px(11.0))
            .child(self.0.clone())
    }
}

struct AgentGroupDetails(ProcessGroup);

impl Render for AgentGroupDetails {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_appearance(window.appearance());
        let group = &self.0;
        let mut lines = vec![group.workspace.clone()];
        if let Some(branch) = &group.branch {
            lines.push(format!("Branch: {branch}"));
        }
        if let Some(agent) = &group.agent {
            if let Some(thread) = &agent.thread_id {
                lines.push(format!("Thread: {thread}"));
            }
            if let Some(id) = &agent.agent_id {
                lines.push(format!("Agent: {id}"));
            }
        }
        div()
            .p_3()
            .max_w(px(380.0))
            .rounded_lg()
            .bg(theme.tooltip)
            .border_1()
            .border_color(theme.border)
            .shadow_md()
            .text_size(px(11.0))
            .text_color(theme.text)
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div().font_weight(FontWeight::SEMIBOLD).child(
                    group
                        .agent
                        .as_ref()
                        .map_or("Unattributed", |a| a.harness.label()),
                ),
            )
            .children(lines.into_iter().map(|line| div().child(line)))
    }
}

fn section_label(text: impl Into<SharedString>, theme: Theme) -> gpui::Div {
    div()
        .font_family(MONO)
        .px_4()
        .pt_3()
        .pb_1()
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.faint)
        .child(text.into())
}

fn icon_button(
    id: impl Into<gpui::ElementId>,
    icon: &'static str,
    tooltip: impl Into<SharedString>,
    hover: Rgba,
    theme: Theme,
) -> gpui::Stateful<gpui::Div> {
    let tooltip = tooltip.into();
    div()
        .id(id.into())
        .group("icon-button")
        .size(px(22.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .hover(|s| s.bg(theme.hover))
        .child(
            gpui::svg()
                .path(icon)
                .size(px(14.0))
                .text_color(theme.muted)
                .group_hover("icon-button", move |s| s.text_color(hover)),
        )
        .tooltip(move |_, cx| cx.new(|_| TextTooltip(tooltip.clone())).into())
}

fn button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    danger: bool,
    theme: Theme,
) -> gpui::Stateful<gpui::Div> {
    let (bg, fg) = if danger {
        (theme.danger, theme.on_danger)
    } else {
        (theme.hover, theme.text)
    };
    div()
        .id(id.into())
        .px_3()
        .py_1()
        .rounded_md()
        .bg(bg)
        .text_color(fg)
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(|s| s.opacity(0.8))
        .child(label)
}

fn fetch_logs(name: &str) -> Option<String> {
    match request(&Request::Logs {
        name: name.to_string(),
        tail: Some(300),
    }) {
        Ok(Response::Logs { text }) => Some(text),
        _ => None,
    }
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

fn bytes(n: u64) -> String {
    let mb = n as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.1} GiB", mb / 1024.0)
    } else {
        format!("{mb:.0} MiB")
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
