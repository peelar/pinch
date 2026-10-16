//! Engine executable: selects headless operation or the macOS desktop shell.

#[cfg(all(target_os = "macos", feature = "desktop"))]
mod desktop;

use anyhow::Result;
use pinch_engine::{EngineRuntime, Hooks};
use pinch_proto::NotificationStatus;

fn main() -> Result<()> {
    std::fs::create_dir_all(pinch_client::home())?;
    if pinch_client::request(&pinch_proto::Request::Hello {
        version: pinch_proto::PROTOCOL_VERSION,
    })
    .is_ok()
    {
        eprintln!("pinchd is already running");
        return Ok(());
    }

    #[cfg(all(target_os = "macos", feature = "desktop"))]
    if !std::env::args().any(|a| a == "--headless") {
        let runtime = EngineRuntime::start(desktop::hooks(), desktop::notification_status)?;
        return desktop::run(runtime);
    }

    let runtime = EngineRuntime::start(Hooks::default(), || NotificationStatus::Unavailable)?;
    runtime.start_sampler();
    std::thread::park();
    Ok(())
}
