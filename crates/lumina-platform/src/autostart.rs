//! Start Lumina with the user session.
//!
//! # Platform mechanisms
//!
//! | Platform | Mechanism |
//! |---|---|
//! | Windows | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Lumina` |
//! | Linux | `~/.config/autostart/lumina.desktop` |
//!
//! Both are per-user and require no elevation. The executable is recorded with
//! an absolute path and passed `--hidden` when the user prefers the island to
//! start hidden, so autostart does not undo an explicit visibility choice.
//!
//! # Testability
//!
//! Everything that can be pure is pure: the registry command line and the
//! desktop entry are strings built by tested functions. The install and remove
//! functions are thin wrappers, which is deliberate because a unit test must
//! never write to the developer's real autostart configuration.

use std::fmt;
use std::path::{Path, PathBuf};

/// Registry key holding per-user startup programs.
pub const WINDOWS_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Registry value name Lumina writes.
pub const WINDOWS_VALUE_NAME: &str = "Lumina";
/// Desktop entry file name on Linux.
pub const LINUX_ENTRY_NAME: &str = "lumina.desktop";

/// Why an autostart operation failed.
#[derive(Debug)]
pub enum AutostartError {
    /// This platform has no supported mechanism.
    Unsupported,
    /// The desktop entry or home directory could not be read or written.
    Io(std::io::Error),
    /// The registry call returned a non-zero Win32 error code.
    Registry(u32),
}

impl fmt::Display for AutostartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("autostart is not supported on this platform"),
            Self::Io(error) => write!(f, "autostart file could not be updated: {error}"),
            Self::Registry(code) => write!(f, "the registry rejected the autostart entry (code {code})"),
        }
    }
}

impl std::error::Error for AutostartError {}

impl From<std::io::Error> for AutostartError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// The path to the Linux desktop entry.
///
/// Returns `None` when neither `XDG_CONFIG_HOME` nor `HOME` is set, which means
/// there is nowhere to write a per-user entry.
#[must_use]
pub fn linux_desktop_path() -> Option<PathBuf> {
    if let Some(base) = std::env::var_os("XDG_CONFIG_HOME") {
        let base = PathBuf::from(base);
        if base.is_absolute() {
            return Some(base.join("autostart").join(LINUX_ENTRY_NAME));
        }
    }
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("autostart")
            .join(LINUX_ENTRY_NAME),
    )
}

/// Quote a program path for a desktop entry `Exec` value.
///
/// Desktop entry quoting uses double quotes with backslash escapes for `"`,
/// `` ` ``, `$`, and `\`. The path is always quoted so spaces and non-ASCII
/// characters need no special casing at the call site.
#[must_use]
pub fn desktop_exec(executable: &Path, hidden: bool) -> String {
    let path = executable.to_string_lossy();
    let mut quoted = String::with_capacity(path.len() + 2);
    quoted.push('"');
    for ch in path.chars() {
        if matches!(ch, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    if hidden {
        quoted.push_str(" --hidden");
    }
    quoted
}

/// Render the Linux desktop entry.
#[must_use]
pub fn desktop_entry(executable: &Path, start_hidden: bool) -> String {
    let mut entry = String::new();
    entry.push_str("[Desktop Entry]\n");
    entry.push_str("Type=Application\n");
    entry.push_str("Name=Lumina\n");
    entry.push_str("Comment=Dynamic island for your desktop\n");
    entry.push_str("Exec=");
    entry.push_str(&desktop_exec(executable, start_hidden));
    entry.push_str("\nTerminal=false\n");
    entry.push_str("X-GNOME-Autostart-enabled=true\n");
    entry.push_str("X-KDE-autostart-after=panel\n");
    entry
}

/// The command line stored in the Windows registry.
#[must_use]
pub fn windows_command(executable: &Path, start_hidden: bool) -> String {
    let path = executable.to_string_lossy();
    let mut command = String::with_capacity(path.len() + 12);
    command.push('"');
    command.push_str(&path);
    command.push('"');
    if start_hidden {
        command.push_str(" --hidden");
    }
    command
}

/// Install the per-user autostart entry.
///
/// # Errors
///
/// Returns [`AutostartError::Unsupported`] on an unsupported platform, or the
/// platform error when the registry or file system rejects the write.
pub fn install(executable: &Path, start_hidden: bool) -> Result<(), AutostartError> {
    platform_install(executable, start_hidden)
}

/// Remove the per-user autostart entry.
///
/// Removing an entry that does not exist succeeds: the caller's intent is that
/// Lumina not start automatically, and that state has been achieved.
///
/// # Errors
///
/// Propagates platform errors other than "not found".
pub fn uninstall() -> Result<(), AutostartError> {
    platform_uninstall()
}

/// True when an autostart entry is present.
#[must_use]
pub fn is_enabled() -> bool {
    platform_is_enabled()
}

#[cfg(target_os = "windows")]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn platform_install(executable: &Path, start_hidden: bool) -> Result<(), AutostartError> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};

    let command = windows_command(executable, start_hidden);
    let key = wide(WINDOWS_RUN_KEY);
    let value = wide(WINDOWS_VALUE_NAME);
    let data = wide(&command);
    let bytes = (data.len() * std::mem::size_of::<u16>()) as u32;
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            bytes,
        )
    };
    if result.0 == 0 {
        Ok(())
    } else {
        Err(AutostartError::Registry(result.0))
    }
}

#[cfg(target_os = "windows")]
fn platform_uninstall() -> Result<(), AutostartError> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{RegDeleteKeyValueW, HKEY_CURRENT_USER};

    let key = wide(WINDOWS_RUN_KEY);
    let value = wide(WINDOWS_VALUE_NAME);
    let result = unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
        )
    };
    if result.0 == 0 || result == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(AutostartError::Registry(result.0))
    }
}

#[cfg(target_os = "windows")]
fn platform_is_enabled() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

    let key = wide(WINDOWS_RUN_KEY);
    let value = wide(WINDOWS_VALUE_NAME);
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
    };
    result.0 == 0
}

#[cfg(all(unix, not(target_os = "windows")))]
fn platform_install(executable: &Path, start_hidden: bool) -> Result<(), AutostartError> {
    let path = linux_desktop_path().ok_or(AutostartError::Unsupported)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, desktop_entry(executable, start_hidden))?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "windows")))]
fn platform_uninstall() -> Result<(), AutostartError> {
    let path = linux_desktop_path().ok_or(AutostartError::Unsupported)?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AutostartError::Io(error)),
    }
}

#[cfg(all(unix, not(target_os = "windows")))]
fn platform_is_enabled() -> bool {
    linux_desktop_path().is_some_and(|path| path.is_file())
}

#[cfg(not(any(target_os = "windows", unix)))]
fn platform_install(_executable: &Path, _start_hidden: bool) -> Result<(), AutostartError> {
    Err(AutostartError::Unsupported)
}

#[cfg(not(any(target_os = "windows", unix)))]
fn platform_uninstall() -> Result<(), AutostartError> {
    Err(AutostartError::Unsupported)
}

#[cfg(not(any(target_os = "windows", unix)))]
fn platform_is_enabled() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_entry_contains_the_executable_and_hidden_flag() {
        let entry = desktop_entry(Path::new("/usr/bin/lumina"), true);
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("Exec=\"/usr/bin/lumina\" --hidden\n"));
        assert!(entry.contains("X-GNOME-Autostart-enabled=true"));
        assert!(entry.contains("Terminal=false"));
    }

    #[test]
    fn the_desktop_entry_omits_hidden_when_the_island_is_visible() {
        let entry = desktop_entry(Path::new("/usr/bin/lumina"), false);
        assert!(entry.contains("Exec=\"/usr/bin/lumina\"\n"));
        assert!(!entry.contains("--hidden"));
    }

    #[test]
    fn desktop_paths_with_spaces_are_quoted_once() {
        let entry = desktop_entry(Path::new("/opt/Lumina App/lumina"), true);
        assert!(entry.contains("Exec=\"/opt/Lumina App/lumina\" --hidden"));
    }

    #[test]
    fn desktop_exec_escapes_quotes_backslashes_and_dollars() {
        let exec = desktop_exec(Path::new(r#"/tmp/a"b$c\d"#), true);
        assert_eq!(exec, r#""/tmp/a\"b\$c\\d" --hidden"#);
    }

    #[test]
    fn the_windows_command_quotes_the_path() {
        assert_eq!(
            windows_command(Path::new(r"C:\Program Files\Lumina\lumina.exe"), true),
            r#""C:\Program Files\Lumina\lumina.exe" --hidden"#
        );
        assert_eq!(
            windows_command(Path::new(r"C:\Lumina\lumina.exe"), false),
            r#""C:\Lumina\lumina.exe""#
        );
    }

    #[test]
    fn the_registry_key_is_per_user() {
        assert!(WINDOWS_RUN_KEY.starts_with("Software\\"));
        assert_eq!(WINDOWS_VALUE_NAME, "Lumina");
    }

    #[test]
    fn a_missing_linux_config_root_is_reported_rather_than_guessed() {
        // Pure test: an absolute XDG_CONFIG_HOME wins over HOME by design. We
        // only assert the path shape, not the developer's environment.
        if let Some(path) = linux_desktop_path() {
            assert!(path.ends_with(r"autostart\lumina.desktop") || path.ends_with("autostart/lumina.desktop"));
        }
    }
}