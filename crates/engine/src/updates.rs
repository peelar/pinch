//! Update capability supplied by the desktop host; headless engines stay inert.

use anyhow::{Result, bail};
use pinch_proto::UpdateStatus;

pub struct UpdateService {
    pub status: Box<dyn Fn() -> UpdateStatus + Send + Sync>,
    pub check: Box<dyn Fn() -> Result<()> + Send + Sync>,
    pub set_automatic_checks: Box<dyn Fn(bool) -> Result<()> + Send + Sync>,
}

impl Default for UpdateService {
    fn default() -> Self {
        Self {
            status: Box::new(UpdateStatus::default),
            check: Box::new(|| bail!("Updates are unavailable in this build")),
            set_automatic_checks: Box::new(|_| bail!("Updates are unavailable in this build")),
        }
    }
}
