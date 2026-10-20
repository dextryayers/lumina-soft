//! Spring integration benchmark for the performance CI job.
//!
//! `harness = false` keeps this a plain binary, so `cargo bench` runs it on
//! stable with no nightly-only test harness. The numbers are CPU-side and do
//! not need a GPU, which is why they can run on every CI push.

use std::hint::black_box;
use std::time::Instant;

use lumina_core::Spring;

const WARMUP_SECONDS: u32 = 2;
const MEASURE_SECONDS: u32 = 5;
const STEP: f32 = 1.0 / 240.0;
const STEPS_PER_SECOND: u32 = 240;

fn main() {
    let mut spring = Spring::new(62.0, 0.34, 0.78);
    spring.set_target(320.0);

    // Warm the branch predictors and let the CPU reach a stable clock.
    for _ in 0..(WARMUP_SECONDS * STEPS_PER_SECOND) {
        spring.advance(STEP);
        black_box(spring.position());
    }

    spring.snap_to_target();
    spring.set_target(320.0);
    black_box(spring.position());

    let steps = u64::from(MEASURE_SECONDS * STEPS_PER_SECOND);
    let start = Instant::now();
    for _ in 0..steps {
        spring.advance(STEP);
        black_box(spring.position());
    }
    let elapsed = start.elapsed();

    let ns_per_step = elapsed.as_secs_f64() * 1.0e9 / steps as f64;
    let simulated_ms = steps as f64 * f64::from(STEP) * 1000.0;

    println!("Lumina spring benchmark");
    println!("  steps          {steps}");
    println!("  simulated      {simulated_ms:.1} ms");
    println!("  wall           {:.1} ms", elapsed.as_secs_f64() * 1000.0);
    println!("  per step       {ns_per_step:.1} ns");
    println!(
        "  realtime       {:.1}x",
        simulated_ms / (elapsed.as_secs_f64() * 1000.0)
    );
}
