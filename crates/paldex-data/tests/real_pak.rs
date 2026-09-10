//! Integration tests against the real, locally-installed Palworld game pak.
//!
//! Resolves the pak path via `PALDEX_TEST_PAK` if set, otherwise the known
//! Steam-on-CrossOver install layout on this machine. Skips (not fails) if
//! neither is found, so CI and machines without the game installed stay
//! green — the pak is 40+ GB and never something to fixture-commit.

use std::path::PathBuf;

fn real_pak_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PALDEX_TEST_PAK") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let candidate = PathBuf::from(std::env::var("HOME").ok()?)
        .join("Library/Application Support/CrossOver/Bottles/Steam/drive_c/Program Files (x86)/Steam/steamapps/common/Palworld/Pal/Content/Paks/Pal-Windows.pak");
    candidate.is_file().then_some(candidate)
}

macro_rules! require_real_pak {
    () => {
        match real_pak_path() {
            Some(p) => p,
            None => {
                eprintln!("skipping: no Palworld game pak found (set PALDEX_TEST_PAK)");
                return;
            }
        }
    };
}

#[test]
fn opens_the_real_pak_and_matches_verified_header_facts() {
    let path = require_real_pak!();
    let pak = paldex_data::Pak::open(&path).expect("open pak");

    assert_eq!(pak.mount_point(), "../../../");
    assert!(!pak.encrypted_index());
    // Allow content updates around the 185,003-entry reference pak while
    // rejecting implausible counts from a misparsed index.
    let count = pak.files().len();
    assert!(
        (150_000..250_000).contains(&count),
        "entry count {count} far outside the expected range"
    );
}

#[test]
fn reads_and_decompresses_the_monster_parameter_table() {
    let path = require_real_pak!();
    let mut pak = paldex_data::Pak::open(&path).expect("open pak");

    let target = pak
        .files()
        .into_iter()
        .find(|f| f.ends_with("DT_PalMonsterParameter.uasset"))
        .expect("DT_PalMonsterParameter.uasset should exist in the pak");

    let bytes = pak.read(&target).expect("read + decompress entry");
    assert!(!bytes.is_empty());
    // uasset files start with this tag (0x9E2A83C1) regardless of endianness
    // convention used to print it — verified directly against the real file.
    assert_eq!(&bytes[0..4], &[0xC1, 0x83, 0x2A, 0x9E]);
}

#[test]
fn every_entry_path_is_readable() {
    let path = require_real_pak!();
    let mut pak = paldex_data::Pak::open(&path).expect("open pak");

    // Reading every one of 185k entries would be slow; sample instead, but
    // deterministically (every Nth path) so a real regression is still caught.
    let files = pak.files();
    let mut failures = Vec::new();
    for f in files.iter().step_by(211) {
        if let Err(e) = pak.read(f) {
            failures.push(format!("{f}: {e}"));
        }
    }
    assert!(failures.is_empty(), "failed to read entries:\n{}", failures.join("\n"));
}
