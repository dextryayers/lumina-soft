//! Lumina binary entry point.
//!
//! # CLI surface
//!
//! | Flag | Purpose |
//! |---|---|
//! | `--version` | Print version, commit, and target triple |
//! | `--check-config` | Load, validate, and report on the configuration file |
//! | `--diagnostics` | Print the full environment dump for bug reports |
//! | `--autostart [install\|remove\|status]` | Manage the per-user autostart entry |
//! | `--activate` | Ask a running instance to show itself |
//! | `--hidden` | Start with the island hidden (used by autostart) |
//! | *(none)* | Start the island (Phases 6+) |
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! |---|---|
//! | 0 | Success, or the config is valid |
//! | 1 | The config has problems (`--check-config`) |
//! | 2 | Bad arguments |

use std::path::PathBuf;
use std::process::ExitCode;

use lumina_config::validate::{validate, ValidationReport};
use lumina_config::Config;
use lumina_platform::capability::{Capabilities, Platform};
use lumina_platform::{autostart, single_instance};

/// Print version information.
fn print_version() {
    println!("Lumina {}", env!("CARGO_PKG_VERSION"));
    println!("  core     {}", lumina_core::VERSION);
    println!("  config   {}", lumina_config::VERSION);
    println!("  platform {}", lumina_platform::VERSION);
    println!("  render   {}", lumina_render::VERSION);
    println!("  text     {}", lumina_text::VERSION);
    println!("  target   {}", option_env!("TARGET").unwrap_or("unknown"));
}

/// Print the full environment dump.
///
/// Designed to be pasted verbatim into a bug report. Every field a maintainer
/// needs to reproduce a problem is here: OS, display server, GPU, config path,
/// and the validation state.
fn print_diagnostics() {
    let caps = Capabilities::for_platform(Platform::detect());

    println!("Lumina {}", env!("CARGO_PKG_VERSION"));
    println!(
        "  target       {}",
        option_env!("TARGET").unwrap_or("unknown")
    );
    println!("  platform     {}", caps.platform.display_name());
    println!("  window       {}", caps.platform.has_window());
    println!("  always_on_top {}", caps.always_on_top);
    println!("  per_pixel_alpha {}", caps.per_pixel_alpha);
    println!("  click_through {}", caps.click_through);
    println!("  global_hotkeys {}", caps.global_hotkeys);
    println!("  multi_monitor {}", caps.multi_monitor);

    let config_path = config_path();
    println!("  config_path  {}", config_path.display());

    match load_config(&config_path) {
        Ok((config, report)) => {
            println!("  config       loaded");
            println!("  valid        {}", report.is_clean());
            if !report.is_clean() {
                println!("  problems     {}", report.len());
            }
            let _ = config;
        }
        Err(e) => {
            println!("  config       FAILED: {e}");
        }
    }

    for limitation in caps.limitations() {
        println!("  limitation   {limitation}");
    }
}

/// Resolve the configuration file path.
fn config_path() -> PathBuf {
    if let Ok(path) = std::env::var("LUMINA_CONFIG") {
        return PathBuf::from(path);
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_default()
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map_or_else(
            || {
                let home = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default();
                home.join(".config")
            },
            PathBuf::from,
        )
    };
    base.join("lumina").join("config.toml")
}

/// Load and validate the configuration.
fn load_config(path: &std::path::Path) -> Result<(Config, ValidationReport), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let parsed = Config::from_toml_str(&text)
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    Ok(validate(parsed))
}

/// Run `--check-config`.
fn run_check_config() -> ExitCode {
    let path = config_path();
    println!("Checking {}", path.display());

    match load_config(&path) {
        Ok((config, report)) => {
            println!("{}", report.render());
            if report.is_clean() {
                let _ = config;
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Run `--autostart`.
fn run_autostart(action: &str, hidden: bool) -> ExitCode {
    match action {
        "install" => {
            let executable = match std::env::current_exe() {
                Ok(path) => path,
                Err(error) => {
                    eprintln!("error: cannot locate the Lumina executable: {error}");
                    return ExitCode::FAILURE;
                }
            };
            match autostart::install(&executable, hidden) {
                Ok(()) => {
                    println!("Autostart installed for {}", executable.display());
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        "remove" | "uninstall" => match autostart::uninstall() {
            Ok(()) => {
                println!("Autostart removed.");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        },
        "status" => {
            if autostart::is_enabled() {
                println!("Autostart is enabled.");
            } else {
                println!("Autostart is disabled.");
            }
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("error: unknown autostart action {other:?}");
            eprintln!("Try 'lumina --autostart status|install|remove'.");
            ExitCode::from(2)
        }
    }
}

/// Parse arguments and dispatch.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        print_version();
        return ExitCode::SUCCESS;
    }

    if args.iter().any(|a| a == "--diagnostics" || a == "-D") {
        print_diagnostics();
        return ExitCode::SUCCESS;
    }

    if args.iter().any(|a| a == "--check-config" || a == "-C") {
        return run_check_config();
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Lumina {}", env!("CARGO_PKG_VERSION"));
        println!();
        println!("Usage: lumina [OPTIONS]");
        println!();
        println!("Options:");
        println!("  -V, --version        Print version information");
        println!("  -D, --diagnostics    Print the environment dump");
        println!("  -C, --check-config   Validate the configuration file");
        println!("      --autostart [ACTION]");
        println!("                       Manage autostart: status, install, remove");
        println!("      --activate       Ask a running instance to show itself");
        println!("      --hidden         Start hidden (used by autostart)");
        println!("  -h, --help           Print this help");
        return ExitCode::SUCCESS;
    }

    let hidden = args.iter().any(|a| a == "--hidden");

    if let Some(index) = args.iter().position(|a| a == "--autostart") {
        let action = args.get(index + 1).map_or("status", String::as_str);
        return run_autostart(action, hidden);
    }
    if let Some(value) = args.iter().find_map(|a| a.strip_prefix("--autostart=")) {
        return run_autostart(value, hidden);
    }

    if args.iter().any(|a| a == "--activate") {
        if single_instance::notify_existing("lumina", "show") {
            println!("Asked the running Lumina instance to show itself.");
            ExitCode::SUCCESS
        } else {
            eprintln!("error: no running Lumina instance was found.");
            ExitCode::FAILURE
        }
    } else if !args.is_empty() {
        eprintln!("error: unknown argument '{}'", args[0]);
        eprintln!("Try 'lumina --help' for more information.");
        ExitCode::from(2)
    } else if let Ok(_guard) = single_instance::claim("lumina") {
        // The island itself arrives in Phase 6. Until then, print the
        // environment so a user can confirm the binary runs and see what
        // the platform supports.
        print_diagnostics();
        println!();
        println!("Lumina is not yet able to create its window. See PLAN.md Phase 6.");
        ExitCode::SUCCESS
    } else {
        let _ = single_instance::notify_existing("lumina", "show");
        println!("Lumina is already running; asked it to show itself.");
        ExitCode::SUCCESS
    }
}
