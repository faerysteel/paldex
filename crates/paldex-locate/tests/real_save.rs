//! Integration tests against a real Palworld installation.
//!
//! These are the tests that catch drift between our assumptions and what the game
//! actually writes, so they run against real save files rather than fixtures. They
//! resolve a save directory in this order:
//!
//! 1. `PALDEX_TEST_SAVE_DIR` — an explicit per-Steam-ID directory
//! 2. whatever [`paldex_locate::discover`] finds on this machine
//!
//! If neither yields anything the test reports a skip and passes, so that CI and
//! machines without Palworld installed stay green. Nothing personal is committed:
//! the path comes from the environment or from discovery.

use paldex_locate::{discover, resolve_manual, SaveRoot, WorldKind};

/// A real save root, or `None` if this machine has no Palworld install.
fn real_root() -> Option<SaveRoot> {
    if let Ok(dir) = std::env::var("PALDEX_TEST_SAVE_DIR") {
        let roots = resolve_manual(std::path::Path::new(&dir))
            .unwrap_or_else(|e| panic!("PALDEX_TEST_SAVE_DIR is set but unusable: {e}"));
        return roots.into_iter().next();
    }
    discover().into_iter().next()
}

macro_rules! require_real_save {
    () => {
        match real_root() {
            Some(root) => root,
            None => {
                eprintln!("skipping: no Palworld save found (set PALDEX_TEST_SAVE_DIR)");
                return;
            }
        }
    };
}

#[test]
fn discovery_finds_a_save_root() {
    let root = require_real_save!();

    assert!(root.path.is_dir(), "discovered path must exist");
    assert!(
        !root.steam_id.is_empty(),
        "a discovered root must carry its Steam ID"
    );
    eprintln!("found {} via {}", root.path.display(), root.source);
}

#[test]
fn a_real_root_contains_at_least_one_world() {
    let root = require_real_save!();

    let worlds = root.worlds();

    assert!(!worlds.is_empty(), "save root has no world directories");
    for w in &worlds {
        assert!(w.path.is_dir());
        assert!(!w.id.is_empty());
    }
}

#[test]
fn hosted_worlds_are_distinguished_from_guest_stubs() {
    let root = require_real_save!();

    let worlds = root.worlds();
    let hosted: Vec<_> = worlds.iter().filter(|w| w.is_trackable()).collect();
    let stubs: Vec<_> = worlds
        .iter()
        .filter(|w| w.kind == WorldKind::CoopGuestStub)
        .collect();

    eprintln!(
        "{} world(s): {} hosted, {} co-op guest stub(s)",
        worlds.len(),
        hosted.len(),
        stubs.len()
    );

    // A hosted world must have a Level.sav and at least one player; a stub must have
    // neither. This is the invariant the whole UI distinction rests on.
    for w in hosted {
        assert!(w.path.join("Level.sav").is_file());
        assert!(
            !w.players.is_empty(),
            "hosted world {} has no player saves",
            w.id
        );
    }
    for w in stubs {
        assert!(!w.path.join("Level.sav").exists());
        assert!(w.players.is_empty());
    }
}

#[test]
fn worlds_are_ordered_most_recently_played_first() {
    let root = require_real_save!();

    let worlds = root.worlds();
    let times: Vec<_> = worlds.iter().filter_map(|w| w.last_played).collect();

    assert!(
        times.windows(2).all(|w| w[0] >= w[1]),
        "worlds are not in descending last-played order"
    );
}

#[test]
fn every_hosted_world_excludes_dps_files_from_its_player_list() {
    let root = require_real_save!();

    for w in root.worlds().iter().filter(|w| w.is_trackable()) {
        let on_disk = std::fs::read_dir(w.path.join("Players"))
            .expect("hosted world must have a Players directory")
            .flatten()
            .count();

        // Palworld writes both `<uid>.sav` and `<uid>_dps.sav` per player, so the
        // player count should be half the file count.
        assert_eq!(
            w.players.len() * 2,
            on_disk,
            "world {} — {} players from {} files",
            w.id,
            w.players.len(),
            on_disk
        );
    }
}
