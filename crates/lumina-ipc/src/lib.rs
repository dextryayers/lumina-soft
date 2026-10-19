//! Local IPC for Lumina.
//!
//! Lumina exposes one small control surface to the rest of the user's desktop:
//! a newline-delimited JSON protocol over a per-user Unix domain socket or
//! Windows named pipe. Shell scripts, editor plugins, stream decks, and test
//! harnesses all use the same transport.
//!
//! # Layers
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`protocol`] | Commands, responses, error codes, and state snapshots |
//! | [`framing`] | Newline framing with a hard 64 KiB line cap |
//! | [`rate_limit`] | Per-client token bucket (120 commands per second) |
//! | [`transport`] | Per-user socket discovery and platform listeners |
//! | [`server`] | Accept loop, serialized dispatch, and multi-client handling |
//! | [`client`] | Blocking client used by `lumina-send` and integration tests |
//! | [`events`] | Unsolicited event push to every connected client |
//!
//! # Guarantees
//!
//! * Commands are executed on a single dispatcher thread, so the order in which
//!   the orchestrator observes them is deterministic across clients.
//! * A malformed command is answered with an error and never closes the
//!   connection. Only a framing violation (a line longer than 64 KiB) does.
//! * At most [`protocol::MAX_CLIENTS`] connections are served at once, and each
//!   connection gets its own request id space and rate limiter.
//! * The socket is created with mode `0600` on Unix. The Windows pipe carries a
//!   DACL that grants the creating user full control and nobody else access.

#![deny(missing_docs)]

pub mod client;
pub mod events;
pub mod framing;
pub mod protocol;
pub mod rate_limit;
pub mod server;
pub mod transport;

pub use client::{Client, ClientError};
pub use events::{Event, EventKind, EventSender, Subscription};
pub use framing::{encode_line, FrameError, LineReader, MAX_LINE_BYTES};
pub use protocol::{
    Command, ExecuteArgs, GeometryArgs, GeometrySnapshot, GetConfigArgs, IpcError, IpcErrorCode,
    MediaAction, MediaArgs, MediaSnapshot, NotifyArgs, Request, RequestId, Response, RgbSnapshot,
    SetConfigArgs, SetProfileArgs, SetRgbArgs, ShowArgs, SpectrumArgs, StateSnapshot, MAX_CLIENTS,
};
pub use rate_limit::{RateLimiter, DEFAULT_RATE_PER_SECOND};
pub use server::{Handler, Server, ServerHandle, ServerOptions};
pub use transport::{Connection, IpcAddress, Listener};

/// Crate version string, injected at build time from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
