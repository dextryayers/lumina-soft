//! Layering partial configuration documents over a complete one.
//!
//! Profiles and themes both express their overrides the same way: a sparse TOML
//! document that is layered over the full configuration. This module owns that
//! one mechanism so the two features cannot drift apart.
//!
//! # Why serialise and re-deserialise rather than walk the tree
//!
//! The obvious implementation is to iterate the partial document's keys and
//! assign them onto the already-deserialised struct. That requires a hand
//! written setter per field, which is about two thousand lines that would all
//! need tests and would all need updating when a field is added.
//!
//! Instead the partial is serialised to text and deserialised on top of the
//! base's text. Because every schema field is `#[serde(default)]`, absent keys
//! keep the base value and present keys win. The consequence that matters is
//! that a profile uses *exactly* the same syntax as the config file, so there is
//! no second dialect to document and no third code path to keep in sync.

use crate::schema::Config;

/// Expand literal dotted keys into nested tables.
///
/// A TOML document can express a nested key two ways. `rgb.mode = "x"` is
/// expanded by the parser into nested tables. A *quoted* `"rgb.mode" = "x"` is
/// a single key whose name happens to contain dots, which is what a user
/// writing inside a `[profiles.name]` table actually types.
///
/// Users expect both to work and to mean the same thing, so quoted dotted keys
/// are expanded here before the document is merged.
///
/// # Panics
///
/// Never. A key that is already a table is left alone, and a malformed path is
/// inserted as a literal key rather than being dropped, so nothing the user
/// wrote disappears without trace.
#[must_use]
pub fn expand_dotted(table: &toml::Table) -> toml::Table {
    let mut result = toml::Table::new();

    for (key, value) in table {
        if !key.contains('.') {
            insert(&mut result, key, value.clone());
            continue;
        }

        // Walk the path, creating intermediate tables as needed.
        let mut segments = key.split('.').peekable();
        let mut current = &mut result;
        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                insert(current, segment, value.clone());
                break;
            }
            let entry = current
                .entry(segment.to_string())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            if !entry.is_table() {
                // A scalar is sitting where a table is needed. Replace it, since
                // the deeper key is the one the user actually meant.
                *entry = toml::Value::Table(toml::Table::new());
            }
            current = entry.as_table_mut().expect("just ensured it is a table");
        }
    }

    result
}

/// Insert a value into a table, merging tables rather than replacing them.
///
/// Without this, two quoted keys sharing a prefix would overwrite each other:
/// `"rgb.mode"` and `"rgb.speed"` would leave only whichever came last.
fn insert(target: &mut toml::Table, key: &str, value: toml::Value) {
    match (target.get_mut(key), &value) {
        (Some(toml::Value::Table(existing)), toml::Value::Table(incoming)) => {
            for (k, v) in incoming {
                insert(existing, k, v.clone());
            }
        }
        _ => {
            target.insert(key.to_string(), value);
        }
    }
}

/// Recursively merge `overlay` into `base`, with `overlay` winning.
///
/// Used when a theme `extends` another and both define the same table.
pub fn merge_tables(base: &mut toml::Table, overlay: &toml::Table) {
    for (key, value) in overlay {
        match (base.get_mut(key), value) {
            (Some(toml::Value::Table(existing)), toml::Value::Table(incoming)) => {
                merge_tables(existing, incoming);
            }
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Layer a partial document over a complete configuration.
///
/// Returns the base unchanged when the overlay is empty, when the base cannot be
/// represented as a TOML document, or when the merged document does not
/// deserialise. Every one of those is a recoverable case that should degrade to
/// "no override" rather than lose the user's whole configuration.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn overlay_config(base: &Config, partial: &toml::Table) -> Config {
    if partial.is_empty() {
        return base.clone();
    }

    // Convert to a document and merge structurally. Doing this on text instead
    // would mean appending `[rgb]` after the base's own `[rgb]`, which TOML
    // rejects as a duplicate table definition.
    let Ok(mut document) = toml::Table::try_from(base.clone()) else {
        return base.clone();
    };

    merge_tables(&mut document, &expand_dotted(partial));

    let Ok(value) = toml::Value::try_from(document) else {
        return base.clone();
    };
    value.try_into().unwrap_or_else(|_| base.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{RgbMode, VisualizerStyle};

    fn table(text: &str) -> toml::Table {
        toml::from_str(text).expect("test table parses")
    }

    #[test]
    fn plain_keys_pass_through_unchanged() {
        let expanded = expand_dotted(&table("version = 1\nname = \"x\"\n"));
        assert_eq!(expanded.get("version").and_then(toml::Value::as_integer), Some(1));
        assert_eq!(expanded.get("name").and_then(toml::Value::as_str), Some("x"));
    }

    #[test]
    fn quoted_dotted_keys_expand_into_nested_tables() {
        let expanded = expand_dotted(&table("\"rgb.mode\" = \"cycling\"\n"));
        let rgb = expanded.get("rgb").and_then(toml::Value::as_table).expect("rgb table");
        assert_eq!(rgb.get("mode").and_then(toml::Value::as_str), Some("cycling"));
    }

    #[test]
    fn unquoted_dotted_keys_arrive_already_nested() {
        // TOML expands this one for us, so the expander must leave it alone.
        let expanded = expand_dotted(&table("rgb.mode = \"cycling\"\n"));
        let rgb = expanded.get("rgb").and_then(toml::Value::as_table).expect("rgb table");
        assert_eq!(rgb.get("mode").and_then(toml::Value::as_str), Some("cycling"));
    }

    #[test]
    fn deeply_nested_dotted_keys_expand() {
        let expanded = expand_dotted(&table("\"audio.visualizer.style\" = \"waveform\"\n"));
        let audio = expanded.get("audio").and_then(toml::Value::as_table).expect("audio");
        let visualizer = audio.get("visualizer").and_then(toml::Value::as_table).expect("visualizer");
        assert_eq!(visualizer.get("style").and_then(toml::Value::as_str), Some("waveform"));
    }

    #[test]
    fn two_keys_sharing_a_prefix_do_not_overwrite_each_other() {
        // This is the bug a naive expansion has: inserting "rgb.mode" then
        // "rgb.speed" must leave both, not just the second.
        let expanded = expand_dotted(&table("\"rgb.mode\" = \"cycling\"\n\"rgb.speed\" = 2.0\n"));
        let rgb = expanded.get("rgb").and_then(toml::Value::as_table).expect("rgb table");
        assert_eq!(rgb.len(), 2, "both keys should survive: {rgb:?}");
        assert_eq!(rgb.get("mode").and_then(toml::Value::as_str), Some("cycling"));
        assert_eq!(rgb.get("speed").and_then(toml::Value::as_float), Some(2.0));
    }

    #[test]
    fn a_scalar_where_a_table_is_needed_is_replaced() {
        let expanded = expand_dotted(&table("rgb = 5\n\"rgb.mode\" = \"cycling\"\n"));
        let rgb = expanded.get("rgb").and_then(toml::Value::as_table).expect("rgb becomes a table");
        assert_eq!(rgb.get("mode").and_then(toml::Value::as_str), Some("cycling"));
    }

    #[test]
    fn overlay_applies_top_level_overrides() {
        let base = Config::default();
        let applied = overlay_config(&base, &table("\"rgb.mode\" = \"cycling\"\n"));
        assert_eq!(applied.rgb.mode, RgbMode::Cycling);
        // Untouched keys keep the base value.
        assert_eq!(applied.rgb.speed, base.rgb.speed);
    }

    #[test]
    fn overlay_applies_nested_overrides() {
        let base = Config::default();
        let applied = overlay_config(&base, &table("\"audio.visualizer.style\" = \"waveform\"\n"));
        assert_eq!(applied.audio.visualizer.style, VisualizerStyle::Waveform);
    }

    #[test]
    fn overlay_combines_multiple_keys_in_one_section() {
        let base = Config::default();
        let applied = overlay_config(
            &base,
            &table("\"rgb.mode\" = \"static\"\n\"rgb.speed\" = 2.0\n\"rgb.brightness\" = 0.5\n"),
        );
        assert_eq!(applied.rgb.mode, RgbMode::Static);
        assert_eq!(applied.rgb.speed, 2.0);
        assert_eq!(applied.rgb.brightness, 0.5);
    }

    #[test]
    fn empty_overlay_returns_the_base() {
        let base = Config::default();
        assert_eq!(overlay_config(&base, &toml::Table::new()), base);
    }

    #[test]
    fn an_overlay_naming_a_nonexistent_key_returns_the_base() {
        // The overlay does not deserialise, so we keep the base rather than
        // discarding the whole configuration. The validator reports the unknown
        // key separately.
        let base = Config::default();
        let applied = overlay_config(&base, &table("\"rgb.not_a_real_key\" = 5\n"));
        assert_eq!(applied, base, "a bad overlay must not damage the base");
    }

    #[test]
    fn an_overlay_with_a_wrong_type_returns_the_base() {
        let base = Config::default();
        let applied = overlay_config(&base, &table("\"rgb.mode\" = 42\n"));
        assert_eq!(applied, base);
    }

    #[test]
    fn overlay_does_not_mutate_the_base() {
        let base = Config::default();
        let before = base.clone();
        let _ = overlay_config(&base, &table("\"rgb.mode\" = \"cycling\"\n"));
        assert_eq!(base, before);
    }

    #[test]
    fn merge_tables_prefers_the_overlay() {
        let mut base = table("rgb.mode = \"static\"\nrgb.speed = 1.0\n");
        let overlay = table("rgb.mode = \"reactive\"\n");
        merge_tables(&mut base, &overlay);

        let rgb = base.get("rgb").and_then(toml::Value::as_table).expect("rgb");
        assert_eq!(rgb.get("mode").and_then(toml::Value::as_str), Some("reactive"));
        assert_eq!(rgb.get("speed").and_then(toml::Value::as_float), Some(1.0), "sibling keys survive");
    }
}