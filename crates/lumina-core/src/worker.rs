//! Panic containment for background workers.
//!
//! The audio, media, and IPC workers each own a subsystem that can fail without
//! taking the island down. [`run_guarded`] wraps a worker body in
//! [`std::panic::catch_unwind`], and [`spawn_guarded`] does the same on a named
//! thread. The orchestrator treats a returned [`WorkerPanic`] as "that subsystem
//! is unavailable": audio stops, media polling stops, and the rest of the UI
//! keeps rendering.
//!
//! This only works when the binary unwinds. The release profile therefore sets
//! `panic = "unwind"`; `panic = "abort"` would terminate the process before the
//! guard could report anything.

use std::any::Any;
use std::fmt;
use std::io;
use std::panic::{catch_unwind, UnwindSafe};
use std::thread::{self, JoinHandle};

/// A worker panicked, and the panic was contained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerPanic {
    worker: String,
    message: String,
}

impl WorkerPanic {
    /// Name of the worker that failed.
    #[must_use]
    pub fn worker(&self) -> &str {
        &self.worker
    }

    /// Best-effort panic message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for WorkerPanic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "worker '{}' panicked: {}",
            self.worker, self.message
        )
    }
}

impl std::error::Error for WorkerPanic {}

/// Run `body`, converting a panic into a [`WorkerPanic`].
///
/// The panic hook still runs, so the default backtrace output is preserved.
/// Callers should log the returned value at error level and degrade the
/// corresponding subsystem.
pub fn run_guarded<F>(worker: &str, body: F) -> Result<(), WorkerPanic>
where
    F: FnOnce() + UnwindSafe,
{
    catch_unwind(body).map_err(|payload| WorkerPanic {
        worker: worker.to_string(),
        message: panic_message(payload.as_ref()),
    })
}

/// Spawn a named worker whose panic is contained and returned from the join.
///
/// The thread name is prefixed with `lumina-` so a panic report identifies the
/// subsystem even when the panic hook prints the thread name.
pub fn spawn_guarded<F>(
    worker: &'static str,
    body: F,
) -> io::Result<JoinHandle<Result<(), WorkerPanic>>>
where
    F: FnOnce() + Send + UnwindSafe + 'static,
{
    thread::Builder::new()
        .name(format!("lumina-{worker}"))
        .spawn(move || run_guarded(worker, body))
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_worker_finishes_successfully() {
        let result = run_guarded("audio", || {});
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn a_panicking_worker_is_contained_with_its_message() {
        let result = run_guarded("media", || panic!("device disappeared"));
        let panic = result.expect_err("the panic must be returned");
        assert_eq!(panic.worker(), "media");
        assert_eq!(panic.message(), "device disappeared");
        assert!(panic.to_string().contains("media"));
    }

    #[test]
    fn a_string_payload_is_preserved() {
        let result = run_guarded("ipc", || {
            std::panic::panic_any(String::from("protocol desync"));
        });
        assert_eq!(
            result.expect_err("the panic must be returned").message(),
            "protocol desync"
        );
    }

    #[test]
    fn a_spawned_worker_returns_its_outcome_on_join() {
        let handle = spawn_guarded("audio", || panic!("capture loop failed"))
            .expect("the thread should spawn");
        let result = handle.join().expect("the thread itself must not unwind");
        let panic = result.expect_err("the worker panic must be reported");
        assert_eq!(panic.worker(), "audio");
        assert_eq!(panic.message(), "capture loop failed");
    }

    #[test]
    fn a_spawned_worker_reports_success() {
        let handle = spawn_guarded("media", || {}).expect("the thread should spawn");
        assert_eq!(handle.join().expect("the thread must not unwind"), Ok(()));
    }
}
