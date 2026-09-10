//! Debounced filesystem notifications for `Level.sav` and player saves.
//!
//! Retains the last matching event path, waits for a quiet window, and invokes
//! the callback after a size-stable read. Failed reads retry with fixed backoff;
//! exhaustion skips the callback until another event.
//!
//! Size stability does not validate content or detect same-size rewrites.
//! Decoding, hashing, and ingest are the caller's responsibility.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

/// Debounce, size-poll, and read-attempt settings.
#[derive(Debug, Clone, Copy)]
pub struct WatchConfig {
    pub debounce: Duration,
    pub stability_poll: Duration,
    pub max_retries: u32,
    pub retry_backoff: Duration,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_secs(2),
            stability_poll: Duration::from_millis(200),
            max_retries: 5,
            retry_backoff: Duration::from_millis(500),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("notify error: {0}")]
    Notify(#[from] notify::Error),
}

/// Read `path` after two equal size samples `config.stability_poll` apart.
/// Requires the read byte count to match the sampled size.
///
/// Makes at most `config.max_retries` attempts with fixed `retry_backoff`
/// between attempts. Returns `None` on exhaustion; does not validate contents.
#[must_use]
pub fn read_when_stable(path: &Path, config: &WatchConfig) -> Option<Vec<u8>> {
    for attempt in 0..config.max_retries {
        if attempt > 0 {
            std::thread::sleep(config.retry_backoff);
        }
        let Ok(size_before) = std::fs::metadata(path).map(|m| m.len()) else {
            continue;
        };
        std::thread::sleep(config.stability_poll);
        let Ok(size_after) = std::fs::metadata(path).map(|m| m.len()) else {
            continue;
        };
        if size_before != size_after {
            continue; // still being written
        }
        let Ok(mut file) = std::fs::File::open(path) else {
            continue;
        };
        let mut buf = Vec::new();
        if file.read_to_end(&mut buf).is_ok() && buf.len() as u64 == size_after {
            return Some(buf);
        }
    }
    None
}

/// Watch `world_dir` recursively and invoke `on_change` after a quiet debounce
/// window. Only the last matching path is read; the callback receives that path
/// and its size-checked bytes. Paths that exhaust read attempts are skipped.
///
/// Matches `Level.sav` and `.sav` files in `Players`, excluding `*_dps.sav`.
/// Blocks until the event channel disconnects; run on a dedicated thread.
///
/// # Errors
///
/// Returns [`WatchError::Notify`] if the platform watcher can't be created or
/// can't be pointed at `world_dir`.
pub fn watch(
    world_dir: &Path,
    config: WatchConfig,
    on_change: impl FnMut(&Path, Vec<u8>),
) -> Result<(), WatchError> {
    static NEVER: AtomicBool = AtomicBool::new(false);
    watch_until(world_dir, config, &NEVER, on_change)
}

/// [`watch`] with a cooperative stop flag.
///
/// Checks `stop` between event windows and before a pending read. An active
/// read or callback may finish before the watcher returns.
///
/// # Errors
///
/// Returns [`WatchError::Notify`] if the platform watcher can't be created or
/// can't be pointed at `world_dir`.
pub fn watch_until(
    world_dir: &Path,
    config: WatchConfig,
    stop: &AtomicBool,
    mut on_change: impl FnMut(&Path, Vec<u8>),
) -> Result<(), WatchError> {
    let (tx, rx) = channel::<PathBuf>();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            for path in event.paths {
                if is_watched_save(&path) {
                    let _ = tx.send(path);
                }
            }
        }
    })?;
    watcher.watch(world_dir, RecursiveMode::Recursive)?;

    let mut pending: Option<PathBuf> = None;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let wait = config.debounce;
        match rx.recv_timeout(wait) {
            Ok(path) => {
                pending = Some(path);
                // Keep draining immediately-available events into the same
                // debounce window rather than firing once per event.
                while let Ok(path) = rx.try_recv() {
                    pending = Some(path);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(path) = pending.take() {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    if let Some(bytes) = read_when_stable(&path, &config) {
                        on_change(&path, bytes);
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn is_watched_save(path: &Path) -> bool {
    match path.file_name().and_then(|n| n.to_str()) {
        Some("Level.sav") => true,
        Some(name) => {
            name.ends_with(".sav")
                && !name.ends_with("_dps.sav")
                && path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    == Some("Players")
        }
        None => false,
    }
}
