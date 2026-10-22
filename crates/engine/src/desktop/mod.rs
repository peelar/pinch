//! macOS desktop composition. Native types never enter the engine library.

mod ui;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use pinch_engine::{EngineRuntime, Hooks, UpdateService};
use pinch_macos::{Action, Callbacks, Handle};
use pinch_proto::DecisionAnswer;

pub use pinch_macos::notification_status;

static MENU_BAR: OnceLock<Handle> = OnceLock::new();

pub fn updates() -> UpdateService {
    UpdateService {
        status: Box::new(pinch_macos::updater::status),
        check: Box::new(|| pinch_macos::updater::check().map_err(anyhow::Error::msg)),
        set_automatic_checks: Box::new(|enabled| {
            pinch_macos::updater::set_automatic(enabled).map_err(anyhow::Error::msg)
        }),
    }
}

pub fn hooks() -> Hooks {
    Hooks {
        decisions_changed: Box::new(|decisions| {
            if let Some(menu_bar) = MENU_BAR.get() {
                menu_bar.sync_decisions(decisions, false);
            }
        }),
    }
}

pub fn run(runtime: EngineRuntime) -> Result<()> {
    let runtime = Arc::new(runtime);
    let ready_runtime = Arc::clone(&runtime);
    let engine = runtime.engine();
    let ui = Arc::new(ui::Launcher::default());
    let first_launch_ui = Arc::clone(&ui);
    let update_ui = Arc::clone(&ui);
    let action_engine = Arc::clone(&engine);
    let callbacks = Callbacks {
        on_update_install: Box::new(move || update_ui.stop()),
        on_click: Box::new(move |anchor| ui.toggle(anchor)),
        on_action: Box::new(move |decision, action| {
            let answer = match action {
                Action::Kill => DecisionAnswer::Kill,
                Action::Keep => DecisionAnswer::Keep,
            };
            let engine = Arc::clone(&action_engine);
            std::thread::spawn(move || {
                engine.tick();
                if let Err(e) = engine.answer(&decision, answer) {
                    eprintln!("pinchd: could not apply {action:?} to {decision}: {e:#}");
                }
            });
        }),
    };
    // The menu bar must own the main thread; everything else runs elsewhere.
    pinch_macos::run(
        callbacks,
        pinch_client::installation::variant().map(str::to_owned),
        move |handle| {
            handle.sync_decisions(&engine.snapshot().1, true);
            let _ = MENU_BAR.set(handle);
            ready_runtime.start_sampler();
            first_launch_ui.prepare();
            // First launch from the desktop app: show the popover once so the
            // onboarding card is seen without a terminal.
            if pinch_client::onboarding::needed() && !pinch_client::onboarding::detect().is_empty()
            {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(1));
                    first_launch_ui.toggle(None);
                });
            }
        },
    )
}
