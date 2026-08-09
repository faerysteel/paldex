//! Commands exposed to the frontend.
//!
//! Commands that do real work — decoding a save, querying the roster, decoding
//! icons — are declared `#[tauri::command(async)]`. A plain `#[tauri::command]`
//! on a synchronous function runs on the **main thread**, which is also the
//! webview's event loop, so anything slow there freezes the window instead of
//! merely being slow.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use paldex_data::{Pak, ReferenceData, ReferenceIndex};
use paldex_locate::{discover, discover_paks, resolve_manual, SaveRoot, World};
use paldex_store::{Store, WatchConfig};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

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
    /// Stop flag for the live-sync thread watching the selected world, if one
    /// is running. Replacing it stops the previous world's watcher.
    watcher: Mutex<Option<Arc<AtomicBool>>>,
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

/// Emitted to the frontend after every automatic re-sync that actually
/// ingested something. Payload is the same [`SnapshotSummaryView`] the manual
/// Resync button gets back, so the UI can treat both paths identically.
pub const SNAPSHOT_EVENT: &str = "paldex://snapshot";

/// Watch the selected world's save directory and re-ingest on every in-game
/// save, emitting [`SNAPSHOT_EVENT`] when the content actually changed.
///
/// This is the plan's headline behaviour — "re-syncs within ~10s of every
/// in-game save, no user action". The watcher runs on its own detached thread
/// because `watch_until` blocks; the app owns a stop flag so that selecting a
/// different world tears the previous one down instead of leaving it
/// re-ingesting behind the new selection.
///
/// Failure to start is reported but never fatal: the manual Resync button
/// remains, so a world that can't be watched is degraded, not broken.
fn start_watcher(app: &AppHandle, state: &State<AppState>, world: World) {
    let stop = Arc::new(AtomicBool::new(false));
    match state.watcher.lock() {
        Ok(mut guard) => {
            if let Some(previous) = guard.replace(Arc::clone(&stop)) {
                previous.store(true, Ordering::Relaxed);
            }
        }
        Err(_) => {
            eprintln!("[paldex] watcher: lock poisoned, not starting live sync");
            return;
        }
    }

    let app = app.clone();
    std::thread::spawn(move || {
        let world_dir = world.path.clone();
        eprintln!("[paldex] watcher: watching {}", world_dir.display());
        let result = paldex_store::watch_until(
            &world_dir,
            WatchConfig::default(),
            &stop,
            |path, _bytes| {
                // `watch_until` hands over the stability-checked bytes, but a
                // full sync needs `Players/*.sav` too, so it re-reads the world
                // rather than using them. The stability check is still what
                // keeps this from firing mid-write.
                eprintln!("[paldex] watcher: {} changed", path.display());
                on_watched_change(&app, &world);
            },
        );
        if let Err(e) = result {
            eprintln!("[paldex] watcher: stopped with error: {e}");
        }
    });
}

/// One automatic re-sync pass. Runs on the watcher thread.
fn on_watched_change(app: &AppHandle, world: &World) {
    let state = app.state::<AppState>();

    let outcome = with_store(app, &state, |store| {
        sync::sync_world_if_changed(store, world).map_err(|e| e.to_string())
    });

    let snapshot_id = match outcome {
        Ok(sync::SyncOutcome::Ingested(id)) => id,
        Ok(sync::SyncOutcome::Unchanged) => {
            eprintln!("[paldex] watcher: content unchanged, skipping ingest");
            return;
        }
        Err(e) => {
            // A torn read that slipped past the stability check parses as
            // garbage. The next autosave will fire this again, so a transient
            // failure must not surface as a user-visible error.
            eprintln!("[paldex] watcher: re-sync failed, will retry on next save: {e}");
            return;
        }
    };

    // The selection can change while a sync is in flight. Publishing a
    // snapshot for a world the user has already navigated away from would
    // point the UI at another world's data.
    match state.selected.lock() {
        Ok(mut guard) => match guard.as_mut() {
            Some(selected) if selected.world_id == world.id => {
                selected.latest_snapshot_id = snapshot_id;
            }
            _ => {
                eprintln!("[paldex] watcher: selection changed mid-sync, dropping snapshot");
                return;
            }
        },
        Err(_) => return,
    }

    match with_store(app, &state, |store| queries::snapshot_summary(store, snapshot_id)) {
        Ok(summary) => {
            eprintln!("[paldex] watcher: emitting snapshot {snapshot_id}");
            if let Err(e) = app.emit(SNAPSHOT_EVENT, summary) {
                eprintln!("[paldex] watcher: emit failed: {e}");
            }
        }
        Err(e) => eprintln!("[paldex] watcher: summary failed: {e}"),
    }
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
#[tauri::command(async)]
pub fn select_world(
    app: AppHandle,
    state: State<AppState>,
    world_path: PathBuf,
) -> Result<SnapshotSummaryView, String> {
    eprintln!("[paldex] select_world: {}", world_path.display());
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

    start_watcher(&app, &state, world);

    eprintln!("[paldex] select_world: ingested snapshot {snapshot_id}");
    with_store(&app, &state, |store| queries::snapshot_summary(store, snapshot_id))
}

/// Re-decode and re-ingest the currently selected world.
///
/// # Errors
///
/// A display-ready message if no world is selected yet, or the re-sync
/// itself fails the same way `select_world` can.
#[tauri::command(async)]
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
#[tauri::command(async)]
pub fn pal_roster(app: AppHandle, state: State<AppState>) -> Result<Vec<PalView>, String> {
    eprintln!("[paldex] pal_roster: start");
    let snapshot_id = selected_snapshot_id(&state)?;
    let mut roster = with_store(&app, &state, |store| queries::pal_roster(store, snapshot_id))?;

    if let Some(reference) = state.reference() {
        enrich_roster(&mut roster, reference);
    }
    eprintln!("[paldex] pal_roster: returning {} rows", roster.len());
    Ok(roster)
}

/// Drop human NPCs and attach localized species and passive names, plus the
/// species-level facts the roster shows per pal.
fn enrich_roster(roster: &mut Vec<PalView>, reference: &ReferenceIndex) {
    roster.retain(|pal| !reference.is_human_npc(&pal.character_id));
    for pal in roster.iter_mut() {
        if let Some(species) = reference.species(&pal.character_id) {
            pal.display_name = Some(species.display_name.clone());
            pal.elements = element_views(&species.elements, reference);
            pal.rarity = species.rarity;
        }
        pal.passive_names = pal
            .passives
            .iter()
            .map(|id| resolve_or_id(reference.passive(id).map(|p| p.display_name.as_str()), id))
            .collect();
    }
}

/// Pair each element enum name with its localized label, keeping the id so the
/// UI can colour by element without depending on the display language.
fn element_views(elements: &[String], reference: &ReferenceIndex) -> Vec<queries::ElementView> {
    elements
        .iter()
        .map(|id| queries::ElementView {
            name: resolve_or_id(reference.element_name(id), id).to_owned(),
            id: id.clone(),
        })
        .collect()
}

/// A species' work suitabilities in the game's own display order.
///
/// `Species::work_suitabilities` is a `BTreeMap`, so iterating it directly
/// would put Collection before EmitFlame — alphabetical, and not the icon row
/// the player recognises from the Paldeck. Anything the order table doesn't
/// mention still appears, after the ordered entries, rather than being dropped.
fn work_views(
    species: &paldex_data::Species,
    reference: &ReferenceIndex,
) -> Vec<queries::WorkSuitabilityView> {
    let rank = |id: &str| {
        reference
            .work_suitability_order()
            .iter()
            .position(|o| o.eq_ignore_ascii_case(id))
            .unwrap_or(usize::MAX)
    };
    let mut views: Vec<queries::WorkSuitabilityView> = species
        .work_suitabilities
        .iter()
        .map(|(id, level)| queries::WorkSuitabilityView {
            name: resolve_or_id(reference.work_suitability_name(id), id).to_owned(),
            id: id.clone(),
            level: *level,
        })
        .collect();
    views.sort_by_key(|w| (rank(&w.id), w.id.clone()));
    views
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
#[tauri::command(async)]
pub fn pal_icons(
    state: State<AppState>,
    character_ids: Vec<String>,
) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    // Escape hatch for diagnosing UI trouble without a rebuild: run the app
    // with PALDEX_NO_ICONS=1 to serve the roster with no artwork at all.
    if std::env::var_os("PALDEX_NO_ICONS").is_some() {
        eprintln!("[paldex] pal_icons: disabled via PALDEX_NO_ICONS");
        return Ok(out);
    }
    let Some(reference) = state.loaded_reference() else {
        eprintln!("[paldex] pal_icons: no reference data (no pak found)");
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
    let bytes: usize = out.values().map(String::len).sum();
    eprintln!("[paldex] pal_icons: {} icons, {} KB of payload", out.len(), bytes / 1024);
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
#[tauri::command(async)]
pub fn dex_progress(app: AppHandle, state: State<AppState>) -> Result<DexProgressView, String> {
    let world_id = selected_world_id(&state)?;
    let snapshot_id = selected_snapshot_id(&state)?;
    let facts = with_store(&app, &state, |store| {
        queries::dex_facts(store, &world_id, snapshot_id)
    })?;

    Ok(match state.reference() {
        Some(index) => dex_with_reference(&facts, index),
        None => dex_without_reference(&facts),
    })
}

/// Project the save's dex facts onto the pak's species list, so the grid can
/// show what hasn't been caught yet rather than only what has.
///
/// Save ids and reference keys don't match literally — `FName`s are
/// case-insensitive, the data is inconsistent (`Sheepball` vs `SheepBall`),
/// and variants carry a `BOSS_`/`PREDATOR_` prefix. Every id is therefore
/// funnelled through `index.species()`, which owns that normalization, rather
/// than compared directly. Alpha and predator variants collapse onto their
/// base species, which is what the Paldeck does too.
fn dex_with_reference(facts: &queries::DexFacts, index: &ReferenceIndex) -> DexProgressView {
    let canonical = |save_id: &str| index.species(save_id).map(|s| s.character_id.clone());

    let caught: std::collections::HashSet<String> =
        facts.unlocked.iter().filter_map(|id| canonical(id)).collect();
    let bonus: std::collections::HashSet<String> =
        facts.bonus_claimed.iter().filter_map(|id| canonical(id)).collect();

    let mut counts: HashMap<String, i64> = HashMap::new();
    for (save_id, count) in &facts.capture_counts {
        if let Some(key) = canonical(save_id) {
            // Variants fold into the base species, so keep the best.
            let slot = counts.entry(key).or_insert(0);
            *slot = (*slot).max(*count);
        }
    }

    // Only species with a Paldeck number are Paldeck entries. Tower bosses,
    // raid/collab content, and unused entries are excluded here for the same
    // reason the game excludes them — counting them was what inflated the
    // denominator to 322. A caught species with no number is still listed
    // rather than dropped, so the caught total can never silently shrink.
    let mut entries: Vec<queries::DexEntryView> = index
        .species_iter()
        .filter(|species| {
            species.dex_number.is_some() || caught.contains(&species.character_id)
        })
        .map(|species| queries::DexEntryView {
            caught: caught.contains(&species.character_id),
            capture_count: counts.get(&species.character_id).copied().unwrap_or(0),
            bonus_claimed: bonus.contains(&species.character_id),
            character_id: species.character_id.clone(),
            display_name: species.display_name.clone(),
            dex_label: species.dex_label(),
            elements: element_views(&species.elements, index),
            rarity: species.rarity,
            stats: Some(queries::BaseStatsView {
                hp: species.stats.hp,
                melee_attack: species.stats.melee_attack,
                shot_attack: species.stats.shot_attack,
                defense: species.stats.defense,
                support: species.stats.support,
                craft_speed: species.stats.craft_speed,
            }),
            work_suitabilities: work_views(species, index),
        })
        .collect();

    // Paldeck order: by number, with a base species ahead of its `B` variant.
    // Anything unnumbered sorts last, by name.
    entries.sort_by(|a, b| {
        let key = |e: &queries::DexEntryView| {
            (
                e.dex_label.is_none(),
                e.dex_label.clone().unwrap_or_default(),
                e.display_name.clone(),
            )
        };
        key(a).cmp(&key(b))
    });

    DexProgressView {
        unlocked_species_count: entries.iter().filter(|e| e.caught).count() as i64,
        unlocked_species: facts.unlocked.clone(),
        total_species_count: index.dex_entry_count() as i64,
        entries,
    }
}

/// Fallback with no game installed: the save alone knows what was caught but
/// not what exists, so the grid shows caught species only.
fn dex_without_reference(facts: &queries::DexFacts) -> DexProgressView {
    let mut entries: Vec<queries::DexEntryView> = facts
        .unlocked
        .iter()
        .map(|id| queries::DexEntryView {
            character_id: id.clone(),
            display_name: id.clone(),
            dex_label: None,
            caught: true,
            capture_count: facts.capture_counts.get(id).copied().unwrap_or(0),
            bonus_claimed: facts.bonus_claimed.contains(id),
            // No pak means no parameter table, so none of these are knowable.
            elements: Vec::new(),
            rarity: 0,
            stats: None,
            work_suitabilities: Vec::new(),
        })
        .collect();
    entries.sort_by(|a, b| a.display_name.cmp(&b.display_name));

    DexProgressView {
        unlocked_species_count: entries.len() as i64,
        unlocked_species: facts.unlocked.clone(),
        total_species_count: 0,
        entries,
    }
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

    /// The dex grid's whole value is showing what *hasn't* been caught, which
    /// only works if the save's species ids line up with the pak's. A silent
    /// normalization mismatch would show a fully-played world as almost
    /// entirely unseen, so this asserts the join really lands.
    #[test]
    fn dex_entries_join_save_ids_onto_pak_species() {
        let Some(loaded) = load_reference() else {
            eprintln!("skipping: need the game pak");
            return;
        };
        let Some(world) = crate::sync::tests::real_trackable_world() else {
            eprintln!("skipping: need a real save");
            return;
        };
        let mut store = Store::open_in_memory().expect("store");
        let snapshot_id = crate::sync::sync_world(&mut store, &world).expect("sync");
        let facts = queries::dex_facts(&store, &world.id, snapshot_id).expect("dex_facts");

        let view = dex_with_reference(&facts, &loaded.index);
        eprintln!(
            "dex: {}/{} species caught, {} entries",
            view.unlocked_species_count,
            view.total_species_count,
            view.entries.len()
        );

        assert!(!facts.unlocked.is_empty(), "the real world has caught species");
        assert!(
            view.total_species_count > view.unlocked_species_count,
            "the pak should know more species than one save has caught"
        );

        // The join is the part that breaks silently. Nearly every id the save
        // recorded must map onto a pak species; a handful of quest-only forms
        // legitimately have no name row of their own.
        let unmapped = facts
            .unlocked
            .iter()
            .filter(|id| loaded.index.species(id).is_none())
            .collect::<Vec<_>>();
        eprintln!("{} unlocked ids did not map to a species", unmapped.len());
        assert!(
            unmapped.len() * 100 / facts.unlocked.len() <= 10,
            "expected nearly every unlocked id to map; {} of {} did not, e.g. {:?}",
            unmapped.len(),
            facts.unlocked.len(),
            &unmapped[..unmapped.len().min(5)]
        );

        let caught = view.entries.iter().filter(|e| e.caught).count();
        assert_eq!(
            caught as i64, view.unlocked_species_count,
            "the caught tiles must agree with the headline count"
        );
        assert!(
            view.entries.iter().any(|e| !e.caught),
            "a real world should still have uncaught species to show"
        );
        assert!(
            view.entries.iter().all(|e| !e.display_name.is_empty()),
            "every tile needs a label"
        );
    }

    /// Dev-only: dump real command output as JSON so the frontend can be
    /// driven with genuine data outside the Tauri shell. Runs only when
    /// `PALDEX_FIXTURE_OUT` names a directory; otherwise it is a no-op.
    #[test]
    fn dump_frontend_fixture() {
        let Ok(out) = std::env::var("PALDEX_FIXTURE_OUT") else { return };
        let (Some((mut roster, mut flags)), Some(loaded)) = (real_snapshot(), load_reference())
        else {
            eprintln!("skipping fixture dump: need a real save and the game pak");
            return;
        };
        enrich_roster(&mut roster, &loaded.index);
        enrich_flags(&mut flags, &loaded.index);

        // The dex grid needs the same projection the command performs, so the
        // harness renders real Paldeck numbers rather than a hand-made stub.
        // `player_progress` comes from the same snapshot: the roster screen
        // requests it too, and a missing fixture surfaces as an error banner.
        let derived = crate::sync::tests::real_trackable_world().and_then(|world| {
            let mut store = Store::open_in_memory().ok()?;
            let snapshot_id = crate::sync::sync_world(&mut store, &world).ok()?;
            let facts = queries::dex_facts(&store, &world.id, snapshot_id).ok()?;
            let players = queries::player_progress(&store, snapshot_id).ok()?;
            let summary = queries::snapshot_summary(&store, snapshot_id).ok()?;
            Some((dex_with_reference(&facts, &loaded.index), players, summary))
        });

        // Roster species *plus* every dex entry: the dex grid shows uncaught
        // species too, and those never appear in the roster. Dumping only
        // roster species left the "missing" tiles artwork-less in the harness
        // while the real app renders them fine — a preview that lies about the
        // screen is worse than no preview.
        let mut species: std::collections::BTreeSet<String> =
            roster.iter().map(|p| p.character_id.clone()).collect();
        if let Some((dex, _, _)) = &derived {
            species.extend(dex.entries.iter().map(|e| e.character_id.clone()));
        }
        let mut icons = serde_json::Map::new();
        for id in &species {
            let Some(path) = loaded.index.icon_path(id) else { continue };
            let path = path.to_owned();
            let mut pak = loaded.pak.lock().expect("pak lock");
            if let Ok(png) = paldex_data::load_icon_png(&mut pak, &path) {
                icons.insert(id.clone(), serde_json::Value::String(to_png_data_url(&png)));
            }
        }

        std::fs::create_dir_all(&out).expect("create fixture dir");
        let write = |name: &str, v: serde_json::Value| {
            std::fs::write(format!("{out}/{name}.json"), serde_json::to_vec(&v).unwrap()).unwrap();
        };
        write("pal_roster", serde_json::to_value(&roster).unwrap());
        write("player_flags_detail", serde_json::to_value(&flags).unwrap());
        write("pal_icons", serde_json::Value::Object(icons));
        if let Some((dex, players, summary)) = &derived {
            write("dex_progress", serde_json::to_value(dex).unwrap());
            write("player_progress", serde_json::to_value(players).unwrap());
            // `list_saves` and `select_world` are what let the harness render
            // the *whole* App, world picker included — the transition that
            // took the UI down once already. Without them the preview stops at
            // the picker with a JSON parse error.
            write("list_saves", serde_json::to_value(list_saves()).unwrap());
            write("select_world", serde_json::to_value(summary).unwrap());
            write("force_resync", serde_json::to_value(summary).unwrap());
            eprintln!(
                "fixture: dex {}/{}",
                dex.unlocked_species_count, dex.total_species_count
            );
        }
        eprintln!("fixture: {} pals, {} icons -> {out}", roster.len(), species.len());
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
