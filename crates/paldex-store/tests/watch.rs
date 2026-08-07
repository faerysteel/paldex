//! Watcher behavior tests using short, test-only timings (see `WatchConfig`)
//! rather than the real ~2s debounce — these assert the *logic*
//! (debouncing, stability, retry) works, not real-world timing.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use paldex_store::{read_when_stable, watch, watch_until, WatchConfig};

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

/// The app re-points the watcher whenever a different world is selected. If a
/// stopped watcher kept running, the previous world would go on re-ingesting
/// behind the new selection — so this asserts both halves: the thread actually
/// terminates, and it stops firing.
#[test]
fn a_stopped_watcher_terminates_and_fires_no_further_callbacks() {
    let dir = tempfile::tempdir().unwrap();
    let world_dir = dir.path().to_path_buf();
    std::fs::write(world_dir.join("Level.sav"), b"initial").unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let (stop_thread, calls_thread) = (Arc::clone(&stop), Arc::clone(&calls));
    let config = fast_config();

    let handle = thread::spawn(move || {
        let _ = watch_until(&world_dir, config, &stop_thread, move |_path, _bytes| {
            calls_thread.fetch_add(1, Ordering::Relaxed);
        });
    });

    thread::sleep(Duration::from_millis(100));
    stop.store(true, Ordering::Relaxed);

    // The flag is read between debounce windows (80ms here), so this must not
    // hang. A watcher that ignored it would leave the thread alive forever and
    // spin here until the deadline.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handle.is_finished() {
        assert!(Instant::now() < deadline, "watch_until ignored the stop flag");
        thread::sleep(Duration::from_millis(20));
    }
    handle.join().unwrap();

    let after_stop = calls.load(Ordering::Relaxed);
    std::fs::write(dir.path().join("Level.sav"), b"post-stop").unwrap();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        calls.load(Ordering::Relaxed),
        after_stop,
        "a stopped watcher must not fire again"
    );
}
