//! Commands exposed to the frontend.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use paldex_data::{Pak, ReferenceData, ReferenceIndex};
use paldex_locate::{discover, discover_paks, resolve_manual, SaveRoot, World};
use paldex_store::Store;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::queries::{self, BaseCampView, DexProgressView, PalView, PlayerProgressView, SnapshotSummaryView};
use crate::sync;

/// Holds the app-wide SQLite store and which world is currently selected.
/// One store file backs every world (partitioned by `world_id` internally),
/// opened lazily on first use rather than at startup.
#[derive(Default)]
pub struct AppState {
    store: Mutex<Option<Store>>,
    selected: Mutex<Option<SelectedWorld>>,
    /// Reference data from the installed game pak, loaded once on first use.
    /// The inner `None` means no pak was found — the app stays usable without
    /// the game installed, just with internal ids instead of display names.
    reference: OnceLock<Option<LoadedReference>>,
}

/// The extracted reference index plus the pak it came from.
///
/// The pak is kept open because icons are decoded lazily: re-opening it per
/// request would re-parse a 185,000-entry index (~65 ms) every time.
struct LoadedReference {
    index: ReferenceIndex,
    pak: Mutex<Pak>,
    /// Species key -> PNG data URL, or `None` for a species with no artwork.
    /// Decoding is a block-decompress plus a PNG encode, so results are kept.
    icons: Mutex<HashMap<String, Option<String>>>,
}

impl AppState {
    fn loaded_reference(&self) -> Option<&LoadedReference> {
        self.reference.get_or_init(load_reference).as_ref()
    }

    fn reference(&self) -> Option<&ReferenceIndex> {
        self.loaded_reference().map(|r| &r.index)
    }
}

/// Open the first discoverable game pak and extract reference data from it.
///
/// Failure is never fatal: a missing or unreadable pak degrades the UI to raw
/// ids rather than breaking it.
fn load_reference() -> Option<LoadedReference> {
    for path in discover_paks() {
        let Ok(mut pak) = Pak::open(&path) else {
            continue;
        };
        match ReferenceIndex::extract(&mut pak, "en") {
            Ok(index) => {
                return Some(LoadedReference {
                    index,
                    pak: Mutex::new(pak),
                    icons: Mutex::new(HashMap::new()),
                })
            }
            Err(e) => eprintln!("reference extraction failed for {}: {e}", path.display()),
        }
    }
    None
}

struct SelectedWorld {
    world_id: String,
    world_path: PathBuf,
    latest_snapshot_id: i64,
}

fn with_store<T>(
    app: &AppHandle,
    state: &State<AppState>,
    f: impl FnOnce(&mut Store) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = state.store.lock().map_err(|_| "store lock poisoned".to_owned())?;
    if guard.is_none() {
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("resolving app data directory: {e}"))?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("creating app data directory: {e}"))?;
        let store = Store::open(&dir.join("paldex.sqlite3")).map_err(|e| e.to_string())?;
        *guard = Some(store);
    }
    f(guard.as_mut().expect("just initialized above"))
}

fn selected_world_id(state: &State<AppState>) -> Result<String, String> {
    state
        .selected
        .lock()
        .map_err(|_| "selection lock poisoned".to_owned())?
        .as_ref()
        .map(|s| s.world_id.clone())
        .ok_or_else(|| "no world selected — call select_world first".to_owned())
}

fn selected_snapshot_id(state: &State<AppState>) -> Result<i64, String> {
    state
        .selected
        .lock()
        .map_err(|_| "selection lock poisoned".to_owned())?
        .as_ref()
        .map(|s| s.latest_snapshot_id)
        .ok_or_else(|| "no world selected — call select_world first".to_owned())
}

/// A save root plus everything the UI needs to render it in one payload, so the
/// frontend never has to make a second round trip to find out what is inside.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRootView {
    #[serde(flatten)]
    root: SaveRoot,
    /// Human-readable rendering of the source, e.g. `CrossOver — Steam`.
    source_label: String,
    worlds: Vec<World>,
}

impl From<SaveRoot> for SaveRootView {
    fn from(root: SaveRoot) -> Self {
        Self {
            source_label: root.source.to_string(),
            worlds: root.worlds(),
            root,
        }
    }
}

/// Scan every known save location for this platform.
#[tauri::command]
pub fn list_saves() -> Vec<SaveRootView> {
    discover().into_iter().map(SaveRootView::from).collect()
}

/// Resolve a folder the user picked, for when auto-discovery comes up empty.
///
/// # Errors
///
/// Returns a display-ready message if the folder is missing or holds no saves.
#[tauri::command]
pub fn resolve_folder(path: PathBuf) -> Result<Vec<SaveRootView>, String> {
    resolve_manual(&path)
        .map(|roots| roots.into_iter().map(SaveRootView::from).collect())
        .map_err(|e| e.to_string())
}

/// Select a world (identified by its own directory, as returned in
/// `SaveRootView::worlds[].path`), decode it fresh from disk, and ingest a
/// snapshot. Every other command below acts on whichever world was most
/// recently selected.
///
/// # Errors
///
/// A display-ready message if the world can't be re-resolved, its save
/// files can't be read/decompressed/parsed, or the store can't be opened.
#[tauri::command]
pub fn select_world(
    app: AppHandle,
    state: State<AppState>,
    world_path: PathBuf,
) -> Result<SnapshotSummaryView, String> {
    let world = sync::find_world_by_path(&world_path)
        .ok_or_else(|| format!("world not found at {}", world_path.display()))?;
    if !world.is_trackable() {
        return Err("this world has no local save data to track (a co-op guest stub)".to_owned());
    }

    let snapshot_id = with_store(&app, &state, |store| {
        sync::sync_world(store, &world).map_err(|e| e.to_string())
    })?;

    *state.selected.lock().map_err(|_| "selection lock poisoned".to_owned())? = Some(SelectedWorld {
        world_id: world.id.clone(),
        world_path,
        latest_snapshot_id: snapshot_id,
    });

    with_store(&app, &state, |store| queries::snapshot_summary(store, snapshot_id))
}

/// Re-decode and re-ingest the currently selected world.
///
/// # Errors
///
/// A display-ready message if no world is selected yet, or the re-sync
/// itself fails the same way `select_world` can.
#[tauri::command]
pub fn force_resync(app: AppHandle, state: State<AppState>) -> Result<SnapshotSummaryView, String> {
    let world_path = state
        .selected
        .lock()
        .map_err(|_| "selection lock poisoned".to_owned())?
        .as_ref()
        .map(|s| s.world_path.clone())
        .ok_or_else(|| "no world selected — call select_world first".to_owned())?;

    let world = sync::find_world_by_path(&world_path)
        .ok_or_else(|| format!("world not found at {}", world_path.display()))?;

    let snapshot_id = with_store(&app, &state, |store| {
        sync::sync_world(store, &world).map_err(|e| e.to_string())
    })?;

    if let Some(selected) = state.selected.lock().map_err(|_| "selection lock poisoned".to_owned())?.as_mut() {
        selected.latest_snapshot_id = snapshot_id;
    }

    with_store(&app, &state, |store| queries::snapshot_summary(store, snapshot_id))
}

/// The currently selected world's Pal roster, with localized species names.
///
/// Human NPCs are excluded: the save gives them the same shape as Pals (they
/// carry full IV stats), so Phase 2 can't tell them apart and they would
/// otherwise show up as roster entries. The pak can — see `paldex-data`'s
/// reference docs. Without a pak nothing is filtered or renamed, which is the
/// pre-existing behaviour.
///
/// # Errors
///
/// A display-ready message if no world is selected, or the query fails.
#[tauri::command]
pub fn pal_roster(app: AppHandle, state: State<AppState>) -> Result<Vec<PalView>, String> {
    let snapshot_id = selected_snapshot_id(&state)?;
    let mut roster = with_store(&app, &state, |store| queries::pal_roster(store, snapshot_id))?;

    if let Some(reference) = state.reference() {
        enrich_roster(&mut roster, reference);
    }
    Ok(roster)
}

/// Drop human NPCs and attach localized species and passive names.
fn enrich_roster(roster: &mut Vec<PalView>, reference: &ReferenceIndex) {
    roster.retain(|pal| !reference.is_human_npc(&pal.character_id));
    for pal in roster.iter_mut() {
        pal.display_name = reference
            .species(&pal.character_id)
            .map(|s| s.display_name.clone());
        pal.passive_names = pal
            .passives
            .iter()
            .map(|id| resolve_or_id(reference.passive(id).map(|p| p.display_name.as_str()), id))
            .collect();
    }
}

/// Attach localized technology names.
fn enrich_flags(flags: &mut [queries::PlayerFlagsView], reference: &ReferenceIndex) {
    for player in flags.iter_mut() {
        player.unlocked_tech_names = player
            .unlocked_tech
            .iter()
            .map(|id| resolve_or_id(reference.technology(id), id))
            .collect();
    }
}

/// An unresolved id is shown as-is rather than blanked — a raw id is still
/// informative, an empty cell is not.
fn resolve_or_id(resolved: Option<&str>, id: &str) -> String {
    resolved.unwrap_or(id).to_owned()
}

/// Species artwork as `data:image/png;base64,…` URLs, keyed by the
/// `character_id` that was asked for.
///
/// Batched deliberately: a full roster spans a few hundred distinct species,
/// and one IPC round trip each would be far more expensive than the decoding.
/// Species with no artwork are simply absent from the result, so the UI falls
/// back to text.
///
/// # Errors
///
/// A display-ready message only if the icon cache lock is poisoned. A pak that
/// is missing, or an individual icon that fails to decode, yields fewer
/// entries rather than an error — artwork is never worth failing a screen over.
#[tauri::command]
pub fn pal_icons(
    state: State<AppState>,
    character_ids: Vec<String>,
) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    let Some(reference) = state.loaded_reference() else {
        return Ok(out);
    };

    let mut cache = reference.icons.lock().map_err(|_| "icon cache poisoned".to_owned())?;
    for id in character_ids {
        if !cache.contains_key(&id) {
            let decoded = reference.index.icon_path(&id).and_then(|path| {
                let path = path.to_owned();
                let mut pak = reference.pak.lock().ok()?;
                match paldex_data::load_icon_png(&mut pak, &path) {
                    Ok(png) => Some(png),
                    Err(e) => {
                        eprintln!("decoding icon for {id}: {e}");
                        None
                    }
                }
            });
            cache.insert(id.clone(), decoded.map(|png| to_png_data_url(&png)));
        }
        if let Some(Some(url)) = cache.get(&id) {
            out.insert(id, url.clone());
        }
    }
    Ok(out)
}

fn to_png_data_url(png: &[u8]) -> String {
    use base64::Engine as _;
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}

/// Whether pak-derived reference data is available, so the UI can explain why
/// species show as internal ids when it isn't.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceStatusView {
    pub available: bool,
    pub language: Option<String>,
    pub species_count: usize,
}

#[tauri::command]
pub fn reference_status(state: State<AppState>) -> ReferenceStatusView {
    match state.reference() {
        Some(index) => ReferenceStatusView {
            available: true,
            language: Some(index.language().to_owned()),
            species_count: index.species_count(),
        },
        None => ReferenceStatusView { available: false, language: None, species_count: 0 },
    }
}

/// The currently selected world's dex progress (species unlocked by any
/// player in the world).
///
/// # Errors
///
/// A display-ready message if no world is selected, or the query fails.
#[tauri::command]
pub fn dex_progress(app: AppHandle, state: State<AppState>) -> Result<DexProgressView, String> {
    let world_id = selected_world_id(&state)?;
    with_store(&app, &state, |store| queries::dex_progress(store, &world_id))
}

/// Per-player progression for the currently selected world's latest snapshot.
///
/// # Errors
///
/// A display-ready message if no world is selected, or the query fails.
#[tauri::command]
pub fn player_progress(app: AppHandle, state: State<AppState>) -> Result<Vec<PlayerProgressView>, String> {
    let snapshot_id = selected_snapshot_id(&state)?;
    with_store(&app, &state, |store| queries::player_progress(store, snapshot_id))
}

/// Base camps for the currently selected world's latest snapshot.
///
/// # Errors
///
/// A display-ready message if no world is selected, or the query fails.
#[tauri::command]
pub fn base_summary(app: AppHandle, state: State<AppState>) -> Result<Vec<BaseCampView>, String> {
    let snapshot_id = selected_snapshot_id(&state)?;
    with_store(&app, &state, |store| queries::base_summary(store, snapshot_id))
}

/// Per-player tech/boss/quest/collectible detail for the currently selected
/// world's latest snapshot — the rest of `PlayerProgress` beyond
/// `player_progress`'s scalar counters.
///
/// # Errors
///
/// A display-ready message if no world is selected, or the query fails.
#[tauri::command]
pub fn player_flags_detail(
    app: AppHandle,
    state: State<AppState>,
) -> Result<Vec<queries::PlayerFlagsView>, String> {
    let snapshot_id = selected_snapshot_id(&state)?;
    let mut flags =
        with_store(&app, &state, |store| queries::player_flags_detail(store, snapshot_id))?;
    if let Some(reference) = state.reference() {
        enrich_flags(&mut flags, reference);
    }
    Ok(flags)
}

#[cfg(test)]
mod tests {
    //! Exercises roster enrichment against the real save *and* the real pak
    //! together — the join neither crate can test on its own.
    use super::*;
    use paldex_locate::World;

    fn real_snapshot() -> Option<(Vec<PalView>, Vec<queries::PlayerFlagsView>)> {
        let world = if let Ok(dir) = std::env::var("PALDEX_TEST_SAVE_DIR") {
            resolve_manual(std::path::Path::new(&dir))
                .ok()?
                .into_iter()
                .find_map(|r| r.worlds().into_iter().find(World::is_trackable))?
        } else {
            discover()
                .into_iter()
                .find_map(|r| r.worlds().into_iter().find(World::is_trackable))?
        };
        let mut store = Store::open_in_memory().ok()?;
        let snapshot_id = crate::sync::sync_world(&mut store, &world).ok()?;
        Some((
            queries::pal_roster(&store, snapshot_id).ok()?,
            queries::player_flags_detail(&store, snapshot_id).ok()?,
        ))
    }

    #[test]
    fn enrichment_names_species_and_drops_human_npcs() {
        let (Some((mut roster, _)), Some(loaded)) = (real_snapshot(), load_reference()) else {
            eprintln!("skipping: need both a real save and the game pak");
            return;
        };
        let reference = &loaded.index;

        let before = roster.len();
        let npcs = roster.iter().filter(|p| reference.is_human_npc(&p.character_id)).count();
        enrich_roster(&mut roster, reference);
        eprintln!("roster {before} -> {} ({npcs} human NPCs removed)", roster.len());

        assert_eq!(roster.len(), before - npcs);
        assert!(npcs > 0, "the real world is known to contain human NPCs");
        assert!(
            roster.iter().all(|p| !reference.is_human_npc(&p.character_id)),
            "no human NPC should survive filtering"
        );

        // The point of the whole exercise: real names, not internal ids.
        let named = roster.iter().filter(|p| p.display_name.is_some()).count();
        eprintln!("{named}/{} entries have a display name", roster.len());
        assert!(
            named * 100 / roster.len().max(1) >= 95,
            "expected nearly every Pal to resolve a display name, got {named}/{}",
            roster.len()
        );
        assert!(
            roster.iter().any(|p| p.display_name.as_deref() != Some(p.character_id.as_str())),
            "display names should differ from internal ids for at least some Pals"
        );

        // Passives must resolve too -- the roster's passive column is raw ids
        // otherwise (e.g. CraftSpeed_up2 instead of Artisan).
        //
        // Resolution is asserted directly rather than by "the name differs
        // from the id": a handful of passives (`Legend`, `Invader`) genuinely
        // have display names identical to their internal ids, so a
        // difference-based check would report them as failures.
        let total_passives: usize = roster.iter().map(|p| p.passives.len()).sum();
        let unresolved: Vec<&String> = roster
            .iter()
            .flat_map(|p| p.passives.iter())
            .filter(|id| reference.passive(id).is_none())
            .collect();
        eprintln!(
            "{}/{total_passives} passive instances resolved",
            total_passives - unresolved.len()
        );
        assert!(total_passives > 0, "the real world has Pals with passives");
        assert!(
            unresolved.is_empty(),
            "every passive should resolve; {} did not, e.g. {:?}",
            unresolved.len(),
            &unresolved[..unresolved.len().min(5)]
        );
        for pal in roster.iter() {
            assert_eq!(
                pal.passive_names.len(),
                pal.passives.len(),
                "passive_names must stay parallel to passives"
            );
        }
    }

    /// Icons must survive the whole app-layer path: pak -> decode -> PNG ->
    /// data URL, keyed by the same `character_id` the roster carries.
    #[test]
    fn roster_species_resolve_to_icon_data_urls() {
        let (Some((roster, _)), Some(loaded)) = (real_snapshot(), load_reference()) else {
            eprintln!("skipping: need both a real save and the game pak");
            return;
        };
        let reference = &loaded.index;

        let species: std::collections::BTreeSet<String> =
            roster.iter().map(|p| p.character_id.clone()).collect();
        let mut resolved = 0usize;
        for id in &species {
            let Some(path) = reference.icon_path(id) else { continue };
            let path = path.to_owned();
            let mut pak = loaded.pak.lock().expect("pak lock");
            let png = paldex_data::load_icon_png(&mut pak, &path)
                .unwrap_or_else(|e| panic!("decoding icon for {id}: {e}"));
            let url = to_png_data_url(&png);
            assert!(url.starts_with("data:image/png;base64,"), "{id} should be a PNG data URL");
            assert!(url.len() > 100, "{id} data URL suspiciously short");
            resolved += 1;
        }

        eprintln!("{resolved}/{} roster species decoded to icons", species.len());
        assert!(
            resolved * 100 / species.len().max(1) >= 90,
            "expected most roster species to have artwork, got {resolved}/{}",
            species.len()
        );
    }

    /// The plan's Phase 7 criterion: every unlocked tech name should resolve to
    /// a reference definition.
    #[test]
    fn enrichment_names_unlocked_technologies() {
        let (Some((_, mut flags)), Some(loaded)) = (real_snapshot(), load_reference()) else {
            eprintln!("skipping: need both a real save and the game pak");
            return;
        };
        let reference = &loaded.index;

        enrich_flags(&mut flags, reference);
        assert!(!flags.is_empty(), "the real world has players");

        let total: usize = flags.iter().map(|f| f.unlocked_tech.len()).sum();
        let unresolved: Vec<&String> = flags
            .iter()
            .flat_map(|f| f.unlocked_tech.iter())
            .filter(|id| reference.technology(id).is_none())
            .collect();
        eprintln!("{}/{total} unlocked technologies resolved", total - unresolved.len());
        assert!(total > 0, "the real world has unlocked technologies");

        // The residual was classified against the real save rather than
        // assumed (see `examples/classify_unresolved_tech.rs`). Of the
        // distinct unlocked technologies that don't resolve:
        //
        // - The large majority have **no row** in the technology name table.
        //   Ids of the `Battle_Armor_Grade_01_Cloth` shape were searched
        //   across all 482 localized text tables in *every* shipped language
        //   and appear in none of them — there is no display name to find.
        // - Six have a row whose reference dangles in the game's own data:
        //   `Glider_Tera` is not an item, and techs `GrapplingGun`..
        //   `GrapplingGun5` point at item ids that are actually spelled
        //   `GrapplingGun_1`..`_5`. Guessing past that mismatch would risk
        //   attaching a confidently wrong name, so these keep their raw id.
        // - None are untranslated placeholders.
        let resolved_pct = (total - unresolved.len()) * 100 / total;
        assert!(
            resolved_pct >= 75,
            "expected most technologies to resolve, got {resolved_pct}%; \
             {} unresolved, e.g. {:?}",
            unresolved.len(),
            &unresolved[..unresolved.len().min(10)]
        );
    }
}
