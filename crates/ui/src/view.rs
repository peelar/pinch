//! The popover: a live list of processes with pending decisions on top, and
//! a log view you drill into.

use std::time::Duration;

use gpui::{
    AnyElement, ClickEvent, Context, FocusHandle, FontWeight, IntoElement, KeyDownEvent, Render,
    Rgba, ScrollHandle, SharedString, Window, div, prelude::*, px, size,
};
use pinch_client::{now_ms, onboarding, request, request_autostart};
use pinch_proto::{
    Decision, DecisionAnswer, Origin, ProcessGroup, ProcessInfo, RecognizedApp, Request, Response,
    Status, app_name,
};

use crate::process_list::ProcessList;
use crate::resources;
use crate::theme::Theme;
use pinch_proto::{NotificationStatus, SystemMetrics};

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
    pub recognized: Vec<RecognizedApp>,
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

fn dismiss(window: &mut Window, cx: &mut Context<Popover>) {
    *CLOSED_AT.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
    if std::env::args().any(|a| a == "--resident") {
        window.remove_window();
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
            recognized,
        }) => Snapshot {
            loading: false,
            system,
            notifications,
            updates,
            processes,
            decisions,
            recognized,
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
    let list = ProcessList::new(&snapshot.processes, &snapshot.decisions);
    let managed = list.managed.len() as f32;
    let unmanaged = list.unmanaged.len() as f32;
    let mut h = 66.0 + 40.0 + 12.0; // header + footer + list margin
    if snapshot.onboarding {
        h += 150.0;
    }
    if snapshot.notifications == NotificationStatus::Disabled {
        h += 84.0;
    }
    if !list.stale.is_empty() {
        h += 38.0 + list.stale.len() as f32 * 60.0;
    }
    if managed + unmanaged == 0.0 && list.stale.is_empty() {
        h += 150.0;
    }
    if managed > 0.0 {
        let groups = list
            .managed
            .iter()
            .map(|(_, p)| p.group())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        h += 30.0 + groups as f32 * 32.0 + managed * 68.0;
    }
    if unmanaged > 0.0 {
        h += 34.0 + unmanaged * 68.0;
    }
    h.clamp(240.0, 640.0)
}

fn configuration_height(snapshot: &Snapshot) -> f32 {
    let recognized = if snapshot.recognized.is_empty() {
        0.0
    } else {
        72.0 + snapshot.recognized.len() as f32 * 40.0
    };
    (280.0 + recognized).min(640.0)
}

enum Screen {
    List,
    Configuration,
    Logs { name: String, text: String },
}

pub struct Popover {
    resources: gpui::Entity<SystemMetrics>,
    snapshot: Snapshot,
    screen: Screen,
    focus: FocusHandle,
    log_scroll: ScrollHandle,
    seen_active: bool,
    answering: bool,
    confirm_stop: Option<ProcessInfo>,
    stop_details_open: bool,
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
                    Screen::List | Screen::Configuration => None,
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
            confirm_stop: None,
            stop_details_open: false,
        }
    }

    fn apply(
        &mut self,
        snapshot: Snapshot,
        logs: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let old_height = height_for(&self.snapshot);
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
            (Screen::List, _) if self.confirm_stop.is_none() => {
                let height = height_for(&self.snapshot);
                if (height - old_height).abs() > 1.0 {
                    window.resize(size(px(WIDTH), px(height)));
                }
            }
            (Screen::Configuration, _) => {
                window.resize(size(px(WIDTH), px(configuration_height(&self.snapshot))));
            }
            _ => {}
        }
        cx.notify();
    }

    /// Send a request in the background, then refresh straight away.
    fn send(&mut self, req: Request, window: &mut Window, cx: &mut Context<Self>) {
        let clearing = matches!(&req, Request::Clear { .. });
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { request(&req) })
                .await;
            let cleared = clearing && matches!(&result, Ok(Response::Ok));
            let mut snapshot = cx.background_executor().spawn(async { fetch() }).await;
            match result {
                Ok(Response::Error { message }) => snapshot.error = Some(message),
                Err(error) => snapshot.error = Some(format!("{error:#}")),
                _ => {}
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.apply(snapshot, None, window, cx);
                if cleared {
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

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = Screen::List;
        window.resize(size(px(WIDTH), px(height_for(&self.snapshot))));
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            if self.confirm_stop.take().is_some() {
                window.resize(size(px(WIDTH), px(height_for(&self.snapshot))));
                cx.notify();
                return;
            }
            match self.screen {
                Screen::Logs { .. } | Screen::Configuration => self.back(window, cx),
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

    fn stale_section(
        &self,
        list: &ProcessList<'_>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let decisions = list
            .stale
            .iter()
            .map(|(_, d)| (*d).clone())
            .collect::<Vec<_>>();
        let summary = list
            .stale
            .iter()
            .any(|(p, _)| p.metrics.is_some())
            .then(|| format!("~{} to free", bytes(list.stale_ram())));
        let stop = div()
            .id("stop-stale")
            .px_2()
            .py_1()
            .rounded_md()
            .text_size(px(11.0))
            .text_color(theme.danger)
            .child("Stop all")
            .when(self.answering, |button| button.opacity(0.5))
            .when(!self.answering, |button| {
                button
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.stop_stale(decisions.clone(), window, cx);
                    }))
            });
        div()
            .mt_2()
            .mb_2()
            .child(
                div()
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .font_family(MONO)
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.faint)
                            .child(format!("STALE · {}", list.stale.len())),
                    )
                    .when_some(summary, |header, summary| {
                        header.child(
                            div()
                                .font_family(MONO)
                                .text_size(px(10.0))
                                .text_color(theme.faint)
                                .child(summary),
                        )
                    })
                    .child(stop),
            )
            .children(list.stale.iter().map(|(process, decision)| {
                let name = process.name.clone();
                let group = process.group();
                let workspace = process.repo.as_deref().unwrap_or(&process.cwd);
                let workspace = std::path::Path::new(workspace)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| workspace.to_string());
                let quiet = decision.quiet_since_ms.map_or_else(
                    || "quiet".into(),
                    |since| format!("quiet {}", duration(now_ms().saturating_sub(since))),
                );
                let port = process
                    .ports
                    .first()
                    .map(|p| format!(" · :{p}"))
                    .unwrap_or_default();
                let memory = process
                    .metrics
                    .map(|m| format!("~{}", bytes(m.rss_bytes)))
                    .unwrap_or_default();
                div()
                    .id(SharedString::from(format!("stale-{}", process.id.0)))
                    .ml_6()
                    .mr_3()
                    .px_2()
                    .py_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_3()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.open_logs(name.clone(), window, cx);
                    }))
                    .child(dot(theme.stale))
                    .child(crate::icons::runtime(&process.kind))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.muted)
                                    .child(process.name.clone()),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "stale-context-{}",
                                        process.id.0
                                    )))
                                    .truncate()
                                    .text_size(px(11.0))
                                    .text_color(theme.faint)
                                    .child(format!("{workspace}{port}"))
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| AgentGroupDetails(group.clone())).into()
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .items_end()
                            .font_family(MONO)
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(theme.muted)
                                    .child(memory),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(theme.faint)
                                    .child(quiet),
                            ),
                    )
            }))
    }

    fn process_row(
        &self,
        index: usize,
        p: &ProcessInfo,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let managed = p.origin == Origin::Managed;
        let alive = p.status.is_alive();
        let color = match &p.status {
            Status::Running => theme.running,
            Status::Stale => theme.stale,
            Status::Exited { .. } => theme.stopped,
        };

        let mut subtitle = vec![p.kind.label().to_string()];
        if let Some(app) = p
            .launcher_app
            .as_deref()
            .filter(|app| Some(*app) != p.app_bundle.as_deref())
            && let Some(name) = std::path::Path::new(app).file_stem()
        {
            subtitle.push(format!("via {}", name.to_string_lossy()));
        }
        if !managed && let Some(repo) = p.repo.as_deref().and_then(|r| r.rsplit('/').next()) {
            subtitle.push(match &p.branch {
                Some(b) => format!("{repo}@{b}"),
                None => repo.to_string(),
            });
        }
        if let Some(owner) = &p.owner {
            subtitle.push(owner.clone());
        }

        if let Status::Exited { code: Some(code) } = &p.status {
            subtitle.push(format!("exit {code}"));
        }

        if !managed {
            subtitle.push(
                p.executable
                    .clone()
                    .unwrap_or_else(|| "Executable unknown".into()),
            );
            subtitle.push(format!("PID {}", p.pid.unwrap_or_default()));
        }

        let right_top = match (p.ports.first(), &p.status) {
            (Some(port), _) => format!(":{port}"),
            (None, Status::Exited { code: Some(c) }) => format!("exit {c}"),
            (None, Status::Exited { code: None }) => "stopped".into(),
            _ if p.metrics.is_none() => "starting".into(),
            _ => String::new(),
        };
        let right_bottom = match (alive, p.metrics) {
            (true, Some(m)) => format!(
                "CPU {:.0}% · {} RAM · up {}",
                m.cpu_percent,
                bytes(m.rss_bytes),
                duration(now_ms().saturating_sub(p.started_at_ms))
            ),
            (true, None) => format!("up {}", duration(now_ms().saturating_sub(p.started_at_ms))),
            _ => String::new(),
        };

        let name = p.name.clone();
        let target = p.id.0.clone();
        let mut row = div()
            .id(("row", index))
            .group("row")
            .mx_2()
            .px_2()
            .py_2()
            .rounded_md()
            .flex()
            .items_center()
            .gap_3()
            .hover(|s| s.bg(theme.hover))
            .child(dot(color))
            .child(crate::icons::runtime(&p.kind))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if alive { theme.text } else { theme.muted })
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.0))
                            .text_color(theme.muted)
                            .child(subtitle.join(" · ")),
                    )
                    .when(alive, |column| {
                        column.child(
                            div()
                                .font_family(MONO)
                                .text_size(px(10.0))
                                .text_color(theme.muted)
                                .child(right_bottom),
                        )
                    }),
            )
            .when(alive, |row| {
                row.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_end()
                        .font_family(MONO)
                        .child(div().text_size(px(12.0)).child(right_top)),
                )
            });

        {
            let recognize = p.recognizable_app().map(|app| {
                let process = p.clone();
                icon_button(
                    ("recognize", index),
                    "icons/eye-off.svg",
                    format!("Recognize {}: stop listing its processes", app_name(app)),
                    theme.text,
                    theme,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.send(
                        Request::Recognize {
                            process: Box::new(process.clone()),
                        },
                        window,
                        cx,
                    );
                }))
            });
            let process = p.clone();
            let stop = icon_button(
                (if alive { "stop" } else { "clear" }, index),
                "icons/x.svg",
                match (alive, managed) {
                    (false, _) => "Clear",
                    (true, true) => "Stop",
                    (true, false) => "Stop…",
                },
                theme.danger,
                theme,
            );
            let mut actions = div().flex().flex_none().gap_1().children(recognize);
            if alive {
                actions = actions.invisible().group_hover("row", |s| s.visible());
            }
            row = row.child(actions.child(stop.on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    if !managed {
                        this.confirm_stop = Some(process.clone());
                        this.stop_details_open = false;
                        window.resize(size(px(WIDTH), px(440.0)));
                        cx.notify();
                        return;
                    }
                    this.send(
                        if alive {
                            Request::Stop {
                                name: target.clone(),
                            }
                        } else {
                            Request::Clear {
                                name: target.clone(),
                            }
                        },
                        window,
                        cx,
                    );
                },
            ))));
        }
        if managed {
            row = row.cursor_pointer().on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    this.open_logs(name.clone(), window, cx);
                },
            ));
        }
        row.into_any_element()
    }

    fn list(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let list = ProcessList::new(&self.snapshot.processes, &self.snapshot.decisions);
        let managed = &list.managed;
        let unmanaged = &list.unmanaged;

        if let Some(process) = &self.confirm_stop {
            let confirmed = process.clone();
            let mut card = div().flex().flex_col().flex_1().min_h_0().px_4().pb_3().gap_2()
                .child(div().flex_shrink_0().text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).child("Stop this process?"))
                .child(div().flex_shrink_0().truncate().text_size(px(12.0)).child(process.name.clone()))
                .child(div().flex_shrink_0().text_size(px(12.0)).child("Started outside Pinch. Stopping it may interrupt another app or lose unsaved work. Only this PID receives SIGTERM; it may stay running or restart."))
                .child(div().flex_shrink_0().text_size(px(11.0)).child(format!("PID {}", process.pid.unwrap_or_default())))
                .child(div().id("stop-details-toggle").flex_shrink_0().cursor_pointer().text_size(px(11.0))
                    .child(if self.stop_details_open { "▾ Process details" } else { "▸ Process details" })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.stop_details_open = !this.stop_details_open;
                        cx.notify();
                    })));
            if self.stop_details_open {
                let mut details = div()
                    .id("stop-details")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_2();
                for detail in [
                    format!("Name: {}", process.name),
                    format!(
                        "App (detected): {}",
                        process.app_bundle.as_deref().unwrap_or("Unknown")
                    ),
                    format!(
                        "Launched via (detected): {}",
                        process.launcher_app.as_deref().unwrap_or("Unknown")
                    ),
                    format!(
                        "Executable: {}",
                        process.executable.as_deref().unwrap_or("Unknown")
                    ),
                    format!("Command: {}", process.command.join(" ")),
                    format!("Directory: {}", process.cwd),
                ] {
                    details =
                        details.child(div().flex_shrink_0().text_size(px(11.0)).child(detail));
                }
                card = card.child(details);
            } else {
                card = card.child(div().flex_1());
            }
            return card
                .child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .gap_2()
                        .child(
                            button("cancel-stop", "Cancel", false, theme).on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| {
                                    this.confirm_stop = None;
                                    window.resize(size(px(WIDTH), px(height_for(&this.snapshot))));
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            button("confirm-stop", "Stop this process", true, theme).on_click(
                                cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    this.confirm_stop = None;
                                    window.resize(size(px(WIDTH), px(height_for(&this.snapshot))));
                                    this.send(
                                        Request::StopDiscovered {
                                            process: Box::new(confirmed.clone()),
                                        },
                                        window,
                                        cx,
                                    );
                                }),
                            ),
                        ),
                )
                .into_any_element();
        }
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
        if !list.stale.is_empty() {
            body = body.child(self.stale_section(&list, theme, cx));
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
        if managed.is_empty()
            && unmanaged.is_empty()
            && list.stale.is_empty()
            && self.snapshot.error.is_none()
            && !self.snapshot.loading
        {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .py_8()
                    .child(
                        div().text_size(px(28.0)).child(
                            gpui::svg()
                                .path("brand/daemon.svg")
                                .w(px(48.))
                                .h(px(48.))
                                .text_color(theme.muted),
                        ),
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
        if !managed.is_empty() {
            body = body.child(section_label("MANAGED", theme));
            let mut groups = std::collections::BTreeMap::<_, Vec<_>>::new();
            for &(i, p) in managed {
                groups.entry(p.group()).or_default().push((i, p));
            }
            for (key, entries) in groups {
                body = body.child(agent_group_label(&key, theme));
                for (i, p) in entries {
                    body = body.child(self.process_row(i, p, theme, cx));
                }
            }
        }
        if !unmanaged.is_empty() {
            body = body.child(section_label("UNRECOGNIZED", theme));
            for &(i, p) in unmanaged {
                body = body.child(self.process_row(i, p, theme, cx));
            }
        }
        body.into_any_element()
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
                button("log-stop", "Stop", true, theme).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.send(
                            Request::Stop {
                                name: stop_name.clone(),
                            },
                            window,
                            cx,
                        );
                    },
                )),
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
                .child(div().text_size(px(12.0)).text_color(theme.muted)
                    .child(if updates.enabled {
                        "Check for new versions automatically. Installing an update always asks for approval."
                    } else {
                        "Updates are unavailable in this build."
                    }))
                .when(updates.enabled, |body| body.child(
                    div().flex().child(button("configuration-check-updates", "Check for Updates…", false, theme)
                        .when(updates.can_check || updates.available_version.is_some(), |button| button.on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| this.check_for_updates(window, cx))))
                        .when(!updates.can_check && updates.available_version.is_none(), |button| button.opacity(0.4)))))
                .when_some(self.snapshot.error.clone(), |body, error| body.child(
                    div().text_size(px(12.0)).text_color(theme.danger).child(error))))
            .when(!self.snapshot.recognized.is_empty(), |view| {
                view.child(self.recognized_apps(theme, cx))
            })
            .into_any_element()
    }

    fn recognized_apps(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("recognized-apps")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pb_3()
            .child(section_label("RECOGNIZED APPS", theme))
            .child(
                div()
                    .px_4()
                    .pb_1()
                    .text_size(px(12.0))
                    .text_color(theme.muted)
                    .child("Their processes are expected, so Pinch doesn't list them."),
            )
            .children(
                self.snapshot
                    .recognized
                    .iter()
                    .enumerate()
                    .map(|(index, app)| {
                        let path = app.path.clone();
                        div()
                            .mx_2()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .flex()
                            .items_center()
                            .gap_3()
                            .hover(|s| s.bg(theme.hover))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(13.0))
                                            .child(app.name().to_string()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(11.0))
                                            .text_color(theme.faint)
                                            .child(app.path.clone()),
                                    ),
                            )
                            .child(
                                icon_button(
                                    ("forget", index),
                                    "icons/eye.svg",
                                    format!("List {} processes again", app.name()),
                                    theme.text,
                                    theme,
                                )
                                .on_click(cx.listener(
                                    move |this, _: &ClickEvent, window, cx| {
                                        this.send(
                                            Request::Forget { app: path.clone() },
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                            )
                    }),
            )
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
                this.confirm_stop = None;
                this.screen = Screen::Configuration;
                window.resize(size(px(WIDTH), px(configuration_height(&this.snapshot))));
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
            .child(resources::indicators(&self.resources, theme, cx))
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

fn section_label(text: impl Into<SharedString>, theme: Theme) -> impl IntoElement {
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
    id: impl Into<SharedString>,
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
