use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

fn temporary_directory(label: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "lumina-cli-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).expect("create temporary directory");
    path
}

fn run(directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lumina"))
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("run the lumina binary")
}

fn assert_exit(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn version_and_help_are_available_without_a_config() {
    let directory = temporary_directory("help");
    let version = run(&directory, &["--version"]);
    assert_exit(&version, 0);
    assert!(String::from_utf8_lossy(&version.stdout).contains("Lumina"));

    let help = run(&directory, &["--help"]);
    assert_exit(&help, 0);
    let text = String::from_utf8_lossy(&help.stdout);
    for expected in [
        "--check-config",
        "--init-config",
        "--list-themes",
        "--export-theme",
        "--check-theme",
    ] {
        assert!(text.contains(expected), "help is missing {expected}");
    }
    std::fs::remove_dir_all(directory).ok();
}

#[test]
fn init_config_creates_a_valid_file_and_protects_existing_content() {
    let directory = temporary_directory("init");
    let config = directory.join("config.toml");

    let init = run(&directory, &["--init-config", "config.toml"]);
    assert_exit(&init, 0);
    let written = std::fs::read_to_string(&config).expect("config was written");
    assert!(written.contains("[appearance]"));

    let check = run(&directory, &["--check-config", "--config", "config.toml"]);
    assert_exit(&check, 0);
    assert!(String::from_utf8_lossy(&check.stdout).contains("Configuration is valid"));

    std::fs::write(&config, "version = 99\n").expect("write sentinel");
    let refused = run(&directory, &["--init-config", "config.toml"]);
    assert_exit(&refused, 1);
    assert_eq!(
        std::fs::read_to_string(&config).expect("read sentinel"),
        "version = 99\n"
    );

    let forced = run(&directory, &["--init-config", "config.toml", "--force"]);
    assert_exit(&forced, 0);
    assert_ne!(
        std::fs::read_to_string(&config).expect("read replaced config"),
        "version = 99\n"
    );
    std::fs::remove_dir_all(directory).ok();
}

#[test]
fn print_config_emits_the_annotated_defaults() {
    let directory = temporary_directory("print");
    let output = run(&directory, &["--print-config"]);
    assert_exit(&output, 0);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("[general]"));
    assert!(text.contains("[accessibility]"));
    assert!(text.contains("version ="));
    std::fs::remove_dir_all(directory).ok();
}

#[test]
fn themes_can_be_listed_exported_and_validated() {
    let directory = temporary_directory("themes");
    let config = directory.join("config.toml");
    std::fs::write(&config, "").expect("write empty config");

    let list = run(&directory, &["--list-themes", "--config", "config.toml"]);
    assert_exit(&list, 0);
    let listed = String::from_utf8_lossy(&list.stdout);
    for name in ["default", "midnight", "neon", "high-contrast"] {
        assert!(listed.contains(name), "theme list is missing {name}");
    }

    let export = run(
        &directory,
        &[
            "--export-theme",
            "neon",
            "--output",
            "neon.toml",
            "--config",
            "config.toml",
        ],
    );
    assert_exit(&export, 0);
    let exported = std::fs::read_to_string(directory.join("neon.toml")).expect("exported theme");
    assert!(exported.contains("name = \"Neon\""));
    assert!(exported.contains("[appearance]"));

    let check = run(
        &directory,
        &["--check-theme", "neon.toml", "--config", "config.toml"],
    );
    assert_exit(&check, 0);
    assert!(String::from_utf8_lossy(&check.stdout).contains("Configuration is valid"));

    std::fs::write(directory.join("broken.toml"), "name = [\n").expect("write broken theme");
    let broken = run(
        &directory,
        &["--check-theme", "broken.toml", "--config", "config.toml"],
    );
    assert_exit(&broken, 1);
    assert!(String::from_utf8_lossy(&broken.stderr).contains("not a valid theme"));
    std::fs::remove_dir_all(directory).ok();
}

#[test]
fn malformed_command_lines_exit_two_without_starting_the_runtime() {
    let directory = temporary_directory("bad-args");
    let unknown = run(&directory, &["--not-a-real-flag"]);
    assert_exit(&unknown, 2);
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown argument"));

    let missing_config = run(&directory, &["--config"]);
    assert_exit(&missing_config, 2);
    assert!(String::from_utf8_lossy(&missing_config.stderr).contains("needs a file path"));

    let missing_theme = run(&directory, &["--check-theme"]);
    assert_exit(&missing_theme, 2);
    assert!(String::from_utf8_lossy(&missing_theme.stderr).contains("needs a file path"));

    let missing_export = run(&directory, &["--export-theme"]);
    assert_exit(&missing_export, 2);
    assert!(String::from_utf8_lossy(&missing_export.stderr).contains("needs a value"));
    std::fs::remove_dir_all(directory).ok();
}
