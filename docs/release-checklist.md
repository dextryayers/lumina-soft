# Release checklist

This checklist is the last gate before a public tag. It is intentionally
manual: CI can prove that code compiles and tests pass, but it cannot prove that
a clean machine installs the artifact or that a compositor behaves.

> **Current status: NOT READY.** The engine and headless tooling pass their
> gates, but the runtime orchestrator is not wired and no installer artifact has
> been built. Do not tag or publish `v0.1.0-alpha.1` until every blocking item
> below is complete and the manual matrix has been executed.

## 1. Scope and version

- [ ] Release scope matches `CHANGELOG.md` and `PLAN.md` Phase 16.
- [ ] `version` in `[workspace.package]` is the intended release version.
- [ ] Installer versions are numeric and monotonic. Prerelease identifiers must
      map to a documented numeric MSI/RPM version.
- [ ] `v0.1.0-alpha.1` does not reuse or move an existing tag.
- [ ] The runtime gate is complete: the window, orchestrator, service threads,
      and IPC lifecycle are wired and verified.
- [ ] No known critical or high severity issue remains open.

## 2. Automated gates

Run from a clean checkout with a cold build at least once:

```sh
cargo fmt --all --check
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --all-features
cargo test --workspace --all-targets -- --test-threads=4
cargo test --workspace --doc
cargo run -q -p lumina-app -- --self-test
cargo xtask docs --check
cargo xtask release-check
```

- [ ] Every command exits 0.
- [ ] The `lumina-ipc` doc-contract tests pass unchanged.
- [ ] The configuration-doc drift test passes.
- [ ] Theme-example tests pass and still match the built-in themes.
- [ ] CI is green on the release commit for Linux and Windows.
- [ ] `cargo-deny` and the advisory audit are green.
- [ ] MSRV 1.85 builds the workspace.

## 3. Documentation

- [ ] `README.md` has no stale status, install, or link text.
- [ ] `docs/configuration.md` is generated, never hand-edited.
- [ ] `docs/ipc.md` matches every command, event, and error code.
- [ ] `docs/themes.md` examples apply and validate.
- [ ] `docs/architecture.md` matches the shipped runtime, not just the target.
- [ ] `docs/troubleshooting.md` matches observed degraded behavior.
- [ ] `docs/manual-test-matrix.md` records every run.
- [ ] `CHANGELOG.md` has the release heading and an accurate date.
- [ ] Privacy claims are still true: no telemetry, no auto-update, no listening
      network socket, and weather as the only optional network client.
- [ ] Known limitations and unsigned-binary friction are stated plainly.

## 4. Artifact build

Use a clean build directory and a fixed source timestamp:

```sh
export SOURCE_DATE_EPOCH=1700000000
export CARGO_INCREMENTAL=0
cargo build --release --locked -p lumina-app -p lumina-ipc
```

Windows PowerShell equivalent:

```powershell
$env:SOURCE_DATE_EPOCH = "1700000000"
$env:CARGO_INCREMENTAL = "0"
cargo build --release --locked -p lumina-app -p lumina-ipc
```

- [ ] Windows MSI builds with WiX v4.
- [ ] Windows NSIS setup builds.
- [ ] Portable ZIP contains the binaries, license, README, and no local config.
- [ ] Debian-compatible staging tree is produced once and reused.
- [ ] deb, rpm, apk, and AppImage are built from that staging tree.
- [ ] Shell completions for `lumina` and `lumina-send` are installed by Linux
      packages.
- [ ] Desktop entry, AppStream metadata, icon, and AppRun are present.
- [ ] Every artifact has a SHA-256 checksum in the release manifest.
- [ ] Artifact file names contain the version and target architecture.
- [ ] Binaries are stripped and the largest artifact is within the documented
      size budget.

## 5. Install and uninstall

Execute the matching rows in `manual-test-matrix.md` on clean VMs.

- [ ] Windows 10 22H2 MSI installs per user without elevation and uninstalls
      cleanly.
- [ ] Windows 11 installs and uninstalls cleanly.
- [ ] NSIS setup installs and uninstalls cleanly.
- [ ] Portable ZIP runs without writing outside the per-user config and state
      directories.
- [ ] Ubuntu 22.04 deb installs, launches, and removes without orphaned files.
- [ ] Ubuntu 24.04 deb installs, launches, and removes without orphaned files.
- [ ] Debian 12 deb installs and removes cleanly.
- [ ] Debian 13 deb installs and removes cleanly.
- [ ] RPM installs and removes cleanly on the supported Fedora-family image.
- [ ] Alpine apk installs and starts.
- [ ] AppImage runs from a directory without writable system paths.
- [ ] Start menu, desktop entry, icon, and completions appear.
- [ ] Uninstall leaves user configuration and themes intact.
- [ ] Reinstall over an existing version works; downgrade behavior is correct.

## 6. Runtime and accessibility acceptance

- [ ] Startup to first paint is under 400 ms on the reference machine.
- [ ] Idle CPU and frame count meet the documented budget.
- [ ] Idle RSS meets the documented ceiling.
- [ ] Dirty-flag loop does not render while nothing changes.
- [ ] Media, spectrum, metrics, notifications, shortcuts, and script widgets
      behave as documented.
- [ ] Missing GPU, audio, D-Bus, tray, and IPC permissions degrade as documented.
- [ ] Keyboard focus order, Escape, and activation work.
- [ ] High-contrast theme and contrast clamp work at every RGB mode.
- [ ] Reduced motion is honored from config and OS.
- [ ] Screen-reader announcements fire once per state change.
- [ ] No crash, hang, or unbounded memory growth during a 10-minute soak.

## 7. Security and privacy review

- [ ] IPC socket is mode `0600` on Unix and user-SID restricted on Windows.
- [ ] IPC rate limiting is active.
- [ ] `execute` remains disabled by default.
- [ ] Script and shortcut execution is documented as unsandboxed.
- [ ] No telemetry, crash upload, analytics, or auto-update is present.
- [ ] Weather is disabled by default and is the only optional network client.
- [ ] Logs do not contain notification bodies, media paths, or API keys.
- [ ] Dependencies pass the license and advisory policy.
- [ ] No secrets, signing material, or local paths are committed.

## 8. Signing and provenance

Signing is out of scope for V0.1.0. That decision must be visible, not implied.

- [ ] The release notes state that Windows builds are unsigned and SmartScreen
      may warn.
- [ ] Checksums and build provenance are published next to the artifacts.
- [ ] The build uses the committed lockfile and pinned toolchain.
- [ ] The source commit hash is embedded in `--version` output or the release
      manifest.
- [ ] If signing is added later, private keys remain outside the repository and
      CI, and verification steps are documented.

## 9. Tag and publish

Only after all blocking items above are checked:

```sh
git tag -s v0.1.0-alpha.1 -m "Lumina v0.1.0-alpha.1"
git push origin v0.1.0-alpha.1
```

- [ ] Tag points at the reviewed release commit.
- [ ] Tag is signed and the signature verifies.
- [ ] Release notes link the changelog, manual matrix result, and checksums.
- [ ] Artifacts are uploaded from the tagged CI job, not from a workstation.
- [ ] The release is marked as a prerelease.
- [ ] The download page states the runtime status honestly.
- [ ] A rollback or yank procedure is written before publishing.

## 10. Post-release

- [ ] Monitor the issue tracker for installation and driver reports.
- [ ] Verify the published checksums from a second machine.
- [ ] Record the release date and artifact URLs in the changelog.
- [ ] Start the next cycle from `Unreleased`; do not reuse the tagged version.
