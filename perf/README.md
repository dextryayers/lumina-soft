# Performance baselines

`lumina --benchmark --output FILE` records CPU-side microbenchmarks as JSON.
Each metric is the median of five rounds; the shader parser benchmark is the
dominant cost because it validates the full WGSL module.

Record or refresh a baseline on the reference machine:

```powershell
cargo run -p lumina-app --release -- --benchmark --output perf/baseline-windows.json
```

Enforce the five percent regression gate:

```powershell
cargo run -p lumina-app --release -- --benchmark --baseline perf/baseline-windows.json
```

A baseline is platform-specific and exits non-zero when any metric is more than
5% slower. The comparison is also profile-specific: do not compare a debug run
against a release baseline.

## CI

The `performance` job runs the benchmarks but does not enforce the gate yet.
GitHub-hosted runners have enough frequency scaling and neighbour noise that a
5% limit produces false failures; two consecutive runs on the reference machine
can differ by more than that for the short text benchmarks. Enforce the gate on
a dedicated benchmark runner (or a quiet self-hosted machine) where the baseline
was recorded, and refresh `perf/baseline-<platform>.json` with the same toolchain
and profile.

The frame histogram (`FrameStats::p50_ms`, `p95_ms`, `p99_ms`) is the real
GPU-side regression signal; it becomes usable once the application orchestrator
exists and can render frames in CI with Lavapipe.
