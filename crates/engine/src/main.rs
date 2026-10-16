//! `pinchd`: the always-on engine. Supervises managed processes, samples
//! metrics, runs the staleness rule, holds pending decisions, and serves
//! clients over a Unix socket. On macOS it also owns the menu bar item and
//! notifications. See DEVELOPMENT.md.
//!
//! `pinchd --headless` skips the menu bar layer (for tests and servers).

mod engine;
mod lsof;
mod server;
mod ui;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use engine::{Config, Engine, Hooks};
use pinch_macos::{Action, Callbacks, Handle};
use pinch_proto::DecisionAnswer;

/// Set once the menu bar layer is up; the engine's hooks reach it through here.
static MENU_BAR: OnceLock<Handle> = OnceLock::new();

fn main() -> Result<()> {
    let headless = std::env::args().any(|a| a == "--headless");

    std::fs::create_dir_all(pinch_client::home())?;
    if pinch_client::request(&pinch_proto::Request::Hello {
        version: pinch_proto::PROTOCOL_VERSION,
    })
    .is_ok()
    {
        eprintln!("pinchd is already running");
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let listener = {
        let _guard = runtime.enter();
        server::bind()?
    };

    let engine = Engine::new(
        Config::from_env(),
        Hooks {
            decisions_changed: Box::new(|decisions| {
                if let Some(menu_bar) = MENU_BAR.get() {
                    menu_bar.sync_decisions(decisions, false);
                }
            }),
        },
    )?;

    runtime.spawn(server::serve(listener, Arc::clone(&engine)));
    eprintln!("pinchd {} listening", env!("CARGO_PKG_VERSION"));

    if headless {
        start_sampler(Arc::clone(&engine));
        std::thread::park();
        return Ok(());
    }

    let ui = Arc::new(ui::Launcher::default());
    let first_launch_ui = Arc::clone(&ui);
    let action_engine = Arc::clone(&engine);
    let callbacks = Callbacks {
        on_click: Box::new(move |anchor| ui.toggle(anchor)),
        on_action: Box::new(move |decision, action| {
            let answer = match action {
                Action::Kill => DecisionAnswer::Kill,
                Action::Keep => DecisionAnswer::Keep,
                Action::Snooze => DecisionAnswer::Snooze,
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
            start_sampler(engine);
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

fn start_sampler(engine: Arc<Engine>) {
    engine.tick();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(2));
            engine.tick();
        }
    });
}
