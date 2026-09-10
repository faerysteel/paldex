//! Integration tests against a real Palworld installation.
//!
//! Resolves a save directory the same way `paldex-locate`'s own real-save tests do:
//! `PALDEX_TEST_SAVE_DIR` first, falling back to [`paldex_locate::discover`]. If neither
//! yields a trackable (hosted) world, every test here skips rather than failing, so CI
//! and machines without Palworld installed stay green.

use std::path::PathBuf;

use paldex_locate::{discover, resolve_manual, World};
use paldex_sav::Compression;

/// The six per-world files a hosted world always has, verified against the real
/// install: the four loose world files plus a player's `.sav` and its `_dps.sav`.
fn hosted_world_files() -> Option<Vec<PathBuf>> {
    let root = if let Ok(dir) = std::env::var("PALDEX_TEST_SAVE_DIR") {
        resolve_manual(std::path::Path::new(&dir))
            .unwrap_or_else(|e| panic!("PALDEX_TEST_SAVE_DIR is set but unusable: {e}"))
            .into_iter()
            .next()
    } else {
        discover().into_iter().next()
    }?;

    let world: World = root.worlds().into_iter().find(World::is_trackable)?;
    let player = world.players.first()?;

    let mut files: Vec<PathBuf> = ["Level.sav", "LevelMeta.sav", "LocalData.sav", "WorldOption.sav"]
        .iter()
        .map(|f| world.path.join(f))
        .collect();
    files.push(world.path.join("Players").join(format!("{}.sav", player.0)));
    files.push(
        world
            .path
            .join("Players")
            .join(format!("{}_dps.sav", player.0)),
    );
    Some(files)
}

macro_rules! require_real_files {
    () => {
        match hosted_world_files() {
            Some(files) => files,
            None => {
                eprintln!("skipping: no trackable Palworld world found (set PALDEX_TEST_SAVE_DIR)");
                return;
            }
        }
    };
}

#[test]
fn every_real_save_file_decompresses_to_gvas() {
    let files = require_real_files!();
    assert_eq!(files.len(), 6, "expected the six canonical per-world files");

    for path in &files {
        let raw = std::fs::read(path).unwrap_or_else(|e| panic!("reading {path:?}: {e}"));
        let (gvas, compression) = paldex_sav::decompress(&raw)
            .unwrap_or_else(|e| panic!("decompressing {path:?}: {e}"));

        assert!(
            gvas.starts_with(b"GVAS"),
            "{path:?} did not decompress to a GVAS payload"
        );
        // Verified fact: every real save on this machine uses Oodle (`PlM`), not zlib.
        assert_eq!(
            compression,
            Compression::Oodle,
            "{path:?} used an unexpected compression scheme"
        );
    }
}

#[test]
fn declared_uncompressed_length_matches_actual_output_for_every_fixture() {
    let files = require_real_files!();

    for path in &files {
        let raw = std::fs::read(path).unwrap();
        let header = paldex_sav::Header::parse(&raw, paldex_sav::DEFAULT_MAX_UNCOMPRESSED_LEN)
            .unwrap_or_else(|e| panic!("parsing header of {path:?}: {e}"));
        let (gvas, _) = paldex_sav::decompress(&raw).unwrap();

        assert_eq!(
            gvas.len(),
            header.uncompressed_len as usize,
            "{path:?} declared vs. actual decompressed length mismatch"
        );
    }
}

#[test]
fn the_dps_file_is_the_extreme_decompression_ratio_case() {
    let files = require_real_files!();
    let dps_path = files
        .iter()
        .find(|p| p.to_string_lossy().ends_with("_dps.sav"))
        .expect("a _dps.sav path");

    let raw = std::fs::read(dps_path).unwrap();
    let (gvas, _) = paldex_sav::decompress(&raw).unwrap();

    // Check high-ratio decompression without pinning mutable box contents.
    assert!(
        gvas.len() > raw.len() * 100,
        "_dps.sav ({} bytes) did not expand into the expected decompression-bomb shape ({} bytes)",
        raw.len(),
        gvas.len()
    );
}
