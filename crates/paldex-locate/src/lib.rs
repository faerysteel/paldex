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

    for root in steam_install_roots() {
        paks.extend(scan_steam_libraries(&root));
    }

    paks.sort();
    paks.dedup();
    paks
}

/// Where Steam itself might be installed.
///
/// Returns nothing off Windows: the macOS branch of [`discover_paks`] reaches
/// Steam through a Wine bottle instead, where the prefix supplies the root.
///
/// The registry is the authority — Steam writes down where it actually landed —
/// and the `C:` defaults are only a fallback for a machine where the key is
/// missing or unreadable. Assuming `C:` was the original bug: it happened to be
/// right on the test VM, so the failure looked like a library problem rather
/// than a root problem.
fn steam_install_roots() -> Vec<PathBuf> {
    if !cfg!(target_os = "windows") {
        return Vec::new();
    }
    let mut roots = registry_steam_roots();
    roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    roots.push(PathBuf::from(r"C:\Program Files\Steam"));
    roots.sort();
    roots.dedup();
    roots
}

/// Steam's install path as recorded in the registry.
///
/// Three keys are consulted because which one is populated depends on who is
/// asking. `HKCU\...\SteamPath` is per-user, so it reads empty for a process
/// running as SYSTEM — observed directly on the test VM, where only the
/// `HKLM` 32-bit view had a value. Reading all three costs nothing and avoids
/// depending on the process's identity or bitness.
#[cfg(windows)]
fn registry_steam_roots() -> Vec<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    const KEYS: &[(isize, &str, &str)] = &[
        (HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Valve\Steam",
            "InstallPath",
        ),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"),
    ];

    KEYS.iter()
        .filter_map(|(hive, subkey, value)| {
            let raw: String = RegKey::predef(*hive).open_subkey(subkey).ok()?.get_value(value).ok()?;
            // Steam writes SteamPath with forward slashes and InstallPath with
            // backslashes; both are valid here, but an empty value is not.
            (!raw.trim().is_empty()).then(|| PathBuf::from(raw.trim()))
        })
        .collect()
}

#[cfg(not(windows))]
fn registry_steam_roots() -> Vec<PathBuf> {
    Vec::new()
}

/// Check every Steam library reachable from an install root for the pak.
fn scan_steam_libraries(steam_root: &Path) -> Vec<PathBuf> {
    steam_libraries(steam_root)
        .into_iter()
        .map(|lib| PAK_TAIL.iter().fold(lib, |p, c| p.join(c)))
        .filter(|candidate| candidate.is_file())
        .collect()
}

/// Every library root reachable from a Steam install: the install itself, plus
/// each `path` recorded in `steamapps/libraryfolders.vdf`.
///
/// Looking only inside `steam_root` finds Steam but routinely misses the game.
/// Steam lets a library live on any drive, and a 39 GB install is exactly the
/// thing people put on the roomy disk rather than the boot one — the observed
/// case being Steam on `C:` with Palworld in `G:\SteamLibrary`. That layout is
/// recorded *only* in `libraryfolders.vdf`, so without reading it the pak is
/// invisible and the app silently drops to no species list and no artwork.
fn steam_libraries(steam_root: &Path) -> Vec<PathBuf> {
    let mut libraries = vec![steam_root.to_path_buf()];
    let manifest = steam_root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = fs::read_to_string(&manifest) {
        libraries.extend(parse_library_paths(&text));
    }
    libraries.sort();
    libraries.dedup();
    libraries
}

/// Pull the library paths out of a `libraryfolders.vdf`.
///
/// A real VDF parser is not warranted for this: the file is machine-written
/// with one `"path"  "<value>"` pair per library, and the only escaping that
/// appears in it is the doubled backslash of a Windows path.
fn parse_library_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| {
            // Splitting on the quote character yields the leading indent, the
            // key, the gap between key and value, then the value.
            let mut fields = line.split('"').skip(1);
            (fields.next()? == "path").then_some(())?;
            let value = fields.nth(1)?;
            Some(PathBuf::from(value.replace("\\\\", "\\")))
        })
        .collect()
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

    /// Verbatim from the Windows test VM, where Steam sits on `C:` and Palworld
    /// on `G:` — the layout that left the packaged build with no artwork.
    const REAL_VDF: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"apps"
		{
			"228980"		"128606066"
		}
	}
	"1"
	{
		"path"		"G:\\SteamLibrary"
		"label"		""
		"apps"
		{
			"1623730"		"41175628606"
		}
	}
}
"#;

    #[test]
    fn library_paths_come_from_the_vdf_with_backslashes_unescaped() {
        let paths = parse_library_paths(REAL_VDF);
        assert_eq!(
            paths,
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"G:\SteamLibrary"),
            ]
        );
    }

    #[test]
    fn library_paths_ignore_every_other_key() {
        // `label` and the app-id lines are also quoted pairs, so a parser that
        // just grabbed quoted values would pull them in as paths.
        for path in parse_library_paths(REAL_VDF) {
            let text = path.to_string_lossy().into_owned();
            assert!(text.contains("Steam"), "not a library path: {text}");
        }
    }

    #[test]
    fn steam_libraries_finds_a_pak_outside_the_steam_root() {
        let tmp = tempdir();
        let steam = tmp.join("Steam");
        let other = tmp.join("SecondDrive");
        fs::create_dir_all(steam.join("steamapps")).unwrap();
        fs::write(
            steam.join("steamapps").join("libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                other.display()
            ),
        )
        .unwrap();

        let pak = PAK_TAIL.iter().fold(other.clone(), |p, c| p.join(c));
        fs::create_dir_all(pak.parent().unwrap()).unwrap();
        fs::write(&pak, b"not really a pak").unwrap();

        assert_eq!(scan_steam_libraries(&steam), vec![pak]);
        cleanup(&tmp);
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
