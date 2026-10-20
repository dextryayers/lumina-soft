use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use lumina_config::theme::builtin_themes;
use lumina_config::theme::{Theme, ThemeStore};
use lumina_config::validate::validate;
use lumina_config::Config;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/themes")
}

fn examples() -> Vec<(String, Theme)> {
    let dir = examples_dir();
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    files.sort();

    files
        .into_iter()
        .map(|path| {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            let theme = Theme::from_toml_str(&text)
                .unwrap_or_else(|e| panic!("{} is not a valid theme: {e}", path.display()));
            let stem = path
                .file_stem()
                .expect("theme file has a stem")
                .to_string_lossy()
                .into_owned();
            (stem, theme)
        })
        .collect()
}

fn applied(name: &str, theme: Theme, base: &Config) -> Config {
    let mut store = ThemeStore::empty();
    store.insert(name, theme);
    store.apply(base, name)
}

#[test]
fn eight_theme_examples_ship_and_match_the_built_ins() {
    let examples = examples();
    assert!(
        examples.len() >= 8,
        "expected at least 8 theme examples, found {}",
        examples.len()
    );

    let builtins: std::collections::BTreeMap<&str, Theme> = builtin_themes().into_iter().collect();
    for (stem, theme) in &examples {
        let builtin = builtins
            .get(stem.as_str())
            .unwrap_or_else(|| panic!("{stem}.toml has no matching built-in theme"));
        assert_eq!(
            theme, builtin,
            "{stem}.toml has drifted from the built-in theme"
        );
    }

    let names: BTreeSet<&str> = examples
        .iter()
        .map(|(_, theme)| theme.name.as_str())
        .collect();
    assert_eq!(
        names.len(),
        examples.len(),
        "theme display names must be unique"
    );
}

#[test]
fn every_theme_example_applies_and_validates_cleanly() {
    for (stem, theme) in examples() {
        let applied = applied(&stem, theme, &Config::default());
        let (checked, report) = validate(applied.clone());
        assert!(
            report.is_clean(),
            "{stem} produces validation issues: {}",
            report.render()
        );
        assert_eq!(checked, applied, "{stem} is altered by validation");
    }
}

#[test]
fn the_high_contrast_example_is_safe_and_legible() {
    let (_, theme) = examples()
        .into_iter()
        .find(|(stem, _)| stem == "high-contrast")
        .expect("high-contrast example");
    let applied = applied("high-contrast", theme, &Config::default());

    assert_eq!(applied.appearance.fill, "#000000");
    assert_eq!(applied.appearance.glow_spread, 0.0);
    assert_eq!(applied.rgb.glow_intensity, 0.0);
    assert_eq!(applied.accessibility.contrast_target, 7.0);
}

#[test]
fn exporting_a_theme_round_trips_through_toml() {
    for (_, theme) in examples() {
        let text = theme.to_toml_string().expect("theme serializes");
        let parsed = Theme::from_toml_str(&text).expect("serialized theme parses");
        assert_eq!(parsed, theme);
    }
}
