//! Compact views of the engine's cached whole-computer metrics.

use gpui::{App, Context, Entity, FontWeight, IntoElement, Render, Window, div, prelude::*, px};
use pinch_proto::SystemMetrics;

use crate::theme::Theme;

pub fn indicators(source: &Entity<SystemMetrics>, theme: Theme, cx: &App) -> impl IntoElement {
    let usage = *source.read(cx);
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(meter("CPU", usage.cpu_percent, source.clone(), theme))
        .child(meter(
            "RAM",
            (usage.memory_total_bytes > 0)
                .then(|| usage.memory_used_bytes as f32 / usage.memory_total_bytes as f32 * 100.0),
            source.clone(),
            theme,
        ))
}

fn meter(
    label: &'static str,
    percent: Option<f32>,
    source: Entity<SystemMetrics>,
    theme: Theme,
) -> impl IntoElement {
    let fill = match percent {
        Some(p) if p >= 85.0 => theme.danger,
        Some(p) if p >= 60.0 => theme.stale,
        _ => theme.running,
    };
    let filled = percent.map(|p| (p.clamp(0.0, 100.0) / 100.0 * 8.0).ceil() as usize);
    div()
        .id(label)
        .flex()
        .items_center()
        .gap_1()
        .px_1()
        .py_1()
        .rounded_md()
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
                    .child("Whole computer · sampled every 2 seconds"),
            )
    }
}

fn bytes(n: u64) -> String {
    if n >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", n as f64 / (1024.0 * 1024.0 * 1024.0))
    } else {
        format!("{:.0} MiB", n as f64 / (1024.0 * 1024.0))
    }
}
