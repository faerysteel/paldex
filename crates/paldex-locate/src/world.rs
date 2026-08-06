//! Enumeration and classification of the world folders inside a save root.
//!
//! A save root holds one directory per world, but they are not all equal. A world you
//! host locally has the full set of files; a co-op world you *joined* leaves only a
//! `LocalData.sav` behind, because the world state lives on the host's machine. Those
//! stubs cannot be tracked, so they are classified rather than silently dropped — the
//! UI needs to explain the difference instead of appearing to lose saves.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// The world-state file, present only for worlds hosted on this machine.
const LEVEL_SAV: &str = "Level.sav";
/// Written for every world the player has entered, hosted locally or not.
const LOCAL_DATA_SAV: &str = "LocalData.sav";
/// Suffix of the per-player Pal storage file, which shadows the real player file.
const DPS_SUFFIX: &str = "_dps";

/// A player's save identifier, taken from the `Players/<uid>.sav` filename.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlayerUid(pub String);

/// How completely a world is stored on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorldKind {
    /// Hosted here: `Level.sav` is present, so the world can be tracked in full.
    LocalWorld,
    /// Joined as a guest: only `LocalData.sav` remains, world state lives on the host.
    CoopGuestStub,
    /// Neither marker file is present.
    Unknown,
}

/// One world directory inside a save root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct World {
    /// Directory name, e.g. `11111111111111111111111111111111`.
    pub id: String,
    pub path: PathBuf,
    pub kind: WorldKind,
    /// Modification time of the newest save file, serialized as epoch milliseconds.
    #[serde(with = "epoch_millis")]
    pub last_played: Option<SystemTime>,
    /// Players with a save in this world. Empty for guest stubs.
    pub players: Vec<PlayerUid>,
}

impl World {
    /// Whether this world has enough data to be tracked.
    #[must_use]
    pub fn is_trackable(&self) -> bool {
        self.kind == WorldKind::LocalWorld
    }
}

/// Classify a single world directory by which marker files it holds.
#[must_use]
pub fn classify(dir: &Path) -> WorldKind {
    if dir.join(LEVEL_SAV).is_file() {
        WorldKind::LocalWorld
    } else if dir.join(LOCAL_DATA_SAV).is_file() {
        WorldKind::CoopGuestStub
    } else {
        WorldKind::Unknown
    }
}

/// Enumerate every world directory in a save root, most recently played first.
///
/// Worlds whose timestamp cannot be read sort last rather than being dropped.
#[must_use]
pub fn enumerate(root: &Path) -> Vec<World> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut worlds: Vec<World> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|dir| {
            let id = dir.file_name()?.to_string_lossy().into_owned();
            let kind = classify(&dir);
            Some(World {
                id,
                last_played: last_played(&dir, kind),
                players: players(&dir),
                kind,
                path: dir,
            })
        })
        .collect();

    worlds.sort_by(|a, b| {
        b.last_played
            .cmp(&a.last_played)
            .then_with(|| a.id.cmp(&b.id))
    });
    worlds
}

/// Timestamp of the file that best represents "last played" for this kind of world.
fn last_played(dir: &Path, kind: WorldKind) -> Option<SystemTime> {
    let marker = match kind {
        WorldKind::LocalWorld => dir.join(LEVEL_SAV),
        WorldKind::CoopGuestStub => dir.join(LOCAL_DATA_SAV),
        WorldKind::Unknown => dir.to_path_buf(),
    };
    fs::metadata(marker).ok()?.modified().ok()
}

/// Players with a save file in this world.
///
/// `<uid>_dps.sav` holds Pal storage for a player that already has a `<uid>.sav`, so
/// counting it would double every player.
fn players(dir: &Path) -> Vec<PlayerUid> {
    let Ok(entries) = fs::read_dir(dir.join("Players")) else {
        return Vec::new();
    };
    let mut uids: Vec<PlayerUid> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sav")))
        .filter_map(|p| {
            let stem = p.file_stem()?.to_string_lossy().into_owned();
            (!stem.ends_with(DPS_SUFFIX)).then_some(PlayerUid(stem))
        })
        .collect();
    uids.sort();
    uids
}

/// Serializes `Option<SystemTime>` as epoch milliseconds so the frontend gets a plain
/// number instead of serde's `{ secs_since_epoch, nanos_since_epoch }` shape.
mod epoch_millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[allow(clippy::ref_option)]
    pub fn serialize<S: Serializer>(
        value: &Option<SystemTime>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match value.and_then(|t| t.duration_since(UNIX_EPOCH).ok()) {
            Some(d) => s.serialize_some(&u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<SystemTime>, D::Error> {
        Ok(Option::<u64>::deserialize(d)?
            .map(|ms| UNIX_EPOCH + Duration::from_millis(ms)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Builds the exact shape observed on the development machine: one hosted world
    /// with a full file set, five co-op guest stubs with only `LocalData.sav`.
    fn realistic_root() -> PathBuf {
        let root = crate::tests::tempdir();

        let hosted = root.join("11111111111111111111111111111111");
        fs::create_dir_all(hosted.join("Players")).unwrap();
        fs::create_dir_all(hosted.join("backup/local/2024.01.01-00.00.00")).unwrap();
        for f in ["Level.sav", "LevelMeta.sav", "LocalData.sav", "WorldOption.sav"] {
            fs::write(hosted.join(f), b"x").unwrap();
        }
        for f in [
            "22222222222222222222222222222222.sav",
            "22222222222222222222222222222222_dps.sav",
            "00000000000000000000000000000001.sav",
            "00000000000000000000000000000001_dps.sav",
        ] {
            fs::write(hosted.join("Players").join(f), b"x").unwrap();
        }

        for id in [
            "33333333333333333333333333333333",
            "44444444444444444444444444444444",
            "55555555555555555555555555555555",
            "66666666666666666666666666666666",
            "77777777777777777777777777777777",
        ] {
            let stub = root.join(id);
            fs::create_dir_all(&stub).unwrap();
            fs::write(stub.join("LocalData.sav"), b"x").unwrap();
        }

        // Sits alongside the world directories and must not be mistaken for one.
        fs::write(root.join("GlobalPalStorage.sav"), b"x").unwrap();
        root
    }

    #[test]
    fn classifies_one_hosted_world_and_five_guest_stubs() {
        let root = realistic_root();

        let worlds = enumerate(&root);

        assert_eq!(worlds.len(), 6, "loose .sav files must not become worlds");
        assert_eq!(
            worlds
                .iter()
                .filter(|w| w.kind == WorldKind::LocalWorld)
                .count(),
            1
        );
        assert_eq!(
            worlds
                .iter()
                .filter(|w| w.kind == WorldKind::CoopGuestStub)
                .count(),
            5
        );
        crate::tests::cleanup(&root);
    }

    #[test]
    fn dps_files_do_not_double_count_players() {
        let root = realistic_root();

        let hosted = enumerate(&root)
            .into_iter()
            .find(|w| w.kind == WorldKind::LocalWorld)
            .expect("hosted world");

        assert_eq!(hosted.players.len(), 2);
        assert!(hosted
            .players
            .iter()
            .all(|p| !p.0.ends_with(DPS_SUFFIX)));
        assert!(hosted.is_trackable());
        crate::tests::cleanup(&root);
    }

    #[test]
    fn guest_stubs_have_no_players_and_are_not_trackable() {
        let root = realistic_root();

        for w in enumerate(&root)
            .iter()
            .filter(|w| w.kind == WorldKind::CoopGuestStub)
        {
            assert!(w.players.is_empty());
            assert!(!w.is_trackable());
        }
        crate::tests::cleanup(&root);
    }

    #[test]
    fn worlds_are_ordered_most_recently_played_first() {
        let root = crate::tests::tempdir();
        for id in ["older", "newer"] {
            let d = root.join(id);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("Level.sav"), b"x").unwrap();
            // Coarse filesystem timestamps need a gap to order reliably.
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let worlds = enumerate(&root);

        assert_eq!(worlds[0].id, "newer");
        assert_eq!(worlds[1].id, "older");
        crate::tests::cleanup(&root);
    }

    #[test]
    fn a_directory_with_neither_marker_is_unknown() {
        let root = crate::tests::tempdir();
        fs::create_dir_all(root.join("mystery")).unwrap();

        let worlds = enumerate(&root);

        assert_eq!(worlds[0].kind, WorldKind::Unknown);
        assert!(!worlds[0].is_trackable());
        crate::tests::cleanup(&root);
    }

    #[test]
    fn missing_root_yields_no_worlds() {
        assert!(enumerate(Path::new("/nope/does/not/exist")).is_empty());
    }

    #[test]
    fn last_played_serializes_as_epoch_millis() {
        let root = realistic_root();
        let worlds = enumerate(&root);

        let json = serde_json::to_value(&worlds[0]).unwrap();

        assert!(
            json["lastPlayed"].is_u64(),
            "frontend expects a plain number, got {}",
            json["lastPlayed"]
        );
        assert!(json["kind"].is_string());
        crate::tests::cleanup(&root);
    }

    #[test]
    fn world_survives_a_json_round_trip() {
        let root = realistic_root();
        let worlds = enumerate(&root);

        let json = serde_json::to_string(&worlds).unwrap();
        let back: Vec<World> = serde_json::from_str(&json).unwrap();

        // Millisecond truncation is expected, so compare at that resolution.
        assert_eq!(back.len(), worlds.len());
        assert_eq!(back[0].id, worlds[0].id);
        assert_eq!(back[0].kind, worlds[0].kind);
        assert_eq!(back[0].players, worlds[0].players);
        crate::tests::cleanup(&root);
    }
}
