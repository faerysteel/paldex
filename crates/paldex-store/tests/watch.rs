//! Watcher behavior tests using short, test-only timings (see `WatchConfig`)
//! rather than the real ~2s debounce — these assert the *logic*
//! (debouncing, stability, retry) works, not real-world timing.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use paldex_store::{read_when_stable, watch, WatchConfig};

fn fast_config() -> WatchConfig {
    WatchConfig {
        debounce: Duration::from_millis(80),
        stability_poll: Duration::from_millis(20),
        max_retries: 10,
        retry_backoff: Duration::from_millis(20),
    }
}

#[test]
fn read_when_stable_returns_the_final_content_of_a_settled_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Level.sav");
    std::fs::write(&path, b"hello world").unwrap();

    let bytes = read_when_stable(&path, &fast_config()).expect("should read a settled file");
    assert_eq!(bytes, b"hello world");
}

#[test]
fn read_when_stable_gives_up_gracefully_on_a_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("does-not-exist.sav");

    let mut cfg = fast_config();
    cfg.max_retries = 2; // keep the test fast
    assert!(read_when_stable(&path, &cfg).is_none());
}

#[test]
fn rapid_writes_within_the_debounce_window_trigger_exactly_one_callback() {
    let dir = tempfile::tempdir().unwrap();
    let world_dir = dir.path().to_path_buf();
    std::fs::write(world_dir.join("Level.sav"), b"initial").unwrap();

    let calls: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let calls_clone = calls.clone();
    let config = fast_config();

    let handle = thread::spawn(move || {
        let _ = watch(&world_dir, config, move |_path, bytes| {
            calls_clone.lock().unwrap().push(bytes);
        });
    });

    // Give the watcher a moment to start, then fire 10 rapid writes well
    // within the debounce window (80ms), each far faster than that.
    thread::sleep(Duration::from_millis(100));
    let level_path = dir.path().join("Level.sav");
    for i in 0..10 {
        std::fs::write(&level_path, format!("write-{i}")).unwrap();
        thread::sleep(Duration::from_millis(5));
    }

    // Wait past the debounce window plus stability polling so the single
    // collapsed event has time to fire and be read.
    thread::sleep(Duration::from_millis(400));

    let seen = calls.lock().unwrap();
    assert_eq!(seen.len(), 1, "expected exactly one debounced callback, got {}", seen.len());
    assert_eq!(seen[0], b"write-9");

    drop(handle); // the watcher thread runs forever; let the test process exit
}
