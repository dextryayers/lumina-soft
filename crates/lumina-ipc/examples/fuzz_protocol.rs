//! Deterministic mutation fuzzer for request parsing, framing, and round trips.
//!
//! Run it with an optional duration in seconds:
//!
//! ```text
//! cargo run -p lumina-ipc --example fuzz_protocol -- 60
//! ```
//!
//! The generator is seeded, so a failure can be reproduced by keeping the seed
//! printed on startup. A parsed request must serialize to a line that parses
//! back to the same value; framing must reject overlong lines instead of
//! growing without bound.

use std::io::Cursor;
use std::time::{Duration, Instant};

use lumina_ipc::framing::{LineReader, MAX_LINE_BYTES};
use lumina_ipc::Request;

const SEEDS: &[&str] = &[
    r#"{"id":1,"command":"status"}"#,
    r#"{"id":2,"command":"show","params":{"widget":"media","duration_ms":5000,"pin":true}}"#,
    r#"{"id":3,"command":"hide"}"#,
    r#"{"id":4,"command":"collapse"}"#,
    r##"{"id":5,"command":"notify","params":{"title":"Build done","body":"all green","accent":"#34c759","duration_ms":5000}}"##,
    r##"{"id":6,"command":"set_rgb","params":{"mode":"reactive","color":"#ff9500","speed":1.4}}"##,
    r#"{"id":7,"command":"cycle_rgb"}"#,
    r#"{"id":8,"command":"set_profile","params":{"name":"work"}}"#,
    r#"{"id":9,"command":"get_config","params":{"section":"widgets.clock"}}"#,
    r#"{"id":10,"command":"set_config","params":{"path":"widgets.clock.show_seconds","value":true}}"#,
    r#"{"id":11,"command":"reload_config"}"#,
    r#"{"id":12,"command":"media","params":{"action":"seek","player":"spotify","position_ms":42000}}"#,
    r#"{"id":13,"command":"player_list"}"#,
    r#"{"id":"shortcut-1","command":"shortcut","params":{"id":"toggle_visibility"}}"#,
    r#"{"id":15,"command":"execute","params":{"command":"echo","args":["hello"],"timeout_ms":1000}}"#,
    r#"{"id":16,"command":"spectrum","params":{"peak":true}}"#,
    r#"{"id":17,"command":"geometry","params":{"top_half_width":150,"bottom_half_width":170,"height":44,"corner_radius":18}}"#,
    r#"{"id":18,"command":"debug_overlay","params":{"enabled":false}}"#,
    r#"{"id":19,"command":"quit"}"#,
];

const ALPHABET: &[u8] = b"{}[]\":,0123456789abcdefghijklmnopqrstuvwxyz_-. \\\r\n\t/unexpected";

fn next(state: &mut u64) -> u64 {
    let mut value = *state;
    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;
    *state = value;
    value
}

fn mutate(bytes: &mut Vec<u8>, state: &mut u64) {
    let edits = 1 + (next(state) % 5) as usize;
    for _ in 0..edits {
        if bytes.is_empty() {
            bytes.push(ALPHABET[(next(state) as usize) % ALPHABET.len()]);
            continue;
        }
        match next(state) % 5 {
            0 => {
                let index = (next(state) as usize) % bytes.len();
                bytes[index] ^= 1 << (next(state) % 8);
            }
            1 => {
                let index = (next(state) as usize) % bytes.len();
                let length = 1 + (next(state) % 4) as usize;
                for offset in 0..length {
                    bytes.insert(
                        index + offset,
                        ALPHABET[(next(state) as usize) % ALPHABET.len()],
                    );
                }
            }
            2 => {
                let index = (next(state) as usize) % bytes.len();
                let length = (1 + (next(state) % 4) as usize).min(bytes.len() - index);
                bytes.drain(index..index + length);
            }
            3 => {
                let index = (next(state) as usize) % bytes.len();
                let length = (1 + (next(state) % 16) as usize).min(bytes.len() - index);
                let copy = bytes[index..index + length].to_vec();
                bytes.extend_from_slice(&copy);
            }
            _ => {
                let index = (next(state) as usize) % bytes.len();
                let byte = ALPHABET[(next(state) as usize) % ALPHABET.len()];
                bytes[index] = byte;
            }
        }
    }
}

fn fuzz_line(bytes: &[u8]) {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let Ok(request) = Request::parse(text) else {
        return;
    };
    let line = request
        .to_line()
        .expect("a validated request must serialize");
    let reparsed = Request::parse(&line).expect("a serialized request must parse");
    assert_eq!(request, reparsed);
}

fn fuzz_framing(bytes: &[u8]) {
    let chunk = 1 + (bytes.len() % 17);
    let reader = Cursor::new(bytes.to_vec());
    let mut lines = LineReader::new(reader);
    for _ in 0..=bytes.len() / chunk + 1 {
        match lines.read_line() {
            Ok(Some(line)) => assert!(
                line.len() <= MAX_LINE_BYTES,
                "framing returned an oversized line"
            ),
            Ok(None) | Err(_) => break,
        }
    }
}

fn main() {
    let seconds = std::env::args()
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(10);
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    println!("fuzz_protocol: seed {state:#x}, duration {seconds}s");
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut iterations = 0_u64;

    while Instant::now() < deadline {
        for _ in 0..1_000 {
            iterations += 1;
            let seed = SEEDS[(next(&mut state) as usize) % SEEDS.len()];
            let mut bytes = seed.as_bytes().to_vec();
            mutate(&mut bytes, &mut state);
            fuzz_line(&bytes);
            fuzz_framing(&bytes);
        }
    }

    println!("fuzz_protocol: {iterations} cases, no panics or round-trip mismatches");
}
