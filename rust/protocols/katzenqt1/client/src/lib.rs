//! A katzenqt1 group chat client: the engine, its state on disk, and its work carried out over a
//! transport.

mod client;
pub mod exec;
pub mod store;

pub use client::{ChatView, Client, ClientError, Config, Event, LineView, MemberView};

pub use exec::{Executor, Files, Outcome, RETRY_MS, failure};
