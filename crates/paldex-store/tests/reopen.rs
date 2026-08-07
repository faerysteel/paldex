//! Reopening a database file must be a no-op, not an error.
//!
//! Regression test for a bug the rest of the suite could not see: every other
//! test uses `Store::open_in_memory()`, which is always a fresh database, so
//! re-running the schema DDL never collided. Against a real file it failed
//! with `table worlds already exists` on the second open — meaning the app
//! worked once and then broke on every subsequent launch.

use paldex_store::Store;

#[test]
fn reopening_an_existing_database_file_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("paldex.sqlite3");

    let first = Store::open(&path).expect("first open");
    first
        .upsert_world("world-1", "test", "steam-1", "/tmp/world-1")
        .expect("write through the first handle");
    drop(first);

    let second = Store::open(&path).expect("reopening an existing database should succeed");
    // Data written before the reopen must still be there — a migration that
    // silently recreated the schema would look like success but lose it.
    let worlds: i64 = second
        .conn()
        .query_row("SELECT COUNT(*) FROM worlds", [], |row| row.get(0))
        .expect("count worlds");
    assert_eq!(worlds, 1, "reopening must preserve existing rows");

    // A third open exercises the already-stamped path.
    Store::open(&path).expect("third open");
}

/// Databases created before schema versioning have the tables but
/// `user_version = 0`; they must be adopted rather than re-migrated.
#[test]
fn adopts_an_unversioned_database_that_already_has_the_schema() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("legacy.sqlite3");

    {
        let store = Store::open(&path).expect("create");
        store
            .conn()
            .pragma_update(None, "user_version", 0)
            .expect("simulate a pre-versioning database");
    }

    Store::open(&path).expect("an unversioned but fully-formed database should be adopted");
}
