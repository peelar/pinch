//! Socket observations consumed by the engine. OS commands and parsing stay
//! in the backend; callers only see listener ownership and connected ports.

mod lsof;

pub use lsof::{established_local_ports, listeners};
