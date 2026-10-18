//! A katzenqt1 group chat client: the engine, its state on disk, and its work carried out over a
//! transport.

pub mod exec;

pub use exec::{Executor, Files, Outcome, RETRY_MS};
