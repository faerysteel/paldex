//! Discovery of Palworld save directories.
//!
//! Palworld stores saves under a fixed tail:
//!
//! ```text
//! …/AppData/Local/Pal/Saved/SaveGames/<steamid64>/<worldid>/
//! ```
//!
//! On Windows that tail sits directly under `%LOCALAPPDATA%`. On macOS the game runs
//! through a Wine prefix — CrossOver or Whisky — so the same tail lives inside a
//! bottle's `drive_c/users/<winuser>/`. Bottle names and Windows usernames are both
//! user-chosen, so every level of that path has to be enumerated rather than assumed.

mod world;

pub use world::{PlayerUid, World, WorldKind};

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Path components between a Wine prefix user directory (or `%LOCALAPPDATA%`'s parent)
/// and the directory holding per-Steam-ID save folders.
const SAVE_TAIL: [&str; 5] = ["AppData", "Local", "Pal", "Saved", "SaveGames"];

/// Where a discovered set of saves came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SaveSource {
    /// Native Windows install, found under `%LOCALAPPDATA%`.
    WindowsSteam,
    /// CrossOver bottle on macOS.
    CrossOver { bottle: String },
    /// Whisky bottle on macOS.
    Whisky { bottle: String },
    /// Chosen by the user through the folder picker.
    Manual,
}

impl fmt::Display for SaveSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WindowsSteam => write!(f, "Steam (Windows)"),
            Self::CrossOver { bottle } => write!(f, "CrossOver — {bottle}"),
            Self::Whisky { bottle } => write!(f, "Whisky — {bottle}"),
            Self::Manual => write!(f, "Chosen manually"),
        }
    }
}

/// A `SaveGames/<steamid64>` directory, holding one or more worlds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRoot {
    /// Absolute path to the per-Steam-ID directory.
    pub path: PathBuf,
    pub source: SaveSource,
    /// Directory name of the per-Steam-ID folder. Numeric for auto-discovered roots.
    pub steam_id: String,
}

impl SaveRoot {
    /// Enumerate the worlds inside this root, newest first.
    #[must_use]
    pub fn worlds(&self) -> Vec<World> {
        world::enumerate(&self.path)
    }
}

/// Why a user-picked directory could not be resolved to a save root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "path", rename_all = "camelCase")]
pub enum LocateError {
    /// The path does not exist or is not a directory.
    NotADirectory(PathBuf),
    /// The directory exists but contains nothing that looks like a Palworld save.
    NoSavesFound(PathBuf),
}

impl fmt::Display for LocateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotADirectory(p) => write!(f, "not a directory: {}", p.display()),
            Self::NoSavesFound(p) => {
                write!(f, "no Palworld saves found in {}", p.display())
            }
        }
    }
}

impl std::error::Error for LocateError {}

/// Scan every known save location for the current platform.
///
/// Never fails: locations that are absent or unreadable are skipped, since a machine
/// with no CrossOver (or no Windows Steam) is the normal case rather than an error.
#[must_use]
pub fn discover() -> Vec<SaveRoot> {
    let mut roots = Vec::new();

    for (bottles_dir, whisky) in wine_bottle_dirs() {
        roots.extend(scan_bottles(&bottles_dir, whisky));
    }
    roots.extend(scan_windows_local_appdata());

    roots.sort_by(|a, b| a.path.cmp(&b.path));
    roots.dedup_by(|a, b| a.path == b.path);
    roots
}

/// Path from a Steam library root down to the game pak.
const PAK_TAIL: &[&str] = &[
    "steamapps",
    "common",
    "Palworld",
    "Pal",
    "Content",
    "Paks",
    "Pal-Windows.pak",
];

/// Locate the installed game pak, which holds the reference data (species
/// names, skills, technologies, items).
///
/// Returns every candidate, newest-looking first is not meaningful here — the
/// caller should just take the first that opens. Never fails: a machine
/// without the game installed is a normal case, and the app must still run
/// without reference data.
#[must_use]
pub fn discover_paks() -> Vec<PathBuf> {
    let mut paks = Vec::new();

    if cfg!(target_os = "macos") {
        for (bottles_dir, _) in wine_bottle_dirs() {
            for bottle in subdirs(&bottles_dir) {
                let drive_c = bottle.join("drive_c");
                paks.extend(scan_steam_libraries(&drive_c.join("Program Files (x86)/Steam")));
                paks.extend(scan_steam_libraries(&drive_c.join("Program Files/Steam")));
            }
        }
    }

    if cfg!(target_os = "windows") {
        for root in ["C:/Program Files (x86)/Steam", "C:/Program Files/Steam"] {
            paks.extend(scan_steam_libraries(Path::new(root)));
        }
    }

    paks.sort();
    paks.dedup();
    paks
}

/// Check a Steam install root for the pak.
fn scan_steam_libraries(steam_root: &Path) -> Vec<PathBuf> {
    let candidate = PAK_TAIL.iter().fold(steam_root.to_path_buf(), |p, c| p.join(c));
    if candidate.is_file() {
        vec![candidate]
    } else {
        Vec::new()
    }
}

/// Container directories that hold Wine bottles, paired with whether they are Whisky.
///
/// Returns nothing off macOS.
fn wine_bottle_dirs() -> Vec<(PathBuf, bool)> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    vec![
        (
            home.join("Library/Application Support/CrossOver/Bottles"),
            false,
        ),
        (
            home.join("Library/Containers/com.isaacmarovitz.Whisky/Bottles"),
            true,
        ),
    ]
}

/// Scan a directory of Wine bottles for Palworld saves.
///
/// Every bottle is checked, and within each bottle every `drive_c/users/<name>`
/// directory — neither the bottle name nor the Windows username is fixed.
#[must_use]
pub fn scan_bottles(bottles_dir: &Path, whisky: bool) -> Vec<SaveRoot> {
    let mut roots = Vec::new();
    for bottle in subdirs(bottles_dir) {
        let Some(name) = dir_name(&bottle) else {
            continue;
        };
        let source = if whisky {
            SaveSource::Whisky {
                bottle: name.clone(),
            }
        } else {
            SaveSource::CrossOver { bottle: name }
        };
        for user_dir in subdirs(&bottle.join("drive_c").join("users")) {
            let save_games = join_tail(&user_dir);
            roots.extend(scan_save_games(&save_games, &source));
        }
    }
    roots
}

/// Scan `%LOCALAPPDATA%\Pal\Saved\SaveGames`. Returns nothing off Windows.
fn scan_windows_local_appdata() -> Vec<SaveRoot> {
    if !cfg!(target_os = "windows") {
        return Vec::new();
    }
    let Some(local) = dirs::data_local_dir() else {
        return Vec::new();
    };
    let save_games = local.join("Pal").join("Saved").join("SaveGames");
    scan_save_games(&save_games, &SaveSource::WindowsSteam)
}

/// Collect the per-Steam-ID roots inside a `SaveGames` directory.
///
/// Only all-numeric directory names are accepted, which filters out the loose
/// `UserOption.sav` and any stray folders that sit alongside them.
#[must_use]
pub fn scan_save_games(save_games: &Path, source: &SaveSource) -> Vec<SaveRoot> {
    subdirs(save_games)
        .into_iter()
        .filter_map(|dir| {
            let name = dir_name(&dir)?;
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            Some(SaveRoot {
                path: dir,
                source: source.clone(),
                steam_id: name,
            })
        })
        .collect()
}

/// Resolve a user-picked directory into save roots.
///
/// Deliberately forgiving about what the user selected: the `SaveGames` directory, a
/// per-Steam-ID directory, or a single world directory all resolve, because it is not
/// obvious from the outside which level is the "right" one to pick.
///
/// # Errors
///
/// Returns [`LocateError::NotADirectory`] if the path is not a directory, or
/// [`LocateError::NoSavesFound`] if nothing under it looks like a Palworld save.
pub fn resolve_manual(path: &Path) -> Result<Vec<SaveRoot>, LocateError> {
    if !path.is_dir() {
        return Err(LocateError::NotADirectory(path.to_path_buf()));
    }

    // A world directory: step up to its per-Steam-ID parent.
    if world::classify(path) != WorldKind::Unknown {
        if let Some(parent) = path.parent() {
            return Ok(vec![manual_root_at(parent)]);
        }
    }

    // A per-Steam-ID directory: at least one child looks like a world.
    if subdirs(path)
        .iter()
        .any(|d| world::classify(d) != WorldKind::Unknown)
    {
        return Ok(vec![manual_root_at(path)]);
    }

    // A `SaveGames` directory, or something above it that still contains one.
    let mut roots = scan_save_games(path, &SaveSource::Manual);
    if roots.is_empty() {
        let nested = join_tail(path);
        roots = scan_save_games(&nested, &SaveSource::Manual);
    }
    if roots.is_empty() {
        return Err(LocateError::NoSavesFound(path.to_path_buf()));
    }
    Ok(roots)
}

fn manual_root_at(path: &Path) -> SaveRoot {
    SaveRoot {
        path: path.to_path_buf(),
        source: SaveSource::Manual,
        steam_id: dir_name(path).unwrap_or_default(),
    }
}

fn join_tail(base: &Path) -> PathBuf {
    SAVE_TAIL.iter().fold(base.to_path_buf(), |p, c| p.join(c))
}

/// Immediate subdirectories of `dir`, or empty if it is missing or unreadable.
///
/// Symlinks are followed, since bottles and game installs are often linked rather
/// than copied.
fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

fn dir_name(path: &Path) -> Option<String> {
    Some(path.file_name()?.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pak_tail_matches_the_documented_install_layout() {
        let joined = PAK_TAIL.iter().fold(PathBuf::from("/steam"), |p, c| p.join(c));
        assert!(joined.ends_with("steamapps/common/Palworld/Pal/Content/Paks/Pal-Windows.pak"));
    }

    #[test]
    fn pak_discovery_skips_missing_installs() {
        assert!(scan_steam_libraries(Path::new("/nonexistent/steam")).is_empty());
    }

    #[test]
    fn save_tail_matches_the_documented_layout() {
        let joined = join_tail(Path::new("/base"));
        assert!(joined.ends_with("AppData/Local/Pal/Saved/SaveGames"));
    }

    #[test]
    fn scan_save_games_keeps_numeric_dirs_and_drops_everything_else() {
        let tmp = tempdir();
        let save_games = tmp.join("SaveGames");
        fs::create_dir_all(save_games.join("12345678901234567")).unwrap();
        fs::create_dir_all(save_games.join("not-a-steam-id")).unwrap();
        fs::write(save_games.join("UserOption.sav"), b"x").unwrap();

        let roots = scan_save_games(&save_games, &SaveSource::WindowsSteam);

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].steam_id, "12345678901234567");
        cleanup(&tmp);
    }

    #[test]
    fn scan_bottles_walks_bottle_and_windows_user_names() {
        let tmp = tempdir();
        let bottles = tmp.join("Bottles");
        // Neither the bottle name nor the Windows user name is fixed in practice.
        let save_games = bottles
            .join("My Games Bottle")
            .join("drive_c/users/someone")
            .join("AppData/Local/Pal/Saved/SaveGames");
        fs::create_dir_all(save_games.join("12345678901234567")).unwrap();

        let roots = scan_bottles(&bottles, false);

        assert_eq!(roots.len(), 1);
        assert_eq!(
            roots[0].source,
            SaveSource::CrossOver {
                bottle: "My Games Bottle".to_owned()
            }
        );
        cleanup(&tmp);
    }

    #[test]
    fn scan_bottles_marks_whisky_separately() {
        let tmp = tempdir();
        let bottles = tmp.join("Bottles");
        let save_games = bottles
            .join("b1")
            .join("drive_c/users/crossover")
            .join("AppData/Local/Pal/Saved/SaveGames");
        fs::create_dir_all(save_games.join("1")).unwrap();

        let roots = scan_bottles(&bottles, true);

        assert_eq!(
            roots[0].source,
            SaveSource::Whisky {
                bottle: "b1".to_owned()
            }
        );
        cleanup(&tmp);
    }

    #[test]
    fn missing_and_unreadable_directories_are_not_errors() {
        assert!(scan_bottles(Path::new("/nope/does/not/exist"), false).is_empty());
        assert!(scan_save_games(Path::new("/nope"), &SaveSource::Manual).is_empty());
        assert!(subdirs(Path::new("/nope")).is_empty());
    }

    #[test]
    fn resolve_manual_accepts_a_world_directory() {
        let tmp = tempdir();
        let world_dir = tmp.join("12345678901234567").join("ABC123");
        fs::create_dir_all(&world_dir).unwrap();
        fs::write(world_dir.join("Level.sav"), b"x").unwrap();

        let roots = resolve_manual(&world_dir).unwrap();

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].steam_id, "12345678901234567");
        assert_eq!(roots[0].source, SaveSource::Manual);
        cleanup(&tmp);
    }

    #[test]
    fn resolve_manual_accepts_a_steam_id_directory() {
        let tmp = tempdir();
        let steam_dir = tmp.join("12345678901234567");
        fs::create_dir_all(steam_dir.join("ABC123")).unwrap();
        fs::write(steam_dir.join("ABC123").join("LocalData.sav"), b"x").unwrap();

        let roots = resolve_manual(&steam_dir).unwrap();

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].path, steam_dir);
        cleanup(&tmp);
    }

    #[test]
    fn resolve_manual_accepts_a_save_games_directory() {
        let tmp = tempdir();
        let save_games = tmp.join("SaveGames");
        let world = save_games.join("12345678901234567").join("ABC123");
        fs::create_dir_all(&world).unwrap();
        fs::write(world.join("Level.sav"), b"x").unwrap();

        let roots = resolve_manual(&save_games).unwrap();

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].steam_id, "12345678901234567");
        cleanup(&tmp);
    }

    #[test]
    fn resolve_manual_reports_a_directory_with_no_saves() {
        let tmp = tempdir();
        fs::create_dir_all(tmp.join("empty")).unwrap();

        let err = resolve_manual(&tmp.join("empty")).unwrap_err();

        assert!(matches!(err, LocateError::NoSavesFound(_)));
        cleanup(&tmp);
    }

    #[test]
    fn resolve_manual_rejects_a_missing_path() {
        let err = resolve_manual(Path::new("/nope/does/not/exist")).unwrap_err();
        assert!(matches!(err, LocateError::NotADirectory(_)));
    }

    #[test]
    fn discover_never_panics_on_this_machine() {
        let _ = discover();
    }

    // --- helpers -------------------------------------------------------------

    pub(super) fn tempdir() -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("paldex-locate-{n}-{:?}", std::thread::current().id()));
        fs::create_dir_all(&p).unwrap();
        p
    }

    pub(super) fn cleanup(p: &Path) {
        let _ = fs::remove_dir_all(p);
    }
}
