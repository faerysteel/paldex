# paldex-store

SQLite snapshot persistence and filesystem change notifications. Save decoding,
content hashing, and sync orchestration belong to the app.

## Storage model

- `worlds`: save-world identity, source, path, and last-seen timestamp.
- `snapshots`: world id, capture timestamp, `Level.sav` mtime, and caller-supplied
  content hash.
- Snapshot rows: `pals`, `pal_passives`, `pal_moves`, `players`, `player_flags`,
  `guilds`, and `base_camps`. Each ingest inserts a complete new set under a new
  snapshot id; it does not replace or delete previous snapshots.
- `dex_events`: first observed unlock per `(world_id, player_uid, character_id)`.
  `INSERT OR IGNORE` preserves `first_seen_at` across ingests. Snapshot pruning
  leaves these events intact; deleting a world cascades to them.

`player_flags` stores sets and counters as
`(snapshot_id, player_uid, flag_kind, flag_key, value)`. `value` is an integer for
counters and `NULL` for presence-only flags; the table has no uniqueness constraint.

## API

| API | Contract |
| --- | --- |
| `Store::open(path)` | Open/create a database, enable foreign keys, apply migrations |
| `Store::open_in_memory()` | Create an in-memory store with the same schema |
| `Store::upsert_world(...)` | Insert or refresh world metadata |
| `Store::ingest_snapshot(...)` | Insert a snapshot and append dex events in one transaction; requires `&mut self` |
| `Store::latest_snapshot_id(world_id)` | Find the newest snapshot by capture timestamp |
| `Store::latest_hash_matches(world_id, hash)` | Compare the supplied hash with the newest stored hash |
| `Store::prune_snapshots(world_id, keep)` | Keep the newest snapshots by capture timestamp; return the number deleted |
| `Store::conn()` | Expose the SQLite connection for queries |

Ingest neither computes hashes nor skips duplicates. Retention is explicit:
callers must invoke `prune_snapshots`; the desktop app currently does not.

## Migrations

`PRAGMA user_version` tracks schema version. Opening an older database applies
missing migrations in order and stamps version 3.

| Version | File | Change |
| --- | --- | --- |
| 1 | `0001_init.sql` | Core tables and indexes |
| 2 | `0002_player_identity.sql` | `players.name`, `players.level` |
| 3 | `0003_base_worker_container.sql` | `base_camps.worker_container_id` |

For `user_version = 0`, `detected_version` checks schema markers newest-first.
An empty database starts at version 0; an existing schema resumes at the inferred
version.

## Watcher

`watch` and `watch_until` recursively monitor a world directory. Matching paths
are `Level.sav` and `.sav` files directly inside a `Players` directory, excluding
`*_dps.sav`.

1. Events reset the debounce timer; only the last matching path is retained.
2. After a quiet window, `read_when_stable` compares file sizes across two polls,
   reads the file, and checks the byte count against the sampled size.
3. A successful read invokes `on_change(path, bytes)`. Failed attempts retry with
   fixed backoff; exhaustion skips the callback until another event.

The watcher does not decompress, validate, hash, or ingest bytes. Size stability
cannot detect same-size rewrites or guarantee a complete save. The app ignores
the callback bytes and re-reads the world for synchronization.

`watch` blocks; `watch_until` adds a cooperative stop flag checked between event
windows. An active read or callback may finish before shutdown.

| `WatchConfig` field | Default |
| --- | --- |
| `debounce` | 2 s |
| `stability_poll` | 200 ms |
| `max_retries` | 5 total read attempts |
| `retry_backoff` | 500 ms |

## Source and verification

| Path | Coverage |
| --- | --- |
| [`src/lib.rs`](src/lib.rs) | Store API, migrations, retention |
| [`src/ingest.rs`](src/ingest.rs) | Snapshot rows and dex-event insertion |
| [`src/watch.rs`](src/watch.rs) | Debounce, stable reads, stop handling |
| [`migrations/`](migrations/) | Versioned SQL |
| [`tests/watch.rs`](tests/watch.rs) | Read attempts, debounce, shutdown |
| [`tests/reopen.rs`](tests/reopen.rs) | Reopening, migration, legacy schema detection |
| [`tests/real_ingest.rs`](tests/real_ingest.rs) | Decode/ingest counts, dex monotonicity, retention, hash lookup |

```bash
cargo test -p paldex-store
```

Run from the repository root. Real-ingest tests use save discovery or
`PALDEX_TEST_SAVE_DIR` and skip when no trackable world is available.
