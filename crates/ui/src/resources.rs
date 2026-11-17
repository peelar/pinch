//! Compact views of the engine's cached whole-computer metrics.

use gpui::{
    App, ClickEvent, Context, Entity, FontWeight, IntoElement, Render, Rgba, Window, div,
    prelude::*, px,
};
use pinch_proto::{AppUsage, Close, MemoryPressure, SystemMetrics};

use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Measure {
    Cpu,
    Memory,
}

/// The footer meters; clicking one opens what is using that resource.
pub fn indicators(
    source: &Entity<SystemMetrics>,
    theme: Theme,
    on_open: impl Fn(Measure, &mut Window, &mut App) + Clone + 'static,
    cx: &App,
) -> impl IntoElement {
    let usage = *source.read(cx);
    let open_memory = on_open.clone();
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            meter("CPU", usage.cpu_percent, None, source.clone(), theme)
                .on_click(move |_: &ClickEvent, window, cx| on_open(Measure::Cpu, window, cx)),
        )
        .child(
            meter(
                "RAM",
                memory_percent(&usage),
                usage.memory_pressure,
                source.clone(),
                theme,
            )
            .on_click(move |_: &ClickEvent, window, cx| open_memory(Measure::Memory, window, cx)),
        )
}

pub fn memory_percent(usage: &SystemMetrics) -> Option<f32> {
    (usage.memory_total_bytes > 0)
        .then(|| usage.memory_used_bytes as f32 / usage.memory_total_bytes as f32 * 100.0)
}

/// Meter color. macOS fills memory with caches on purpose, so when the kernel
/// reports pressure, that decides the RAM color rather than "used".
pub fn level_color(percent: Option<f32>, pressure: Option<MemoryPressure>, theme: Theme) -> Rgba {
    match (pressure, percent) {
        (Some(MemoryPressure::Critical), _) => theme.danger,
        (Some(MemoryPressure::Warning), _) => theme.stale,
        (Some(MemoryPressure::Normal), _) => theme.running,
        (None, Some(p)) if p >= 85.0 => theme.danger,
        (None, Some(p)) if p >= 60.0 => theme.stale,
        _ => theme.running,
    }
}

pub fn pressure_label(pressure: Option<MemoryPressure>) -> Option<&'static str> {
    pressure.map(|p| match p {
        MemoryPressure::Normal => "Memory pressure normal",
        MemoryPressure::Warning => "Memory pressure high",
        MemoryPressure::Critical => "Memory pressure critical",
    })
}

/// Ask an app to quit the normal way, as the Dock's Quit does, so it can save
/// work or refuse. Checks that the PID still belongs to the same app.
pub fn quit(app: &AppUsage) -> Result<(), String> {
    use objc2_app_kit::NSRunningApplication;
    let (Some(Close::Quit { pid }), Some(bundle)) = (app.close, app.app_bundle.as_deref()) else {
        return Err(format!("{} can't be quit from Pinch", app.name));
    };
    let running = i32::try_from(pid)
        .ok()
        .and_then(NSRunningApplication::runningApplicationWithProcessIdentifier)
        .filter(|running| {
            running
                .bundleURL()
                .and_then(|url| url.path())
                .is_some_and(|path| path.to_string() == bundle)
        })
        .ok_or_else(|| format!("{} is no longer running", app.name))?;
    if running.terminate() {
        Ok(())
    } else {
        Err(format!("{} did not accept the request to quit", app.name))
    }
}

fn meter(
    label: &'static str,
    percent: Option<f32>,
    pressure: Option<MemoryPressure>,
    source: Entity<SystemMetrics>,
    theme: Theme,
) -> gpui::Stateful<gpui::Div> {
    let fill = level_color(percent, pressure, theme);
    let filled = percent.map(|p| (p.clamp(0.0, 100.0) / 100.0 * 8.0).ceil() as usize);
    div()
        .id(label)
        .flex()
        .items_center()
        .gap_1()
        .px_1()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|s| s.bg(theme.hover))
        .text_size(px(9.0))
        .text_color(theme.muted)
        .child(label)
        .child(div().flex().gap(px(2.0)).children((0..8).map(|index| {
            div()
                .w(px(3.0))
                .h(px(9.0))
                .rounded(px(1.0))
                .bg(if filled.is_some_and(|n| index < n) {
                    fill
                } else {
                    theme.border
                })
        })))
        .tooltip(move |_, cx| {
            cx.new(|cx| {
                cx.observe(&source, |_, _, cx| cx.notify()).detach();
                Details {
                    label,
                    source: source.clone(),
                }
            })
            .into()
        })
}

struct Details {
    label: &'static str,
    source: Entity<SystemMetrics>,
}

impl Render for Details {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_appearance(window.appearance());
        let usage = *self.source.read(cx);
        let lines = if self.label == "CPU" {
            vec![usage.cpu_percent.map_or_else(
                || "Measuring…".into(),
                |p| format!("{p:.0}% of total CPU capacity in use"),
            )]
        } else if usage.memory_total_bytes == 0 {
            vec!["Measuring…".into()]
        } else {
            vec![
                format!(
                    "{:.0}% used · {} of {}",
                    usage.memory_used_bytes as f64 / usage.memory_total_bytes as f64 * 100.0,
                    bytes(usage.memory_used_bytes),
                    bytes(usage.memory_total_bytes)
                ),
                format!(
                    "{} available · {} swap used",
                    bytes(usage.memory_available_bytes),
                    bytes(usage.swap_used_bytes)
                ),
            ]
            .into_iter()
            .chain(pressure_label(usage.memory_pressure).map(str::to_owned))
            .collect()
        };
        div()
            .p_3()
            .rounded_lg()
            .bg(theme.tooltip)
            .border_1()
            .border_color(theme.border)
            .shadow_md()
            .font_family(".SystemUIFont")
            .text_size(px(11.0))
            .text_color(theme.text)
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("System {}", self.label)),
            )
            .children(lines.into_iter().map(|line| div().child(line)))
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(theme.muted)
                    .child("Whole computer · click to see what's using it"),
            )
    }
}

pub fn bytes(n: u64) -> String {
    if n >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", n as f64 / (1024.0 * 1024.0 * 1024.0))
    } else {
        format!("{:.0} MiB", n as f64 / (1024.0 * 1024.0))
    }
}
