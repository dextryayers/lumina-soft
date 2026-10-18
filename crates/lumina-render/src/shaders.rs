//! WGSL shader sources, embedded at compile time.
//!
//! Every shader is a plain string constant so it can be validated through
//! `naga` in tests without a GPU. That is the single most valuable property of
//! this module: a shader that fails to compile is caught in CI on a machine
//! with no graphics hardware at all.

/// The island body shader.
///
/// Draws one full-target quad and evaluates the island shape per pixel as a
/// signed distance field. Everything else (border, glow, fill) is derived from
/// that field in the same fragment shader, which is what makes the shape
/// resolution independent and the glow follow the shape exactly.
///
/// The shape is an inverted trapezoid: the top edge is wider than the bottom
/// edge. See [`crate::sdf`](lumina_core::sdf) for the Rust reference this
/// mirrors.
pub const ISLAND_WGSL: &str = include_str!("../assets/shaders/island.wgsl");

/// Validate a WGSL source through `naga`.
///
/// Returns `None` when the source parses and validates cleanly, or the error
/// when it does not. Called at startup before any shader reaches the driver,
/// because a shader that fails validation on one GPU driver is a real possibility
/// and the fallback should be used rather than crashing.
///
/// # Panics
///
/// Never.
pub fn validate_wgsl(source: &str) -> Option<String> {
    let module = match naga::front::wgsl::parse_str(source) {
        Ok(module) => module,
        Err(e) => return Some(e.emit_to_string(source)),
    };

    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );

    match validator.validate(&module) {
        Ok(_) => None,
        Err(e) => Some(format!("{e:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_island_shader_validates_cleanly() {
        assert!(
            validate_wgsl(ISLAND_WGSL).is_none(),
            "the island shader must validate: {:?}",
            validate_wgsl(ISLAND_WGSL)
        );
    }

    #[test]
    fn a_broken_shader_is_rejected_with_a_message() {
        let broken = "fn main() { this is not valid wgsl }";
        let error = validate_wgsl(broken).expect("a broken shader must be rejected");
        assert!(!error.is_empty(), "the error should say what went wrong");
    }

    #[test]
    fn an_empty_shader_is_rejected() {
        assert!(validate_wgsl("").is_some(), "an empty shader is not valid");
    }

    #[test]
    fn the_island_shader_declares_the_uniforms_the_render_loop_writes() {
        // The shader and the Rust uniform struct have to agree. A mismatch shows
        // up as a silently wrong render, so this checks the names directly.
        for name in ["half_top", "half_bottom", "half_h", "radius", "fill_rgba", "glow_rgb", "border_width"] {
            assert!(
                ISLAND_WGSL.contains(name),
                "the island shader must declare {name}"
            );
        }
    }
}