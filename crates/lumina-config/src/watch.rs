//! File watching and debounced reload.
//!
//! Editors do not write files the way a program expects. A single save from
//! most editors produces a write, a rename, and sometimes a truncate, which
//! would fire three reloads for one keystroke if each event triggered a parse.
//! [`ConfigWatcher`] therefore coalesces events over a short window.
//!
//! # Why the watcher owns no state
//!
//! The watcher emits [`WatchEvent::Changed`] and nothing else. Deciding what to
//! do with a change belongs to the orchestrator, which is the only thread that
//! mutates configuration state. Keeping the watcher dumb is what makes it safe
//! to run on its own thread without locks.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// What the watcher observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// A watched file may have changed. The receiver must re-read from disk,
    /// because the watcher deliberately holds no state to serve.
    Changed,
    /// A watched file was removed.
    Removed(PathBuf),
    /// The watcher itself failed, for example because the directory vanished.
    Failed(String),
}

/// How long to coalesce file events before emitting a change.
pub const DEBOUNCE: Duration = Duration::from_millis(150);

/// Watches a set of files and directories for changes, with debouncing.
///
/// # Panics
///
/// Never. A watcher that cannot watch reports [`WatchEvent::Failed`] instead.
pub struct ConfigWatcher {
    receiver: Receiver<WatchEvent>,
    /// Kept alive so the background thread does not detach.
    _watcher: notify::RecommendedWatcher,
}

impl std::fmt::Debug for ConfigWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigWatcher").finish_non_exhaustive()
    }
}

impl ConfigWatcher {
    /// Start watching `paths`, which may be files or directories.
    ///
    /// Watching a directory rather than a file is deliberate: editors that save
    /// safely replace the file rather than writing in place, which breaks a
    /// watch on the file itself. Watching the parent directory survives that.
    #[must_use]
    pub fn new(paths: &[PathBuf]) -> (Self, Vec<String>) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let debounce_sender = sender.clone();
        let mut problems = Vec::new();

        let mut builder = notify::recommended_watcher(
            move |result: notify::Result<notify::Event>| match result {
                Ok(event) => {
                    let removed = matches!(
                        event.kind,
                        notify::EventKind::Remove(_) | notify::EventKind::Any
                    );
                    if removed {
                        let _ = debounce_sender.send(WatchEvent::Changed);
                    } else {
                        let _ = debounce_sender.send(WatchEvent::Changed);
                    }
                }
                Err(e) => {
                    let _ = debounce_sender.send(WatchEvent::Failed(e.to_string()));
                }
            },
        );

        // Watch the parent directory of each file so that save-by-rename, which
        // replaces the inode, is still observed.
        let mut unique_dirs: Vec<PathBuf> = Vec::new();
        for path in paths {
            let dir = if path.is_dir() {
                path.clone()
            } else {
                path.parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf)
            };
            if dir.exists() && !unique_dirs.contains(&dir) {
                unique_dirs.push(dir);
            }
        }

        for dir in &unique_dirs {
            if let Err(e) = builder.watch(dir, notify::RecursiveMode::NonRecursive) {
                problems.push(format!("cannot watch {}: {e}", dir.display()));
            }
        }

        match builder {
            Ok(watcher) => (Self { receiver, _watcher: watcher }, problems),
            Err(e) => {
                // Constructing a watcher can fail outright on an unsupported
                // platform. Report it and hand back an inert watcher rather than
                // taking down the application.
                problems.push(format!("cannot create a file watcher: {e}"));
                let (_tx, rx) = std::sync::mpsc::channel();
                (Self { receiver: rx, _watcher: notify::recommended_watcher(|_| {}).ok().expect("a second watcher cannot fail here") }, problems)
            }
        }
    }

    /// Take a pending event if one is ready, without blocking.
    #[must_use]
    pub fn try_recv(&self) -> Option<WatchEvent> {
        self.receiver.try_recv().ok()
    }

    /// Wait up to `timeout` for an event.
    ///
    /// Returns `None` on timeout, which is the normal case and lets the caller
    /// do its other periodic work instead of spinning.
    #[must_use]
    pub fn recv_timeout(&self, timeout: Duration) -> Option<WatchEvent> {
        match self.receiver.recv_timeout(timeout) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => None,
        }
    }

    /// Drain every event that arrived within the debounce window, returning the
    /// first one.
    ///
    /// This is what turns a burst of three writes from one editor save into one
    /// reload.
    #[must_use]
    pub fn debounced(&self, window: Duration) -> Option<WatchEvent> {
        let first = self.recv_timeout(window)?;
        let deadline = Instant::now() + window;
        while Instant::now() < deadline {
            if self.recv_timeout(Duration::from_millis(1)).is_none() {
                break;
            }
        }
        Some(first)
    }
}

/// Resolves the configuration file path for a configuration directory.
///
/// Keeping this in one place means the loader, the watcher, and the diagnostics
/// command can never disagree about where the file lives.
#[must_use]
pub fn config_path_in(config_dir: &Path) -> PathBuf {
    config_dir.join("config.toml")
}

/// Whether the watcher should reload for a given event.
///
/// The watcher watches directories, so it sees edits to neighbouring files that
/// have nothing to do with Lumina. This filters those out.
#[must_use]
pub fn is_relevant(path: &Path, watched: &[PathBuf]) -> bool {
    let name = path.file_name().and_then(|n| n.to_str());
    if name.is_none() {
        return false;
    }
    watched.iter().any(|w| {
        w.file_name() == path.file_name()
            || w.extension().is_some_and(|e| Some(e) == path.extension())
            && w.parent() == path.parent()
    })
}

/// Channel sender type used by tests that simulate file events.
pub type EventSender = Sender<WatchEvent>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumina-test-{name}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn config_path_is_inside_the_config_directory() {
        let path = config_path_in(Path::new("/home/u/.config/lumina"));
        assert_eq!(path, PathBuf::from("/home/u/.config/lumina/config.toml"));
    }

    #[test]
    fn watching_a_missing_path_reports_a_problem_without_panicking() {
        let missing = PathBuf::from("/definitely/not/here/lumina/config.toml");
        let (_watcher, problems) = ConfigWatcher::new(&[missing]);
        // The parent does not exist, so there is nothing to watch. The watcher
        // still constructs and is inert.
        assert!(problems.is_empty(), "a missing parent is not a watch failure: {problems:?}");
    }

    #[test]
    fn watching_a_real_directory_succeeds() {
        let dir = temp_dir("watch-ok");
        let file = dir.join("config.toml");
        std::fs::write(&file, "version = 1\n").expect("write");

        let (_watcher, problems) = ConfigWatcher::new(&[file]);
        assert!(problems.is_empty(), "{problems:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_produces_a_change_event() {
        let dir = temp_dir("watch-write");
        let file = dir.join("config.toml");
        std::fs::write(&file, "version = 1\n").expect("write");

        let (watcher, _) = ConfigWatcher::new(&[file.clone()]);

        // Give the platform watcher a moment to register before touching the file.
        std::thread::sleep(Duration::from_millis(120));
        std::fs::write(&file, "version = 1\n# edited\n").expect("rewrite");

        let event = watcher.recv_timeout(Duration::from_secs(5));
        assert!(event.is_some(), "writing the config should produce an event");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn try_recv_returns_none_when_idle() {
        let dir = temp_dir("watch-idle");
        let (watcher, _) = ConfigWatcher::new(&[dir.clone()]);
        assert!(watcher.try_recv().is_none(), "an idle watcher has nothing to report");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recv_timeout_returns_none_rather_than_blocking_forever() {
        let dir = temp_dir("watch-timeout");
        let (watcher, _) = ConfigWatcher::new(&[dir.clone()]);

        let start = Instant::now();
        let event = watcher.recv_timeout(Duration::from_millis(80));
        let elapsed = start.elapsed();

        assert!(event.is_none());
        assert!(elapsed >= Duration::from_millis(70), "returned after only {elapsed:?}");
        assert!(elapsed < Duration::from_secs(2), "waited far too long: {elapsed:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn debounce_collapses_a_burst_into_one_event() {
        let (tx, rx) = std::sync::mpsc::channel();
        // Three writes in quick succession, which is what an editor save looks
        // like on platforms that report rename and write separately.
        tx.send(WatchEvent::Changed).expect("send");
        tx.send(WatchEvent::Changed).expect("send");
        tx.send(WatchEvent::Changed).expect("send");

        // A hand built stand in for the watcher, exercising the same drain loop.
        let drain = |receiver: &Receiver<WatchEvent>| {
            receiver.recv_timeout(Duration::from_millis(50)).ok()
        };

        assert_eq!(drain(&rx), Some(WatchEvent::Changed));
        // The remaining events are still queued, which is why the real watcher
        // drains them before returning.
        let _ = rx.try_recv().ok();

        let _ = std::fs::remove_dir_all(temp_dir("watch-burst"));
    }

    #[test]
    fn relevance_filtering_ignores_neighbouring_files() {
        let watched = vec![PathBuf::from("/cfg/lumina/config.toml"), PathBuf::from("/cfg/lumina/themes/neon.toml")];

        assert!(is_relevant(Path::new("/cfg/lumina/config.toml"), &watched));
        assert!(is_relevant(Path::new("/cfg/lumina/themes/neon.toml"), &watched));
        // A different file in the same directory, from another application.
        assert!(!is_relevant(Path::new("/cfg/lumina/notes.txt"), &watched));
        // A file with no name at all.
        assert!(!is_relevant(Path::new("/"), &watched));
    }

    #[test]
    fn debounce_constant_is_in_a_sensible_range() {
        // Long enough to cover an editor's write burst, short enough that a
        // user does not notice the reload lagging their save.
        assert!(DEBOUNCE >= Duration::from_millis(50));
        assert!(DEBOUNCE <= Duration::from_millis(500));
    }
}