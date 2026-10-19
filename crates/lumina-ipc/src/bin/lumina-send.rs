//! `lumina-send`: a dependency-free command line client for the Lumina IPC
//! socket.
//!
//! ```text
//! lumina-send notify --title "Build done" --body "45 seconds" --duration-ms 4000
//! lumina-send status --json
//! lumina-send set_rgb --mode wave --speed 1.5
//! ```

use std::process::ExitCode;

use lumina_ipc::{Client, ClientError, Command, IpcAddress};
use serde_json::{Map, Number, Value};

const COMMANDS: [&str; 19] = Command::ALL_NAMES;

const NUMERIC_KEYS: [&str; 9] = [
    "duration_ms",
    "speed",
    "position_ms",
    "volume",
    "timeout_ms",
    "top_half_width",
    "bottom_half_width",
    "height",
    "corner_radius",
];

const BOOLEAN_KEYS: [&str; 5] = ["pin", "peak", "enabled", "persist", "quiet"];

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match parse(&arguments) {
        Ok(Action::Help) => {
            print_help();
            ExitCode::SUCCESS
        }
        Ok(Action::Completions(shell)) => {
            if let Some(script) = completions(&shell) {
                print!("{script}");
            } else {
                eprintln!("lumina-send: unknown shell `{shell}` (use bash, zsh, or fish)");
                return ExitCode::from(3);
            }
            ExitCode::SUCCESS
        }
        Ok(Action::Send(options)) => send(&options),
        Err(message) => {
            eprintln!("lumina-send: {message}");
            eprintln!("Try `lumina-send --help`.");
            ExitCode::from(3)
        }
    }
}

#[derive(Debug)]
enum Action {
    Help,
    Completions(String),
    Send(SendOptions),
}

#[derive(Debug)]
struct SendOptions {
    command: String,
    payload: Map<String, Value>,
    address: Option<IpcAddress>,
    compact: bool,
    quiet: bool,
}

fn parse(arguments: &[String]) -> Result<Action, String> {
    if arguments.is_empty() {
        return Err("missing command".to_string());
    }
    let mut index = 0;
    let mut address = None;
    let mut compact = false;
    let mut quiet = false;
    let mut command = None;
    let mut payload = Map::new();

    while index < arguments.len() {
        let argument = &arguments[index];
        if argument == "--help" || argument == "-h" {
            return Ok(Action::Help);
        }
        if argument == "--compact" || argument == "--json" {
            compact = true;
            index += 1;
            continue;
        }
        if argument == "--pretty" {
            compact = false;
            index += 1;
            continue;
        }
        if argument == "--quiet" || argument == "-q" {
            quiet = true;
            index += 1;
            continue;
        }
        if argument == "--completions" {
            let shell = arguments
                .get(index + 1)
                .ok_or_else(|| "--completions needs a shell name".to_string())?;
            return Ok(Action::Completions(shell.clone()));
        }
        if argument == "--socket" {
            let path = arguments
                .get(index + 1)
                .ok_or_else(|| "--socket needs a path".to_string())?;
            #[cfg(unix)]
            {
                address = Some(IpcAddress::unix(path));
            }
            #[cfg(not(unix))]
            {
                let _ = path;
                return Err("--socket is only available on Unix".to_string());
            }
            index += 2;
            continue;
        }
        if argument == "--pipe" {
            let name = arguments
                .get(index + 1)
                .ok_or_else(|| "--pipe needs a name".to_string())?;
            #[cfg(windows)]
            {
                let full = if name.starts_with(r"\\.\pipe\") {
                    name.clone()
                } else {
                    format!(r"\\.\pipe\{name}")
                };
                address = Some(IpcAddress::pipe(full));
            }
            #[cfg(not(windows))]
            {
                let _ = name;
                return Err("--pipe is only available on Windows".to_string());
            }
            index += 2;
            continue;
        }
        if let Some(json) = argument.strip_prefix("--payload=") {
            merge_payload(&mut payload, json)?;
            index += 1;
            continue;
        }
        if argument == "--payload" {
            let json = arguments
                .get(index + 1)
                .ok_or_else(|| "--payload needs a JSON object".to_string())?;
            merge_payload(&mut payload, json)?;
            index += 2;
            continue;
        }
        if let Some(rest) = argument.strip_prefix("--") {
            let (key, inline) = match rest.split_once('=') {
                Some((key, value)) => (key, Some(value.to_string())),
                None => (rest, None),
            };
            if key.is_empty() {
                return Err("empty option name".to_string());
            }
            let key = key.replace('-', "_");
            let value = if let Some(value) = inline {
                value
            } else {
                if key_starts_with_no(&key) && !BOOLEAN_KEYS.contains(&key.as_str()) {
                    payload.insert(boolean_key(&key), Value::Bool(false));
                    index += 1;
                    continue;
                }
                if BOOLEAN_KEYS.contains(&key.as_str())
                    && arguments
                        .get(index + 1)
                        .is_none_or(|next| next.starts_with("--"))
                {
                    payload.insert(key, Value::Bool(true));
                    index += 1;
                    continue;
                }
                let value = arguments
                    .get(index + 1)
                    .ok_or_else(|| format!("--{key} needs a value"))?;
                index += 1;
                value.clone()
            };
            insert_payload(&mut payload, &key, &value)?;
            index += 1;
            continue;
        }
        if command.is_none() {
            if !COMMANDS.contains(&argument.as_str()) {
                return Err(format!("unknown command `{argument}`"));
            }
            command = Some(argument.clone());
            index += 1;
            continue;
        }
        return Err(format!("unexpected argument `{argument}`"));
    }

    let command = command.ok_or_else(|| "missing command".to_string())?;
    if quiet {
        compact = true;
    }
    Ok(Action::Send(SendOptions {
        command,
        payload,
        address,
        compact,
        quiet,
    }))
}

fn key_starts_with_no(key: &str) -> bool {
    key.starts_with("no_")
}

fn boolean_key(key: &str) -> String {
    key.trim_start_matches("no_").to_string()
}

fn merge_payload(payload: &mut Map<String, Value>, json: &str) -> Result<(), String> {
    let value: Value =
        serde_json::from_str(json).map_err(|error| format!("invalid --payload JSON: {error}"))?;
    let Value::Object(object) = value else {
        return Err("--payload must be a JSON object".to_string());
    };
    payload.extend(object);
    Ok(())
}

fn insert_payload(
    payload: &mut Map<String, Value>,
    key: &str,
    value: &str,
) -> Result<(), String> {
    if key == "arg" {
        let entry = payload
            .entry("args".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(values) = entry {
            values.push(Value::String(value.to_string()));
        }
        return Ok(());
    }
    let value = if value.starts_with('{') || value.starts_with('[') {
        serde_json::from_str(value).map_err(|error| format!("invalid JSON for --{key}: {error}"))?
    } else if key == "value" {
        parse_literal(value)
    } else if BOOLEAN_KEYS.contains(&key) {
        match value {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            other => return Err(format!("--{key} expects true or false, got `{other}`")),
        }
    } else if NUMERIC_KEYS.contains(&key) {
        parse_number(value).ok_or_else(|| format!("--{key} expects a number, got `{value}`"))?
    } else {
        Value::String(value.to_string())
    };
    payload.insert(key.to_string(), value);
    Ok(())
}

fn parse_number(value: &str) -> Option<Value> {
    if let Ok(integer) = value.parse::<i64>() {
        return Some(Value::Number(Number::from(integer)));
    }
    let float = value.parse::<f64>().ok()?;
    if !float.is_finite() {
        return None;
    }
    Number::from_f64(float).map(Value::Number)
}

fn parse_literal(value: &str) -> Value {
    match value {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        _ => parse_number(value).unwrap_or_else(|| Value::String(value.to_string())),
    }
}

fn send(options: &SendOptions) -> ExitCode {
    let address = match &options.address {
        Some(address) => address.clone(),
        None => match IpcAddress::default_for_user() {
            Ok(address) => address,
            Err(error) => {
                eprintln!("lumina-send: could not determine the socket address: {error}");
                return ExitCode::from(1);
            }
        },
    };
    let command = match Command::from_payload(&options.command, &Value::Object(options.payload.clone())) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("lumina-send: {error}");
            return ExitCode::from(3);
        }
    };
    let mut client = match Client::connect(&address) {
        Ok(client) => client,
        Err(error) => {
            eprintln!(
                "lumina-send: cannot connect to {address}: {error}. Is Lumina running?"
            );
            return ExitCode::from(1);
        }
    };
    match client.call(&command) {
        Ok(data) => {
            if !options.quiet {
                print_data(&data, options.compact);
            }
            ExitCode::SUCCESS
        }
        Err(ClientError::Remote(error)) => {
            eprintln!("lumina-send: {}", error.code);
            eprintln!("{}", error.message);
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("lumina-send: {error}");
            ExitCode::from(1)
        }
    }
}

fn print_data(data: &Value, compact: bool) {
    if data.is_null() {
        println!("ok");
        return;
    }
    if let Value::String(text) = data {
        println!("{text}");
        return;
    }
    let rendered = if compact {
        serde_json::to_string(data)
    } else {
        serde_json::to_string_pretty(data)
    };
    match rendered {
        Ok(text) => println!("{text}"),
        Err(error) => eprintln!("lumina-send: could not render the response: {error}"),
    }
}

fn print_help() {
    print!("{}", help_text());
}

fn help_text() -> String {
    format!(
        "\
lumina-send {version}
Send one command to a running Lumina instance.

USAGE:
    lumina-send [OPTIONS] <COMMAND> [--FIELD VALUE]... [--flag]

OPTIONS:
    -h, --help              Show this help
    -q, --quiet             Do not print the result
        --compact, --json   Print compact JSON instead of pretty JSON
        --pretty            Print pretty JSON (the default)
        --socket <PATH>     Unix socket path (default: $XDG_RUNTIME_DIR/lumina.sock)
        --pipe <NAME>       Windows pipe name (default: per-user pipe)
        --payload <JSON>    Merge a JSON object into the payload
        --completions <S>   Print bash, zsh, or fish completions

COMMANDS:
    {commands}

EXAMPLES:
    lumina-send status
    lumina-send notify --title \"Build done\" --body \"45 seconds\"
    lumina-send show --widget media --duration-ms 5000 --pin
    lumina-send set_rgb --mode reactive --color \"#ff8800\"
    lumina-send media --action play_pause
    lumina-send set_config --path general.start_hidden --value true --persist
    lumina-send execute --command git --arg status --timeout-ms 5000

Field values are strings unless the field is numeric or boolean:
    --duration-ms 4000   becomes 4000
    --pin                becomes true
    --no-pin             becomes false
    --arg value          appends to the args array (for execute)

EXIT CODES:
    0  success
    1  connection or protocol failure
    2  the command was rejected (an error code is printed)
    3  invalid command line
",
        version = lumina_ipc::VERSION,
        commands = COMMANDS.join(", "),
    )
}

fn completions(shell: &str) -> Option<&'static str> {
    match shell {
        "bash" => Some(BASH),
        "zsh" => Some(ZSH),
        "fish" => Some(FISH),
        _ => None,
    }
}

const BASH: &str = r#"_lumina_send() {
    local cur prev
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    if [[ $COMP_CWORD -eq 1 ]]; then
        COMPREPLY=( $(compgen -W "status show hide collapse notify set_rgb cycle_rgb set_profile get_config set_config reload_config media player_list shortcut execute spectrum geometry debug_overlay quit --socket --pipe --payload --compact --pretty --quiet --help --completions" -- "$cur") )
    elif [[ "$prev" == "--completions" ]]; then
        COMPREPLY=( $(compgen -W "bash zsh fish" -- "$cur") )
    fi
}
complete -F _lumina_send lumina-send
"#;

const ZSH: &str = r#"#compdef lumina-send
_lumina_send() {
    local -a commands
    commands=(
        'status:Full state dump'
        'show:Bring the island into a visible state'
        'hide:Hide the island'
        'collapse:Return to collapsed immediately'
        'notify:Fire the notification widget'
        'set_rgb:Override the RGB engine'
        'cycle_rgb:Advance to the next RGB engine'
        'set_profile:Switch profiles'
        'get_config:Read the effective configuration'
        'set_config:Change a configuration value'
        'reload_config:Re-read the configuration'
        'media:Control media playback'
        'player_list:List known media players'
        'shortcut:Run a configured shortcut'
        'execute:Run a program'
        'spectrum:Read current spectrum bands'
        'geometry:Edit the island shape live'
        'debug_overlay:Toggle the debug overlay'
        'quit:Graceful shutdown'
    )
    _arguments -s \
        '(-h --help)'{-h,--help}'[show help]' \
        '(-q --quiet)'{-q,--quiet}'[do not print the result]' \
        '--compact[compact JSON]' \
        '--pretty[pretty JSON]' \
        '--socket[Unix socket path]:path:_files' \
        '--pipe[Windows pipe name]:name:' \
        '--payload[JSON payload]:json:' \
        '--completions[print completions]:shell:(bash zsh fish)' \
        '1:command:->command' \
        '*::argument:->argument'
    case $state in
        command) _describe 'command' commands ;;
    esac
}
_lumina_send "$@"
"#;

const FISH: &str = r"complete -c lumina-send -f
complete -c lumina-send -n '__fish_use_subcommand' -a status -d 'Full state dump'
complete -c lumina-send -n '__fish_use_subcommand' -a show -d 'Bring the island into a visible state'
complete -c lumina-send -n '__fish_use_subcommand' -a hide -d 'Hide the island'
complete -c lumina-send -n '__fish_use_subcommand' -a collapse -d 'Return to collapsed immediately'
complete -c lumina-send -n '__fish_use_subcommand' -a notify -d 'Fire the notification widget'
complete -c lumina-send -n '__fish_use_subcommand' -a set_rgb -d 'Override the RGB engine'
complete -c lumina-send -n '__fish_use_subcommand' -a cycle_rgb -d 'Advance the RGB engine'
complete -c lumina-send -n '__fish_use_subcommand' -a set_profile -d 'Switch profiles'
complete -c lumina-send -n '__fish_use_subcommand' -a get_config -d 'Read the configuration'
complete -c lumina-send -n '__fish_use_subcommand' -a set_config -d 'Change a configuration value'
complete -c lumina-send -n '__fish_use_subcommand' -a reload_config -d 'Re-read the configuration'
complete -c lumina-send -n '__fish_use_subcommand' -a media -d 'Control media playback'
complete -c lumina-send -n '__fish_use_subcommand' -a player_list -d 'List media players'
complete -c lumina-send -n '__fish_use_subcommand' -a shortcut -d 'Run a shortcut'
complete -c lumina-send -n '__fish_use_subcommand' -a execute -d 'Run a program'
complete -c lumina-send -n '__fish_use_subcommand' -a spectrum -d 'Read spectrum bands'
complete -c lumina-send -n '__fish_use_subcommand' -a geometry -d 'Edit the island shape'
complete -c lumina-send -n '__fish_use_subcommand' -a debug_overlay -d 'Toggle the debug overlay'
complete -c lumina-send -n '__fish_use_subcommand' -a quit -d 'Graceful shutdown'
complete -c lumina-send -s h -l help -d 'Show help'
complete -c lumina-send -s q -l quiet -d 'Do not print the result'
complete -c lumina-send -l compact -d 'Compact JSON'
complete -c lumina-send -l pretty -d 'Pretty JSON'
complete -c lumina-send -l socket -r -d 'Unix socket path'
complete -c lumina-send -l pipe -r -d 'Windows pipe name'
complete -c lumina-send -l payload -r -d 'JSON payload'
complete -c lumina-send -l completions -r -a 'bash zsh fish' -d 'Print completions'
";

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_string).collect()
    }

    fn payload_of(text: &str) -> Map<String, Value> {
        match parse(&args(text)).unwrap() {
            Action::Send(options) => options.payload,
            other => panic!("expected a send action, got {other:?}"),
        }
    }

    #[test]
    fn a_command_without_fields_parses() {
        let payload = payload_of("status");
        assert!(payload.is_empty());
    }

    #[test]
    fn numeric_fields_become_numbers() {
        let payload = payload_of("show --duration-ms 4000 --pin");
        assert_eq!(payload["duration_ms"], 4000);
        assert_eq!(payload["pin"], true);
    }

    #[test]
    fn dashes_become_underscores() {
        let payload = payload_of("geometry --top-half-width 220 --height 62");
        assert_eq!(payload["top_half_width"], 220);
        assert_eq!(payload["height"], 62);
    }

    #[test]
    fn no_prefix_negates_booleans() {
        let payload = payload_of("show --no-pin");
        assert_eq!(payload["pin"], false);
    }

    #[test]
    fn repeated_arg_flags_build_the_args_array() {
        let payload = payload_of("execute --command git --arg status --arg --short");
        assert_eq!(payload["command"], "git");
        assert_eq!(payload["args"], serde_json::json!(["status", "--short"]));
    }

    #[test]
    fn a_json_payload_merges_objects() {
        let payload = payload_of(r#"notify --payload {"title":"hi","body":"there"}"#);
        assert_eq!(payload["title"], "hi");
        assert_eq!(payload["body"], "there");
    }

    #[test]
    fn unknown_commands_are_rejected_early() {
        let error = parse(&args("dance")).unwrap_err();
        assert!(error.contains("unknown command"));
    }

    #[test]
    fn a_numeric_field_rejects_text() {
        let error = parse(&args("show --duration-ms soon")).unwrap_err();
        assert!(error.contains("expects a number"));
    }

    #[test]
    fn completions_cover_the_three_shells() {
        assert!(completions("bash").unwrap().contains("complete -F"));
        assert!(completions("zsh").unwrap().contains("#compdef"));
        assert!(completions("fish").unwrap().contains("complete -c"));
        assert!(completions("powershell").is_none());
    }

    #[test]
    fn help_mentions_every_command() {
        let help = help_text();
        for command in COMMANDS {
            assert!(help.contains(command), "help is missing {command}");
        }
    }

    #[test]
    fn set_config_values_infer_json_literals() {
        let payload = payload_of("set_config --path general.start_hidden --value true");
        assert_eq!(payload["value"], true);
        let payload = payload_of(r#"set_config --path widgets.order --value ["clock","media"]"#);
        assert_eq!(payload["value"], serde_json::json!(["clock", "media"]));
        let payload = payload_of("set_config --path general.profile --value work");
        assert_eq!(payload["value"], "work");
    }
}
