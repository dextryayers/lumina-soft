//! The script widget: runs a user command and shows its recent output.
//!
//! # Security
//!
//! A script entry is code the user wrote in their own config, but Lumina still
//! treats it carefully:
//!
//! - Command and arguments are executed directly. No shell is involved, so
//!   `$VAR`, `%VAR%`, pipes, and redirects are literal text unless the user
//!   explicitly names a shell as the program. This removes a whole class of
//!   quoting bugs and accidental expansion.
//! - Every run has a timeout. On expiry the process is killed, not detached.
//! - Lines are truncated to [`MAX_LINE_CHARS`] characters and only the newest
//!   [`MAX_LINES`] are retained, so a runaway `yes` cannot grow memory.
//! - Nothing is retried in a tight loop: the next run waits for the configured
//!   interval measured from the previous completion.
//!
//! The same rules are documented for users in `docs/script-widget.md`.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lumina_config::schema::ScriptEntry;
use lumina_core::geometry::{Rect, Size};
use lumina_core::state::{IslandState, StateRequest};

use crate::paint::{self, CRITICAL, DIM, GOOD, MUTED, TEXT, WARN};
use crate::widget::{DrawList, Widget, WidgetContext, WidgetInput};

/// Longest line kept from a script, in characters.
pub const MAX_LINE_CHARS: usize = 120;
/// Most lines shown from one script.
pub const MAX_LINES: usize = 8;
/// Shortest automatic refresh interval.
pub const MIN_INTERVAL_MS: u64 = 250;

/// The outcome of the most recent run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ScriptStatus {
    /// No run has finished yet.
    #[default]
    Idle,
    /// A process is running.
    Running,
    /// The process exited successfully, with an optional exit code.
    Ok(Option<i32>),
    /// The process was killed after its timeout.
    TimedOut,
    /// The process could not be spawned, or exited on a signal.
    Failed(String),
}

impl ScriptStatus {
    /// The color used for the status indicator.
    #[must_use]
    pub fn color(&self) -> [f32; 4] {
        match self {
            Self::Idle => MUTED,
            Self::Running => WARN,
            Self::Ok(_) => GOOD,
            Self::TimedOut | Self::Failed(_) => CRITICAL,
        }
    }

    /// True while a process is alive.
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }
}

#[derive(Debug, Default)]
struct ScriptBuffer {
    lines: VecDeque<String>,
    capacity: usize,
}

impl ScriptBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(capacity),
            capacity: capacity.clamp(1, MAX_LINES),
        }
    }

    fn push_line(&mut self, line: &str) {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            return;
        }
        let truncated = truncate_chars(trimmed, MAX_LINE_CHARS);
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(truncated);
    }
}

/// Keep at most `limit` characters, appending an ellipsis when cut.
///
/// Cutting by characters rather than bytes keeps the result valid UTF-8 even
/// when a script prints emoji or CJK text. The ellipsis counts against the
/// limit, so a truncated line never exceeds it.
#[must_use]
pub fn truncate_chars(input: &str, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    if input.chars().count() <= limit {
        return input.to_string();
    }
    let keep = limit.saturating_sub(1);
    let mut out: String = input.chars().take(keep).collect();
    out.push('…');
    out
}

/// Split a command line into a program and arguments.
///
/// Supports single and double quotes and backslash escapes inside double
/// quotes. There is no variable expansion, globbing, or operator handling; the
/// result is passed straight to [`Command`].
#[must_use]
pub fn split_command(command: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for c in chars.by_ref() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        match quote {
            Some(q) => {
                if c == '\\' && q == '"' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                } else {
                    current.push(c);
                }
            }
            None => {
                if c == '\\' {
                    escaped = true;
                } else if c == '"' || c == '\'' {
                    quote = Some(c);
                } else if c.is_whitespace() {
                    if !current.is_empty() {
                        parts.push(std::mem::take(&mut current));
                    }
                } else {
                    current.push(c);
                }
            }
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// The script widget for one configured entry.
#[derive(Debug)]
pub struct ScriptWidget {
    entry: ScriptEntry,
    buffer: Arc<Mutex<ScriptBuffer>>,
    child: Option<Child>,
    readers: Vec<JoinHandle<()>>,
    started_at: Option<Instant>,
    last_finished: Option<Instant>,
    display_until: Option<Instant>,
    status: ScriptStatus,
}

impl ScriptWidget {
    /// Build a widget for one script entry.
    #[must_use]
    pub fn new(entry: ScriptEntry) -> Self {
        let buffer = Arc::new(Mutex::new(ScriptBuffer::new(entry.max_lines as usize)));
        Self {
            entry,
            buffer,
            child: None,
            readers: Vec::new(),
            started_at: None,
            last_finished: None,
            display_until: None,
            status: ScriptStatus::Idle,
        }
    }

    /// The configured entry.
    #[must_use]
    pub fn entry(&self) -> &ScriptEntry {
        &self.entry
    }

    /// The most recent lines, oldest first.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.buffer
            .lock()
            .map(|buffer| buffer.lines.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// The current status.
    #[must_use]
    pub fn status(&self) -> &ScriptStatus {
        &self.status
    }

    /// The automatic refresh interval.
    #[must_use]
    pub fn interval(&self) -> Duration {
        Duration::from_millis(u64::from(self.entry.interval_ms).max(MIN_INTERVAL_MS))
    }

    /// The run timeout.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(u64::from(self.entry.timeout_ms.max(1_000)))
    }

    /// How long the result stays visible after the process exits.
    #[must_use]
    pub fn display_time(&self) -> Duration {
        Duration::from_millis(u64::from(self.entry.display_ms.max(500)))
    }

    /// True when the next automatic run is due.
    #[must_use]
    pub fn is_due(&self, now: Instant) -> bool {
        if self.child.is_some() {
            return false;
        }
        match self.last_finished.or(self.started_at) {
            Some(last) => now.saturating_duration_since(last) >= self.interval(),
            None => true,
        }
    }

    /// Start the process. Returns an error when it cannot be spawned.
    pub fn start(&mut self, now: Instant) -> Result<(), String> {
        if self.child.is_some() {
            return Err("script is already running".to_string());
        }
        let mut parts = if self.entry.args.is_empty() {
            split_command(&self.entry.command)
        } else {
            let mut parts = vec![self.entry.command.clone()];
            parts.extend(self.entry.args.iter().cloned());
            parts
        };
        if parts.is_empty() || parts[0].is_empty() {
            self.status = ScriptStatus::Failed("empty command".to_string());
            return Err("empty command".to_string());
        }
        let program = parts.remove(0);

        let mut command = Command::new(&program);
        command.args(&parts);
        if !self.entry.cwd.is_empty() {
            command.current_dir(PathBuf::from(&self.entry.cwd));
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let message = format!("{program}: {error}");
                self.status = ScriptStatus::Failed(message.clone());
                self.last_finished = Some(now);
                self.display_until = Some(now + self.display_time());
                return Err(message);
            }
        };

        if let Ok(buffer) = self.buffer.lock() {
            buffer.lines.clear();
        }
        self.readers.clear();
        if let Some(stdout) = child.stdout.take() {
            self.readers
                .push(spawn_reader(stdout, Arc::clone(&self.buffer)));
        }
        if let Some(stderr) = child.stderr.take() {
            self.readers
                .push(spawn_reader(stderr, Arc::clone(&self.buffer)));
        }

        self.child = Some(child);
        self.started_at = Some(now);
        self.last_finished = None;
        self.display_until = None;
        self.status = ScriptStatus::Running;
        Ok(())
    }

    /// Check the running process, enforcing the timeout, and clear old output.
    pub fn poll(&mut self, now: Instant) {
        if let Some(child) = self.child.as_mut() {
            let timed_out = self
                .started_at
                .is_some_and(|started| now.saturating_duration_since(started) >= self.timeout());
            if timed_out {
                let _ = child.kill();
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.child = None;
                    self.last_finished = Some(now);
                    self.display_until = Some(now + self.display_time());
                    if timed_out {
                        self.status = ScriptStatus::TimedOut;
                    } else if status.success() {
                        self.status = ScriptStatus::Ok(status.code());
                    } else {
                        self.status = match status.code() {
                            Some(code) => ScriptStatus::Failed(format!("exit {code}")),
                            None => ScriptStatus::Failed("terminated".to_string()),
                        };
                    }
                    self.join_readers();
                }
                Ok(None) => {}
                Err(error) => {
                    self.child = None;
                    self.last_finished = Some(now);
                    self.display_until = Some(now + self.display_time());
                    self.status = ScriptStatus::Failed(error.to_string());
                    self.join_readers();
                }
            }
        }

        if self.display_until.is_some_and(|until| now >= until) {
            self.display_until = None;
            self.status = ScriptStatus::Idle;
        }
    }

    /// Run the process when due, then poll. Returns true when a run started.
    pub fn tick(&mut self, now: Instant) -> bool {
        self.poll(now);
        if self.is_due(now) {
            let started = self.start(now).is_ok();
            if started {
                return true;
            }
        }
        false
    }

    fn join_readers(&mut self) {
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }

    /// The output currently visible, or a placeholder when there is none.
    #[must_use]
    pub fn display_lines(&self) -> Vec<String> {
        let lines = self.lines();
        if lines.is_empty() {
            match &self.status {
                ScriptStatus::Running => vec!["…".to_string()],
                ScriptStatus::Failed(message) => vec![message.clone()],
                ScriptStatus::TimedOut => vec!["timed out".to_string()],
                _ => Vec::new(),
            }
        } else {
            lines
        }
    }
}

fn spawn_reader(pipe: impl std::io::Read + Send + 'static, buffer: Arc<Mutex<ScriptBuffer>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let reader = BufReader::new(pipe);
        for line in reader.lines().map_while(Result::ok) {
            if let Ok(mut buffer) = buffer.lock() {
                buffer.push_line(&line);
            }
        }
    })
}

impl Widget for ScriptWidget {
    fn id(&self) -> &'static str {
        "script"
    }

    fn measure(&mut self, _ctx: &WidgetContext) -> Size {
        let lines = self.entry.max_lines.clamp(1, 12) as f32;
        Size::new(190.0, 18.0 + lines * 10.0)
    }

    fn draw(&mut self, out: &mut DrawList, _ctx: &WidgetContext, rect: Rect) {
        let accent = self.status.color();
        paint::block(out, rect, [0.08, 0.09, 0.11, 0.9], 8.0);
        paint::block(
            out,
            Rect::new(rect.x + 6.0, rect.y + 6.0, 3.0, 3.0),
            accent,
            1.5,
        );
        paint::label(
            out,
            Rect::new(rect.x + 13.0, rect.y + 3.0, (rect.width - 20.0).max(1.0), 8.0),
            TEXT,
        );

        let max_lines = self.entry.max_lines.clamp(1, 12) as usize;
        let mut drawn = 0;
        if let Ok(buffer) = self.buffer.lock() {
            for (index, _line) in buffer.lines.iter().take(max_lines).enumerate() {
                paint::label(
                    out,
                    Rect::new(
                        rect.x + 8.0,
                        rect.y + 16.0 + index as f32 * 10.0,
                        (rect.width - 16.0).max(1.0),
                        6.0,
                    ),
                    DIM,
                );
                drawn += 1;
            }
        }
        if drawn == 0 {
            let fallback = match &self.status {
                ScriptStatus::Running => Some("…"),
                ScriptStatus::Failed(message) => Some(message.as_str()),
                ScriptStatus::TimedOut => Some("timed out"),
                ScriptStatus::Idle | ScriptStatus::Ok(_) => None,
            };
            if fallback.is_some() {
                paint::label(
                    out,
                    Rect::new(rect.x + 8.0, rect.y + 16.0, (rect.width - 16.0).max(1.0), 6.0),
                    DIM,
                );
            }
        }
    }

    fn on_input(&mut self, ev: &WidgetInput, _ctx: &WidgetContext, _rect: Rect) -> bool {
        match ev {
            WidgetInput::Click(_) => {
                if self.child.is_none() {
                    let _ = self.start(Instant::now());
                }
                true
            }
            WidgetInput::RightClick(_) => {
                self.display_until = None;
                self.status = if self.child.is_some() {
                    ScriptStatus::Running
                } else {
                    ScriptStatus::Idle
                };
                true
            }
            _ => false,
        }
    }

    fn request(&self, _ctx: &WidgetContext) -> Option<StateRequest> {
        let until = self.display_until?;
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        Some(StateRequest::until(
            IslandState::Notify,
            now_ms().saturating_add(remaining.as_millis() as u64),
        ))
    }

    fn priority(&self) -> u8 {
        IslandState::Notify.priority()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(command: &str, args: &[&str]) -> ScriptEntry {
        ScriptEntry {
            id: "test".to_string(),
            label: "Test".to_string(),
            command: command.to_string(),
            args: args.iter().map(|s| (*s).to_string()).collect(),
            interval_ms: 60_000,
            timeout_ms: 5_000,
            display_ms: 8_000,
            max_lines: 4,
            show_in_island: true,
            ..ScriptEntry::default()
        }
    }

    fn ctx() -> WidgetContext {
        WidgetContext::new(IslandState::Notify, Size::new(320.0, 120.0))
    }

    #[test]
    fn command_lines_split_on_whitespace_and_quotes() {
        assert_eq!(
            split_command("notify-send \"hello world\" --icon bell"),
            vec!["notify-send", "hello world", "--icon", "bell"]
        );
        assert_eq!(
            split_command("sh -c 'echo hi'"),
            vec!["sh", "-c", "echo hi"]
        );
        assert_eq!(split_command("a\\ b"), vec!["a b"]);
        assert!(split_command("   ").is_empty());
    }

    #[test]
    fn long_lines_are_truncated_on_a_character_boundary() {
        let text = "é".repeat(200);
        let cut = truncate_chars(&text, 10);
        assert_eq!(cut.chars().count(), 10);
        assert!(cut.ends_with('…'));
        assert_eq!(truncate_chars("short", 10), "short");
        assert_eq!(truncate_chars("anything", 0), "");
    }

    #[test]
    fn the_buffer_keeps_only_the_newest_lines() {
        let mut buffer = ScriptBuffer::new(2);
        buffer.push_line("one");
        buffer.push_line("two");
        buffer.push_line("three");
        assert_eq!(buffer.lines.iter().cloned().collect::<Vec<_>>(), vec!["two", "three"]);
    }

    #[test]
    fn blank_lines_are_ignored() {
        let mut buffer = ScriptBuffer::new(2);
        buffer.push_line("   ");
        buffer.push_line("\n");
        buffer.push_line("value");
        assert_eq!(buffer.lines.len(), 1);
    }

    #[test]
    fn durations_are_clamped_to_sane_minimums() {
        let widget = ScriptWidget::new(ScriptEntry {
            interval_ms: 0,
            timeout_ms: 0,
            display_ms: 0,
            ..entry("true", &[])
        });
        assert_eq!(widget.interval(), Duration::from_millis(MIN_INTERVAL_MS));
        assert_eq!(widget.timeout(), Duration::from_secs(1));
        assert_eq!(widget.display_time(), Duration::from_millis(500));
    }

    #[test]
    fn a_widget_with_no_runs_is_due() {
        let widget = ScriptWidget::new(entry("true", &[]));
        assert!(widget.is_due(Instant::now()));
        assert_eq!(*widget.status(), ScriptStatus::Idle);
    }

    #[test]
    fn an_empty_command_fails_without_spawning() {
        let mut widget = ScriptWidget::new(entry("", &[]));
        let error = widget.start(Instant::now()).expect_err("empty command fails");
        assert!(error.contains("empty command"));
        assert!(matches!(widget.status(), ScriptStatus::Failed(_)));
    }

    #[test]
    fn missing_programs_fail_with_an_error_line() {
        let mut widget = ScriptWidget::new(entry(
            "lumina-command-that-does-not-exist-9f3a",
            &[],
        ));
        widget.start(Instant::now()).expect_err("spawn fails");
        let lines = widget.display_lines();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("lumina-command-that-does-not-exist"));
    }

    #[test]
    fn display_lines_fall_back_to_the_status() {
        let mut widget = ScriptWidget::new(entry("true", &[]));
        widget.status = ScriptStatus::TimedOut;
        assert_eq!(widget.display_lines(), vec!["timed out".to_string()]);
        widget.status = ScriptStatus::Running;
        assert_eq!(widget.display_lines(), vec!["…".to_string()]);
    }

    #[test]
    fn drawing_without_output_still_draws_the_header() {
        let mut widget = ScriptWidget::new(entry("true", &[]));
        let mut out = DrawList::new();
        let ctx = ctx();
        widget.draw(&mut out, &ctx, Rect::new(0.0, 0.0, 190.0, 40.0));
        assert!(out.len() >= 3);
    }

    #[test]
    fn status_colors_are_distinct() {
        assert_ne!(ScriptStatus::Ok(Some(0)).color(), ScriptStatus::Failed("x".to_string()).color());
        assert_eq!(ScriptStatus::Running.color(), WARN);
        assert_eq!(ScriptStatus::TimedOut.color(), CRITICAL);
        assert_eq!(ScriptStatus::Idle.color(), MUTED);
        assert_eq!(ScriptStatus::Ok(None).color(), GOOD);
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn a_real_process_reports_output_and_success() {
        let (program, args) = if cfg!(windows) {
            ("cmd", vec!["/C", "echo lumina-script"])
        } else {
            ("sh", vec!["-c", "echo lumina-script"])
        };
        let mut widget = ScriptWidget::new(entry(program, &args));
        widget.start(Instant::now()).expect("spawns");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            widget.poll(Instant::now());
            if !widget.status().is_running() {
                break;
            }
            assert!(Instant::now() < deadline, "script did not finish");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            widget.lines().iter().any(|line| line.contains("lumina-script")),
            "output was {:?}",
            widget.lines()
        );
        assert!(matches!(widget.status(), ScriptStatus::Ok(Some(0))));
        assert!(widget.display_lines().len() >= 1);
    }

    #[test]
    fn a_timed_out_process_is_killed() {
        let (program, args) = if cfg!(windows) {
            ("ping", vec!["-n", "30", "127.0.0.1"])
        } else {
            ("sleep", vec!["30"])
        };
        let mut widget = ScriptWidget::new(ScriptEntry {
            timeout_ms: 1_000,
            ..entry(program, &args)
        });
        widget.start(Instant::now()).expect("spawns");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            widget.poll(Instant::now());
            if !widget.status().is_running() {
                break;
            }
            assert!(Instant::now() < deadline, "timeout was not enforced");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(*widget.status(), ScriptStatus::TimedOut);
    }

    #[test]
    fn the_widget_advertises_notify_while_output_is_fresh() {
        let mut widget = ScriptWidget::new(entry("true", &[]));
        assert!(widget.request(&ctx()).is_none());
        widget.display_until = Some(Instant::now() + Duration::from_secs(5));
        let request = widget.request(&ctx()).expect("fresh output requests a state");
        assert_eq!(request.state, IslandState::Notify);
    }

    #[test]
    fn clicking_starts_the_process() {
        let (program, args) = if cfg!(windows) {
            ("cmd", vec!["/C", "echo clicked"])
        } else {
            ("sh", vec!["-c", "echo clicked"])
        };
        let mut widget = ScriptWidget::new(entry(program, &args));
        let ctx = ctx();
        let rect = Rect::new(0.0, 0.0, 190.0, 40.0);
        assert!(widget.on_input(&WidgetInput::Click(lumina_core::geometry::Vec2::ZERO), &ctx, rect));
        assert!(widget.status().is_running() || !widget.lines().is_empty());
        widget.poll(Instant::now() + Duration::from_secs(1));
    }
}
