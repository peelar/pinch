//! Keep the UI runtime ready and forward menu bar clicks over its stdin.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use pinch_macos::Anchor;

#[derive(Default)]
pub struct Launcher {
    child: Mutex<Option<Child>>,
}

impl Launcher {
    pub fn stop(&self) {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut process) = child.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
    }

    pub fn prepare(&self) {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        Self::ensure_running(&mut child);
    }

    fn ensure_running(child: &mut Option<Child>) {
        if let Some(process) = child.as_mut() {
            match process.try_wait() {
                Ok(None) => return,
                Ok(Some(_)) => {}
                Err(e) => {
                    eprintln!("pinchd: could not inspect pinch-ui: {e}");
                    return;
                }
            }
        }
        *child = None;
        let Ok(exe) = pinch_client::sibling_exe("pinch-ui") else {
            eprintln!("pinchd: pinch-ui not found next to pinchd");
            return;
        };
        match Command::new(exe)
            .arg("--resident")
            .stdin(Stdio::piped())
            .spawn()
        {
            Ok(process) => *child = Some(process),
            Err(e) => eprintln!("pinchd: could not launch pinch-ui: {e}"),
        }
    }

    pub fn toggle(&self, anchor: Option<Anchor>) {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        Self::ensure_running(&mut child);
        if let Some(input) = child.as_mut().and_then(|p| p.stdin.as_mut()) {
            let command = anchor.map_or_else(
                || "toggle".into(),
                |a| format!("{},{}", a.x_center, a.y_bottom),
            );
            if let Err(e) = writeln!(input, "{command}") {
                eprintln!("pinchd: could not toggle pinch-ui: {e}");
            }
        }
    }
}
