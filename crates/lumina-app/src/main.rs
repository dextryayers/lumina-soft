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

use std::fmt::Write as _;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use lumina_audio::AudioAggregates;
use lumina_color::ColorEngine;
use lumina_config::validate::{validate, ValidationReport};
use lumina_config::{Config, RgbMode};
use lumina_platform::capability::{Capabilities, Platform};
use lumina_platform::{autostart, single_instance};
use lumina_render::render::{FrameIntent, RenderLoop};
use lumina_text::atlas::Atlas;
use lumina_text::layout::{Overflow, TextAlignment, TextLayoutCache};
use lumina_text::shaping::Shaper;

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
    println!(
        "  profile      {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
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

    if config_path.exists() {
        match load_config(&config_path) {
            Ok((config, report)) => {
                println!("  config       loaded");
                println!("  valid        {}", report.is_clean());
                if !report.is_clean() {
                    println!("  problems     {}", report.len());
                }
                println!("  rgb_mode     {:?}", config.rgb.mode);
                println!("  reduced      {}", config.motion.reduced);
                println!("  contrast     {}", config.accessibility.enforce_contrast);
                println!("  screen_reader {:?}", config.accessibility.screen_reader);
            }
            Err(e) => {
                println!("  config       FAILED: {e}");
            }
        }
    } else {
        println!("  config       absent (compiled defaults apply)");
    }

    println!("  os_reduced   {}", lumina_platform::os_reduced_motion());
    println!(
        "  os_reader    {}",
        lumina_platform::os_screen_reader_active()
    );

    for limitation in caps.limitations() {
        println!("  limitation   {limitation}");
    }
}

/// Small harness for `--self-test`.
///
/// The output is intentionally plain text: CI captures it, and a user can paste
/// it into a bug report without stripping colour codes.
#[derive(Debug, Default)]
struct SelfTest {
    passed: usize,
    failed: usize,
}

impl SelfTest {
    fn check(&mut self, name: &str, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.passed += 1;
                println!("PASS  {name}");
            }
            Err(reason) => {
                self.failed += 1;
                println!("FAIL  {name}: {reason}");
            }
        }
    }

    fn succeed(&mut self) -> bool {
        self.failed == 0
    }
}

/// Run environment and subsystem checks without a window or GPU device.
///
/// These checks are deliberately the ones that can run on a headless CI
/// machine. Anything that needs a compositor belongs in an integration test
/// with a display server, not here.
fn run_self_test() -> ExitCode {
    let mut test = SelfTest::default();

    test.check("version metadata", version_metadata());
    test.check("default configuration validates", defaults_validate());
    test.check(
        "shipped configuration matches defaults",
        shipped_config_matches(),
    );
    test.check("built-in themes validate", themes_validate());
    test.check("all RGB engines resolve", rgb_engines_resolve());
    test.check("WCAG text contrast", contrast_is_enforced());
    test.check("island shader validates", shader_validates());
    test.check("text shaping", text_shaping_works());
    test.check("idle render loop", idle_loop_does_not_render());
    test.check("text layout cache", layout_cache_reuses_results());
    test.check("accessibility probes", accessibility_probes());
    test.check(
        "platform capability matrix",
        platform_matrix_is_consistent(),
    );

    println!();
    println!("self-test: {} passed, {} failed", test.passed, test.failed);
    if test.succeed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn version_metadata() -> Result<(), String> {
    if env!("CARGO_PKG_VERSION").is_empty() {
        return Err("the package version is empty".to_string());
    }
    for (name, version) in [
        ("core", lumina_core::VERSION),
        ("config", lumina_config::VERSION),
        ("color", lumina_color::VERSION),
        ("platform", lumina_platform::VERSION),
        ("render", lumina_render::VERSION),
        ("text", lumina_text::VERSION),
    ] {
        if version.is_empty() {
            return Err(format!("{name} has an empty version"));
        }
    }
    Ok(())
}

fn defaults_validate() -> Result<(), String> {
    let (config, report) = validate(Config::default());
    if !report.is_clean() {
        return Err(format!(
            "the compiled defaults are not clean: {}",
            report.render()
        ));
    }
    if config != Config::default() {
        return Err("validation changed the compiled defaults".to_string());
    }
    Ok(())
}

fn shipped_config_matches() -> Result<(), String> {
    let text = lumina_config::defaults::default_toml();
    let parsed = Config::from_toml_str(&text)
        .map_err(|error| format!("the shipped default.toml does not parse: {error}"))?;
    if parsed != Config::default() {
        return Err("the shipped default.toml has drifted from the compiled defaults".to_string());
    }
    let (_, report) = validate(parsed);
    if !report.is_clean() {
        return Err(format!(
            "the shipped default.toml is not clean: {}",
            report.render()
        ));
    }
    Ok(())
}

fn themes_validate() -> Result<(), String> {
    let base = Config::default();
    for (name, theme) in lumina_config::theme::builtin_themes() {
        let mut store = lumina_config::theme::ThemeStore::empty();
        store.insert(name, theme);
        let applied = store.apply(&base, name);
        let (checked, report) = validate(applied.clone());
        if !report.is_clean() {
            return Err(format!("theme {name} is not clean: {}", report.render()));
        }
        if checked != applied {
            return Err(format!("theme {name} is changed by validation"));
        }
    }
    Ok(())
}

fn palette_is_finite(palette: &lumina_core::color::Palette) -> bool {
    let finite = |color: lumina_core::color::Srgb| {
        color.r.is_finite() && color.g.is_finite() && color.b.is_finite() && color.a.is_finite()
    };
    finite(palette.fill)
        && finite(palette.fill_accent)
        && finite(palette.border)
        && finite(palette.glow)
        && finite(palette.text)
        && finite(palette.text_dim)
        && palette.glow_intensity.is_finite()
}

fn rgb_engines_resolve() -> Result<(), String> {
    for mode in RgbMode::ALL {
        let mut config = Config::default();
        config.rgb.mode = mode;
        let mut engine = ColorEngine::new(&config);
        let palette = engine.update(
            1.0 / 60.0,
            AudioAggregates {
                bass: 0.4,
                mid: 0.3,
                treble: 0.2,
                loudness: 0.35,
            },
            Some(lumina_core::color::Srgb::from_rgba8(40, 90, 200, 255)),
        );
        if !palette_is_finite(&palette) {
            return Err(format!("{mode:?} produced a non-finite palette"));
        }
    }
    Ok(())
}

fn contrast_is_enforced() -> Result<(), String> {
    for fill in ["#000000", "#FFFFFF", "#777777", "#12121ACC"] {
        let mut config = Config::default();
        config.appearance.fill = fill.to_string();
        config.text.color = "#FFFFFFFF".to_string();
        config.text.dim_color = "#EEEEEE".to_string();
        config.accessibility.enforce_contrast = true;
        config.accessibility.contrast_target = 4.5;

        let engine = ColorEngine::new(&config);
        let palette = engine.palette();
        let text = palette.text_contrast();
        let dim = palette.dim_contrast();
        if text < 4.49 {
            return Err(format!("fill {fill}: text contrast is {text:.2}"));
        }
        if dim < 4.49 {
            return Err(format!("fill {fill}: dim contrast is {dim:.2}"));
        }
    }
    Ok(())
}

fn shader_validates() -> Result<(), String> {
    lumina_render::shaders::validate_wgsl(lumina_render::shaders::ISLAND_WGSL)
        .map_or(Ok(()), |error| {
            Err(format!("the island WGSL does not validate: {error}"))
        })
}

fn text_shaping_works() -> Result<(), String> {
    let atlas = Atlas::new();
    let shaped = Shaper::new().shape("Lumina", &atlas, 14.0);
    if shaped.is_empty() {
        return Err("shaping a non-empty string produced no glyphs".to_string());
    }
    if !shaped.width.is_finite() || shaped.width <= 0.0 {
        return Err(format!("shaped width is {}", shaped.width));
    }
    Ok(())
}

fn idle_loop_does_not_render() -> Result<(), String> {
    let mut loop_ = RenderLoop::new(60, true);
    let idle = || FrameIntent {
        dirty: false,
        animating: false,
        audio_active: false,
        visible: true,
    };
    for _ in 0..100 {
        if loop_.should_render(idle()) {
            return Err("an idle loop submitted a frame".to_string());
        }
    }
    if loop_.stats().rendered() != 0 || loop_.stats().skipped() != 100 {
        return Err("idle frame counters are wrong".to_string());
    }
    Ok(())
}

fn layout_cache_reuses_results() -> Result<(), String> {
    let atlas = Atlas::new();
    let shaper = Shaper::new();
    let mut cache = TextLayoutCache::new(8);
    let first = cache
        .layout(
            "Now playing",
            &shaper,
            &atlas,
            14.0,
            300.0,
            TextAlignment::Left,
            Overflow::Ellipsis,
        )
        .clone();
    let second = cache
        .layout(
            "Now playing",
            &shaper,
            &atlas,
            14.0,
            300.0,
            TextAlignment::Left,
            Overflow::Ellipsis,
        )
        .clone();
    if first != second {
        return Err("a cached layout differs from its first measurement".to_string());
    }
    let stats = cache.stats();
    if stats.hits != 1 || stats.misses != 1 {
        return Err(format!("expected one hit and one miss, got {stats:?}"));
    }
    Ok(())
}

fn accessibility_probes() -> Result<(), String> {
    let _ = lumina_platform::os_reduced_motion();
    let _ = lumina_platform::os_screen_reader_active();
    let mut gate =
        lumina_platform::AnnouncementGate::new(lumina_config::schema::ScreenReaderMode::On, 1_500);
    let now = Instant::now();
    if !gate.should_announce(now) {
        return Err("the first announcement was suppressed".to_string());
    }
    if gate.should_announce(now) {
        return Err("the rate limit did not suppress a duplicate".to_string());
    }
    Ok(())
}

fn platform_matrix_is_consistent() -> Result<(), String> {
    let detected = Platform::detect();
    let caps = Capabilities::for_platform(detected);
    if caps.platform != detected {
        return Err("the capability matrix reports another platform".to_string());
    }
    if detected.has_window() && !caps.always_on_top && detected != Platform::WaylandNoLayerShell {
        // A windowed platform that cannot stay on top is a real capability gap,
        // not a crash; report it as informational rather than a failure.
        println!(
            "INFO  {} reports no always-on-top support",
            detected.display_name()
        );
    }
    Ok(())
}

/// One benchmark result, before formatting.
#[derive(Debug, Clone, Copy)]
struct BenchResult {
    name: &'static str,
    ns_per_op: f64,
}

/// Measure a closure over a fixed number of iterations.
fn measure(iterations: u32, mut operation: impl FnMut()) -> f64 {
    let iterations = iterations.max(1);
    let start = Instant::now();
    for _ in 0..iterations {
        operation();
    }
    let elapsed = start.elapsed();
    elapsed.as_secs_f64() * 1.0e9 / f64::from(iterations)
}

/// Run microbenchmarks that do not need a GPU or a window.
///
/// These are CPU-side budgets: palette resolution, text shaping, layout, and
/// shader validation. Frame time on a real GPU is measured by the renderer's
/// own histogram once the orchestrator exists.
fn run_benchmark(output: Option<&Path>) -> ExitCode {
    let platform = Platform::detect();
    let config = Config::default();
    let audio = AudioAggregates {
        bass: 0.5,
        mid: 0.35,
        treble: 0.2,
        loudness: 0.4,
    };

    let mut static_engine = ColorEngine::new(&config);
    let static_ns = measure(120_000, || {
        let palette = static_engine.update(1.0 / 60.0, audio, None);
        black_box(palette);
    });

    let mut reactive_config = config.clone();
    reactive_config.rgb.mode = RgbMode::Reactive;
    let mut reactive_engine = ColorEngine::new(&reactive_config);
    let reactive_ns = measure(120_000, || {
        let palette = reactive_engine.update(1.0 / 60.0, audio, None);
        black_box(palette);
    });

    let atlas = Atlas::new();
    let shaper = Shaper::new();
    let text_ns = measure(20_000, || {
        let shaped = shaper.shape("Now playing: Lumen by Northbound", &atlas, 14.0);
        black_box(shaped);
    });

    let mut layout_cache = TextLayoutCache::new(64);
    layout_cache.layout(
        "Now playing: Lumen by Northbound",
        &shaper,
        &atlas,
        14.0,
        320.0,
        TextAlignment::Left,
        Overflow::Ellipsis,
    );
    let layout_ns = measure(120_000, || {
        let layout = layout_cache.layout(
            "Now playing: Lumen by Northbound",
            &shaper,
            &atlas,
            14.0,
            320.0,
            TextAlignment::Left,
            Overflow::Ellipsis,
        );
        black_box(layout);
    });

    let shader_ns = measure(200, || {
        let result = lumina_render::shaders::validate_wgsl(lumina_render::shaders::ISLAND_WGSL);
        black_box(result);
    });

    let results = [
        BenchResult {
            name: "color.static.update",
            ns_per_op: static_ns,
        },
        BenchResult {
            name: "color.reactive.update",
            ns_per_op: reactive_ns,
        },
        BenchResult {
            name: "text.shape",
            ns_per_op: text_ns,
        },
        BenchResult {
            name: "text.layout.cached",
            ns_per_op: layout_ns,
        },
        BenchResult {
            name: "shader.validate",
            ns_per_op: shader_ns,
        },
    ];

    println!("Lumina benchmark ({})", profile_name());
    println!("  platform  {}", platform.display_name());
    for result in results {
        println!("  {:24} {:10.1} ns/op", result.name, result.ns_per_op);
    }

    if let Some(path) = output {
        let json = benchmark_json(&results, platform);
        if let Err(error) = std::fs::write(path, json) {
            eprintln!("error: cannot write {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
        println!("  written   {}", path.display());
    }

    ExitCode::SUCCESS
}

fn profile_name() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn benchmark_json(results: &[BenchResult], platform: Platform) -> String {
    let mut json = String::from("{\n");
    let _ = writeln!(
        json,
        "  \"version\": \"{}\",\n  \"platform\": \"{}\",\n  \"profile\": \"{}\",\n  \"results\": [",
        env!("CARGO_PKG_VERSION"),
        platform.id(),
        profile_name()
    );
    for (index, result) in results.iter().enumerate() {
        let comma = if index + 1 == results.len() { "" } else { "," };
        let _ = writeln!(
            json,
            "    {{\"name\": \"{}\", \"ns_per_op\": {:.3}}}{comma}",
            result.name, result.ns_per_op
        );
    }
    json.push_str("  ]\n}\n");
    json
}

/// Extract `--output PATH` or `--output=PATH`.
fn output_argument(args: &[String]) -> Result<Option<PathBuf>, String> {
    if let Some(value) = args.iter().find_map(|arg| arg.strip_prefix("--output=")) {
        if value.is_empty() {
            return Err("--output needs a file path".to_string());
        }
        return Ok(Some(PathBuf::from(value)));
    }
    if let Some(index) = args.iter().position(|arg| arg == "--output") {
        let value = args
            .get(index + 1)
            .ok_or_else(|| "--output needs a file path".to_string())?;
        if value.starts_with("--") {
            return Err("--output needs a file path".to_string());
        }
        return Ok(Some(PathBuf::from(value)));
    }
    Ok(None)
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

    if args.iter().any(|a| a == "--self-test") {
        return run_self_test();
    }

    if args.iter().any(|a| a == "--benchmark") {
        return match output_argument(&args) {
            Ok(output) => run_benchmark(output.as_deref()),
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(2)
            }
        };
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
        println!("      --self-test      Run headless environment and subsystem checks");
        println!("      --benchmark      Run CPU microbenchmarks");
        println!("      --output PATH    Write benchmark results as JSON");
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
        if !single_instance::is_running("lumina") {
            eprintln!("error: no running Lumina instance was found.");
            ExitCode::FAILURE
        } else if single_instance::notify_existing("lumina", "show") {
            println!("Asked the running Lumina instance to show itself.");
            ExitCode::SUCCESS
        } else {
            eprintln!("error: could not reach the running Lumina instance.");
            ExitCode::FAILURE
        }
    } else if let Some(unknown) = args.iter().find(|a| a.as_str() != "--hidden") {
        eprintln!("error: unknown argument '{unknown}'");
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
