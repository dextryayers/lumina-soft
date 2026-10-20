use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use lumina_ipc::events::EventKind;
use lumina_ipc::protocol::{Command, IpcErrorCode};

fn docs() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/ipc.md");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn section<'a>(docs: &'a str, heading: &str) -> &'a str {
    let start = docs
        .find(heading)
        .unwrap_or_else(|| panic!("missing heading {heading}"));
    let rest = &docs[start + heading.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    &rest[..end]
}

fn table_names(section: &str) -> BTreeSet<String> {
    section
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("| `")?;
            let (name, _) = rest.split_once('`')?;
            Some(name.to_string())
        })
        .collect()
}

#[test]
fn every_command_appears_exactly_once_in_the_command_table() {
    let docs = docs();
    let documented = table_names(section(&docs, "## Commands"));
    let implemented: BTreeSet<String> = Command::ALL_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    assert_eq!(
        documented, implemented,
        "docs/ipc.md command table has drifted from Command::ALL_NAMES"
    );
}

#[test]
fn every_error_code_is_documented() {
    let docs = docs();
    let documented = table_names(section(&docs, "## Error codes"));
    let implemented: BTreeSet<String> = IpcErrorCode::ALL
        .iter()
        .map(|code| code.as_str().to_string())
        .collect();
    assert_eq!(
        documented, implemented,
        "docs/ipc.md error table has drifted from IpcErrorCode::ALL"
    );
}

#[test]
fn every_event_is_documented() {
    let docs = docs();
    let documented = table_names(section(&docs, "## Events"));
    let implemented: BTreeSet<String> = EventKind::ALL
        .iter()
        .map(|kind| kind.as_str().to_string())
        .collect();
    assert_eq!(
        documented, implemented,
        "docs/ipc.md event table has drifted from EventKind::ALL"
    );
}

#[test]
fn protocol_limits_match_the_documented_contract() {
    let docs = docs();
    for fragment in [
        "64 KiB",
        "120 commands per second",
        "8 concurrent connections",
        "one local control surface",
    ] {
        assert!(
            docs.contains(fragment),
            "missing documented limit: {fragment}"
        );
    }
}
