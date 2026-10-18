//! A katzenqt1 group chat client: the engine, its state on disk, and its work carried out over a
//! transport.

pub mod exec;
pub mod store;

pub use exec::{Executor, Files, Outcome, RETRY_MS};
