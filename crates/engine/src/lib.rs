//! Process runtime shared by headless operation and desktop composition.

mod discovery;
mod engine;
mod server;
mod updates;

pub use updates::UpdateService;

pub use engine::{Config, Engine, Hooks};

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use pinch_proto::NotificationStatus;

/// Owns the RPC executor for as long as the engine is running.
pub struct EngineRuntime {
    engine: Arc<Engine>,
    _executor: tokio::runtime::Runtime,
}

impl EngineRuntime {
    pub fn start(
        hooks: Hooks,
        notification_status: impl Fn() -> NotificationStatus + Send + Sync + 'static,
        updates: UpdateService,
    ) -> Result<Self> {
        let executor = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let listener = {
            let _guard = executor.enter();
            server::bind()?
        };
        let engine = Engine::new(Config::from_env(), hooks)?;
        executor.spawn(server::serve(
            listener,
            Arc::clone(&engine),
            Arc::new(notification_status),
            Arc::new(updates),
        ));
        eprintln!("pinchd {} listening", env!("CARGO_PKG_VERSION"));
        Ok(Self {
            engine,
            _executor: executor,
        })
    }

    pub fn engine(&self) -> Arc<Engine> {
        Arc::clone(&self.engine)
    }

    /// Start only after the shell has synchronized restored decisions.
    pub fn start_sampler(&self) {
        let engine = Arc::clone(&self.engine);
        engine.tick();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(2));
                engine.tick();
            }
        });
    }
}
