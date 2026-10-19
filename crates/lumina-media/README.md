# lumina-media

Media session abstraction for Lumina: MPRIS on Linux, GSMTC on Windows.

## Features

- **MPRIS2 adapter** for Linux: D-Bus discovery, properties, signals
- **GSMTC adapter** for Windows: WinRT sessions, async bridging
- **Player ranking** with capability-aware scoring and sticky selection
- **Album art pipeline**: extraction, cache, downscale, dominant color
- **Multi-player cycling** with hotkey and auto-fallback
- **Failure handling** for all failure modes from PLAN.md §16.5

## Usage

```rust
use lumina_media::{MediaSession, MediaWatcher, RankingConfig};

// Discover sessions
let sessions = lumina_media::mpris::discover_sessions();

// Create a watcher
let config = RankingConfig::default();
let mut watcher = MediaWatcher::new(config, 1000);

// Poll for the active player
if let Some(snapshot) = watcher.poll(sessions) {
    println!("Now playing: {} - {}", snapshot.title_or_placeholder(), snapshot.artist_or_placeholder());
}
```
