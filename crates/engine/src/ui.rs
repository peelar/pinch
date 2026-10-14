//! Opening and closing `pinch-ui` when the menu bar icon is clicked.

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pinch_macos::Anchor;

#[derive(Default)]
struct State {
    pid: Option<u32>,
    closed_at: Option<Instant>,
}

#[derive(Default)]
pub struct Launcher {
    state: Arc<Mutex<State>>,
}

impl Launcher {
    /// Click on an open popover closes it; click on a closed one opens it.
    pub fn toggle(&self, anchor: Option<Anchor>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(pid) = state.pid {
            // SAFETY: plain signal delivery to the UI process we spawned.
            unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            return;
        }
        // The popover closes itself when it loses focus, which happens just
        // before the click that would reopen it lands. Treat that click as
        // "close", not "open again".
        if state
            .closed_at
            .is_some_and(|t| t.elapsed() < Duration::from_millis(400))
        {
            return;
        }

        let Ok(exe) = pinch_client::sibling_exe("pinch-ui") else {
            eprintln!("pinchd: pinch-ui not found next to pinchd");
            return;
        };
        let mut cmd = Command::new(exe);
        if let Some(a) = anchor {
            cmd.arg("--anchor")
                .arg(format!("{},{}", a.x_center, a.y_bottom));
        }
        match cmd.spawn() {
            Ok(mut child) => {
                state.pid = Some(child.id());
                let shared = Arc::clone(&self.state);
                std::thread::spawn(move || {
                    let _ = child.wait();
                    let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
                    state.pid = None;
                    state.closed_at = Some(Instant::now());
                });
            }
            Err(e) => eprintln!("pinchd: could not launch pinch-ui: {e}"),
        }
    }
}
