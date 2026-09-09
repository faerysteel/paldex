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
pub fn pal_roster(
    app: AppHandle,
    state: State<AppState>,
    player_uid: Option<String>,
    include_base_pals: Option<bool>,
) -> Result<Vec<PalView>, String> {
    eprintln!("[paldex] pal_roster: start");
    let snapshot_id = selected_snapshot_id(&state)?;
    let scope = player_scope(player_uid.as_deref());
    let mut roster = with_store(&app, &state, |store| {
        queries::pal_roster(store, snapshot_id, scope, base_pals(include_base_pals))
    })?;

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

/// The currently selected world's dex progress, for one player or for the
/// world as a whole.
///
/// `player_uid` of `None` means every player aggregated — see
/// [`queries::PlayerScope`] for why that is a deliberate choice rather than
/// the natural default.
///
/// # Errors
///
/// A display-ready message if no world is selected, or the query fails.
#[tauri::command(async)]
pub fn dex_progress(
    app: AppHandle,
    state: State<AppState>,
    player_uid: Option<String>,
) -> Result<DexProgressView, String> {
    let world_id = selected_world_id(&state)?;
    let snapshot_id = selected_snapshot_id(&state)?;
    let scope = player_scope(player_uid.as_deref());
    let facts = with_store(&app, &state, |store| {
        queries::dex_facts(store, &world_id, snapshot_id, scope)
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

    // Variants fold into the base species, so keep the best of each. This is
    // a *within-player* fold across spellings and alpha/predator forms, not
    // the cross-player pooling `PlayerScope` exists to prevent.
    let fold = |source: &HashMap<String, i64>| {
        let mut out: HashMap<String, i64> = HashMap::new();
        for (save_id, value) in source {
            if let Some(key) = canonical(save_id) {
                let slot = out.entry(key).or_insert(0);
                *slot = (*slot).max(*value);
            }
        }
        out
    };
    let counts = fold(&facts.capture_counts);
    let bonus = fold(&facts.bonus_tiers);

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
            bonus_tier: bonus.get(&species.character_id).copied().unwrap_or(0),
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
            bonus_tier: facts.bonus_tiers.get(id).copied().unwrap_or(0),
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

/// Turn the frontend's optional uid into a [`queries::PlayerScope`].
///
/// An absent or empty uid is "all players": the selector's own default, and
/// what a single-player world always resolves to.
fn player_scope(player_uid: Option<&str>) -> queries::PlayerScope<'_> {
    match player_uid {
        Some(uid) if !uid.is_empty() => queries::PlayerScope::Only(uid),
        _ => queries::PlayerScope::All,
    }
}

/// Every screen's "include base pals" checkbox, which defaults to on.
///
/// Base-camp workers are unowned — they belong to the guild, not a player — so
/// they could plausibly be hidden whenever a single player is selected. They
/// are not: the checkbox is the only thing that decides, under every scope, on
/// every tab. A base Pal is still a Pal you have, and answering "where did it
/// go?" by silently dropping it from the roster is worse than showing it.
///
/// `None` is the default rather than an error because the frontend omits the
/// argument on first render; see `queries::BasePals`.
fn base_pals(include: Option<bool>) -> queries::BasePals {
    if include.unwrap_or(true) {
        queries::BasePals::Include
    } else {
        queries::BasePals::Exclude
    }
}

/// Who is in the currently selected world, for the player selector.
///
/// # Errors
///
/// A display-ready message if no world is selected, or the query fails.
#[tauri::command]
pub fn world_players(
    app: AppHandle,
    state: State<AppState>,
) -> Result<Vec<queries::WorldPlayerView>, String> {
    let snapshot_id = selected_snapshot_id(&state)?;
    with_store(&app, &state, |store| queries::world_players(store, snapshot_id))
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

/// Base camps for the currently selected world's latest snapshot, each with
/// the Pals working there.
///
/// Takes no player argument, and deliberately so: a base belongs to the guild,
/// not to a player — the same reasoning that makes base Pals orthogonal to
/// [`queries::PlayerScope`]. The roster is fetched with `All`/`Include`
/// because base workers are exactly the Pals that `Exclude` drops.
///
/// # Errors
///
/// A display-ready message if no world is selected, or the query fails.
#[tauri::command(async)]
pub fn base_summary(app: AppHandle, state: State<AppState>) -> Result<Vec<BaseCampView>, String> {
    let snapshot_id = selected_snapshot_id(&state)?;
    let bases = with_store(&app, &state, |store| {
        queries::base_camp_workers(store, snapshot_id)
    })?;
    let (views, pals) =
        analysis_roster(&app, &state, queries::PlayerScope::All, queries::BasePals::Include)?;
    Ok(base_summary_view(&bases, &views, &pals))
}

/// The assembly itself, split from the command so it can be exercised against
/// the real save without a running Tauri app — the same split
/// [`quality_view`] makes.
fn base_summary_view(
    bases: &[queries::BaseCampWorkers],
    views: &[PalView],
    pals: &[paldex_model::Pal],
) -> Vec<BaseCampView> {
    let graded: std::collections::HashMap<&str, queries::GradedPalView> = views
        .iter()
        .zip(pals)
        .map(|(view, pal)| (view.instance_id.as_str(), graded_view(view, pal)))
        .collect();

    bases
        .iter()
        .enumerate()
        .map(|(index, base)| {
            let mut workers: Vec<queries::GradedPalView> = base
                .worker_instance_ids
                .iter()
                .filter_map(|id| graded.get(id.as_str()).cloned())
                .collect();
            // Same ordering the analysis screen gives a `GradedPalView` list,
            // so the shared type reads the same wherever it appears.
            workers.sort_by(|a, b| {
                b.composite
                    .total_cmp(&a.composite)
                    .then_with(|| b.level.cmp(&a.level))
                    .then_with(|| a.instance_id.cmp(&b.instance_id))
            });

            BaseCampView {
                id: base.id.clone(),
                guild_id: base.guild_id.clone(),
                // 1-based over a list `base_camp_workers` already sorted by id.
                number: u32::try_from(index + 1).unwrap_or(u32::MAX),
                // Counted after the join back onto the roster, so it can never
                // disagree with the list actually rendered.
                worker_count: u32::try_from(workers.len()).unwrap_or(u32::MAX),
                workers,
            }
        })
        .collect()
}

/// The enriched roster, paired with the domain [`Pal`]s the analysis functions
/// take.
///
/// Both come from one store read: the views carry the localized names the
/// screen renders, and the `Pal`s carry the shape `paldex-model` grades. They
/// are parallel by construction — a Pal whose id doesn't round-trip through
/// `Uuid` is dropped from both, which cannot happen for a row this app wrote.
fn analysis_roster(
    app: &AppHandle,
    state: &State<AppState>,
    scope: queries::PlayerScope,
    base_pals: queries::BasePals,
) -> Result<(Vec<PalView>, Vec<paldex_model::Pal>), String> {
    let snapshot_id = selected_snapshot_id(state)?;
    let mut views = with_store(app, state, |store| {
        queries::pal_roster(store, snapshot_id, scope, base_pals)
    })?;
    if let Some(reference) = state.reference() {
        enrich_roster(&mut views, reference);
    }

    let pals: Vec<paldex_model::Pal> = views.iter().filter_map(to_model_pal).collect();
    views.retain(|v| uuid::Uuid::parse_str(&v.instance_id).is_ok());
    Ok((views, pals))
}

/// Project a stored row back onto the domain type.
///
/// The store flattens a [`paldex_model::Pal`] across several tables, and this
/// rebuilds only what the analysis layer reads — IVs, level, rank, passives,
/// gender, species. `location` is left `None` because the flat row keeps
/// `location_kind` but not the container GUID it was resolved from, and no
/// analysis function looks at it. Re-decoding the save to recover it would
/// cost seconds per request to populate a field nothing reads.
fn to_model_pal(view: &PalView) -> Option<paldex_model::Pal> {
    let clamp = |v: i64| u8::try_from(v).unwrap_or(u8::MAX);
    Some(paldex_model::Pal {
        instance_id: uuid::Uuid::parse_str(&view.instance_id).ok()?,
        character_id: view.character_id.clone(),
        owner: view.owner.as_deref().and_then(|o| uuid::Uuid::parse_str(o).ok()),
        level: clamp(view.level),
        rank: clamp(view.rank),
        souls: paldex_model::SoulUpgrades {
            hp: clamp(view.soul_hp),
            attack: clamp(view.soul_attack),
            defense: clamp(view.soul_defense),
            craft_speed: clamp(view.soul_craft_speed),
        },
        ivs: paldex_model::Ivs {
            hp: clamp(view.iv_hp),
            shot: clamp(view.iv_shot),
            defense: clamp(view.iv_defense),
        },
        passives: view.passives.clone(),
        equipped_moves: view.equipped_moves.clone(),
        mastered_moves: view.mastered_moves.clone(),
        gender: match view.gender.as_str() {
            "male" => paldex_model::Gender::Male,
            "female" => paldex_model::Gender::Female,
            _ => paldex_model::Gender::Unknown,
        },
        is_lucky: view.is_lucky,
        is_boss: view.is_boss,
        is_predator: view.is_predator,
        nickname: view.nickname.clone(),
        location: None,
    })
}

/// Owned Pals with their IV grades, plus the three derived lists the analysis
/// screen shows — best-of-species, condense fodder, and passive ranking.
///
/// The derived lists are instance ids into `graded` rather than copies of the
/// rows, so a 2,000-Pal roster is sent once rather than four times.
///
/// # Errors
///
/// A display-ready message if no world is selected, or the query fails.
#[tauri::command(async)]
pub fn pal_quality(
    app: AppHandle,
    state: State<AppState>,
    player_uid: Option<String>,
    include_base_pals: Option<bool>,
) -> Result<queries::PalQualityView, String> {
    let scope = player_scope(player_uid.as_deref());
    let (views, pals) = analysis_roster(&app, &state, scope, base_pals(include_base_pals))?;
    Ok(quality_view(&views, &pals))
}

/// The grading itself, split from the command so it can be exercised against
/// the real save without a running Tauri app.
fn quality_view(views: &[PalView], pals: &[paldex_model::Pal]) -> queries::PalQualityView {
    let mut graded: Vec<queries::GradedPalView> =
        views.iter().zip(pals).map(|(view, pal)| graded_view(view, pal)).collect();
    graded.sort_by(|a, b| {
        b.composite
            .total_cmp(&a.composite)
            .then_with(|| b.level.cmp(&a.level))
            .then_with(|| a.instance_id.cmp(&b.instance_id))
    });

    // Both derived sets are rendered in `graded`'s order rather than their own.
    // `best_of_species` returns a `HashMap`, so iterating it directly would
    // reorder the list between runs on identical data.
    let best: std::collections::HashSet<String> = paldex_model::best_of_species(pals)
        .values()
        .map(|p| p.instance_id.to_string())
        .collect();
    let fodder: std::collections::HashSet<String> = paldex_model::condense_candidates(pals)
        .iter()
        .map(|p| p.instance_id.to_string())
        .collect();

    let passive_ranking = paldex_model::rank_by_passives(pals)
        .iter()
        .map(|p| p.instance_id.to_string())
        .collect();

    queries::PalQualityView {
        best_of_species: graded.iter().filter(|g| best.contains(&g.instance_id)).map(|g| g.instance_id.clone()).collect(),
        condense_candidates: graded.iter().filter(|g| fodder.contains(&g.instance_id)).map(|g| g.instance_id.clone()).collect(),
        passive_ranking,
        graded,
    }
}

fn graded_view(view: &PalView, pal: &paldex_model::Pal) -> queries::GradedPalView {
    queries::GradedPalView {
        instance_id: view.instance_id.clone(),
        character_id: view.character_id.clone(),
        display_name: view.display_name.clone(),
        nickname: view.nickname.clone(),
        level: view.level,
        rank: view.rank,
        gender: view.gender.clone(),
        iv_hp: view.iv_hp,
        iv_shot: view.iv_shot,
        iv_defense: view.iv_defense,
        composite: paldex_model::grade_ivs(&pal.ivs),
        passive_names: passive_labels(view),
    }
}

/// Localized passive names, falling back to the raw ids when no pak supplied
/// them — the same rule the roster table applies.
fn passive_labels(view: &PalView) -> Vec<String> {
    if view.passive_names.len() == view.passives.len() {
        view.passive_names.clone()
    } else {
        view.passives.clone()
    }
}

/// Owned pairs that breed into `target`, best first.
///
/// Returns an empty list rather than an error when no pak is installed: the
/// breeding table lives in the pak, so without it there is nothing to suggest
/// — the same degradation the rest of the app applies to missing reference
/// data.
///
/// # Errors
///
/// A display-ready message if no world is selected, or the query fails.
#[tauri::command(async)]
pub fn breeding_options(
    app: AppHandle,
    state: State<AppState>,
    target: String,
    player_uid: Option<String>,
    include_base_pals: Option<bool>,
) -> Result<Vec<queries::BreedingPairView>, String> {
    let (views, pals) = analysis_roster(
        &app,
        &state,
        player_scope(player_uid.as_deref()),
        base_pals(include_base_pals),
    )?;
    let Some(reference) = state.reference() else {
        eprintln!("[paldex] breeding_options: no reference data (no pak found)");
        return Ok(Vec::new());
    };
    Ok(breeding_view(&views, &pals, &target, reference))
}

/// The pair search itself, split from the command for the same reason as
/// [`quality_view`].
fn breeding_view(
    views: &[PalView],
    pals: &[paldex_model::Pal],
    target: &str,
    reference: &ReferenceIndex,
) -> Vec<queries::BreedingPairView> {
    let pairs = paldex_model::breeding_suggestions(pals, target, |a, b| {
        reference.breeding_result(a, b).map(str::to_owned)
    });

    let by_id = roster_by_id(views);
    let parent = |pal: &paldex_model::Pal| parent_view(pal, &by_id);

    eprintln!("[paldex] breeding_options: {} pairs for {target}", pairs.len());
    pairs
        .into_iter()
        .map(|pair| queries::BreedingPairView {
            parent_a: parent(pair.parent_a),
            parent_b: parent(pair.parent_b),
            parent_iv_average: pair.parent_iv_average,
            inherited_passives: pair
                .inherited_passives
                .iter()
                .map(|id| resolve_or_id(reference.passive(id).map(|p| p.display_name.as_str()), id))
                .collect(),
            score: pair.score,
        })
        .collect()
}

fn roster_by_id(views: &[PalView]) -> HashMap<&str, &PalView> {
    views.iter().map(|v| (v.instance_id.as_str(), v)).collect()
}

/// A decoded Pal as a breeding parent, taking the localized species name and
/// the gender string from the roster row it came from.
fn parent_view(
    pal: &paldex_model::Pal,
    by_id: &HashMap<&str, &PalView>,
) -> queries::BreedingParentView {
    let id = pal.instance_id.to_string();
    let view = by_id.get(id.as_str());
    queries::BreedingParentView {
        character_id: pal.character_id.clone(),
        display_name: view.and_then(|v| v.display_name.clone()),
        nickname: pal.nickname.clone(),
        level: i64::from(pal.level),
        gender: view.map_or_else(|| "unknown".to_owned(), |v| v.gender.clone()),
        iv_hp: i64::from(pal.ivs.hp),
        iv_shot: i64::from(pal.ivs.shot),
        iv_defense: i64::from(pal.ivs.defense),
        instance_id: id,
    }
}

/// Species the breeding search can actually be asked for — every species that
/// some pairing produces.
///
/// The Paldeck is the wrong list for this picker. It includes species nothing
/// breeds into: Panthalus, Astralym and the two Yakushima raid bosses, which
/// are outside the generic candidate pool and have no unique combo naming
/// them, and variant forms like `PlantSlime_Flower` that are shadowed by their
/// base species — a pair of them yields Gumoss, so nothing produces the
/// variant itself. Offering those as targets promises a result that cannot
/// exist.
///
/// This is *not* a "can this Pal breed" filter, and must not be confused with
/// one. Frostallion, Jetragon, Paladius, Necromus and Bellanoir all sit
/// outside the generic pool too, yet each has a unique combo that breeds it
/// true — they belong in the picker and are perfectly good parents.
///
/// # Errors
///
/// A display-ready message if the reference data is unavailable — an empty
/// list is returned instead, since without a pak there is no breeding table.
#[tauri::command(async)]
pub fn breeding_targets(state: State<AppState>) -> Vec<queries::BreedingTargetView> {
    let Some(reference) = state.reference() else {
        eprintln!("[paldex] breeding_targets: no reference data (no pak found)");
        return Vec::new();
    };

    // A species is producible exactly when it is its own self-breeding result
    // — proven equivalent to pairing everything with everything in
    // `paldex-data`'s `self_breeding_identifies_exactly_the_producible_species`,
    // which is why this can be a single lookup per species rather than a scan.
    let mut targets: Vec<queries::BreedingTargetView> = reference
        .species_iter()
        .filter(|s| {
            reference
                .breeding_result(&s.character_id, &s.character_id)
                .is_some_and(|child| child.eq_ignore_ascii_case(&s.character_id))
        })
        .map(|s| queries::BreedingTargetView {
            character_id: s.character_id.clone(),
            display_name: s.display_name.clone(),
            dex_label: s.dex_label(),
        })
        .collect();

    // Paldeck order, with anything unnumbered last by name — the same ordering
    // the dex grid uses, so the picker reads like the in-game Paldeck.
    targets.sort_by(|a, b| {
        (a.dex_label.is_none(), &a.dex_label, &a.display_name)
            .cmp(&(b.dex_label.is_none(), &b.dex_label, &b.display_name))
    });
    eprintln!("[paldex] breeding_targets: {} producible species", targets.len());
    targets
}

/// Pairings for `target` that need at least one species the player does not
/// own — the "unowned parents" list behind [`breeding_options`]'s sibling tab.
///
/// Returns an empty list without a pak, like [`breeding_options`]: the
/// breeding table and the species list both live there.
///
/// # Errors
///
/// A display-ready message if no world is selected, or the query fails.
#[tauri::command(async)]
pub fn unowned_breeding_options(
    app: AppHandle,
    state: State<AppState>,
    target: String,
    player_uid: Option<String>,
    include_base_pals: Option<bool>,
) -> Result<Vec<queries::UnownedPairingView>, String> {
    let (views, pals) = analysis_roster(
        &app,
        &state,
        player_scope(player_uid.as_deref()),
        base_pals(include_base_pals),
    )?;
    let Some(reference) = state.reference() else {
        eprintln!("[paldex] unowned_breeding_options: no reference data (no pak found)");
        return Ok(Vec::new());
    };
    Ok(unowned_view(&views, &pals, &target, reference))
}

/// The unowned-pairing search, split from the command for the same reason as
/// [`quality_view`].
fn unowned_view(
    views: &[PalView],
    pals: &[paldex_model::Pal],
    target: &str,
    reference: &ReferenceIndex,
) -> Vec<queries::UnownedPairingView> {
    // The Paldeck, plus anything already owned. Quest and variant forms share
    // a breeding tribe with their base species, so admitting every key in the
    // parameter table would offer the same pairing several times under names
    // no player would recognise — `dex_number` is exactly the "a player would
    // know this one" filter, and it is what the target picker uses.
    //
    // A species the player *owns* is recognisable whether or not it carries a
    // number, though, and leaving those out dropped combinations from every
    // list: this world holds one `PlantSlime_Flower`, which has no Paldeck
    // entry, so its self-pairing was neither breedable (one Pal), nor blocked
    // (not a gender problem), nor here.
    let mut candidates: Vec<&str> = reference
        .species_iter()
        .filter(|s| s.dex_number.is_some())
        .map(|s| s.character_id.as_str())
        .collect();
    candidates.extend(pals.iter().map(|p| p.character_id.as_str()));

    let pairings = paldex_model::unowned_pairings(pals, &candidates, target, |a, b| {
        reference.breeding_result(a, b).map(str::to_owned)
    });

    let by_id = roster_by_id(views);
    let side = |species: &str, owned: Option<&paldex_model::Pal>| {
        let reference_species = reference.species(species);
        queries::PairingSideView {
            character_id: species.to_owned(),
            display_name: reference_species.map(|s| s.display_name.clone()),
            dex_label: reference_species.and_then(paldex_data::Species::dex_label),
            owned: owned.map(|pal| parent_view(pal, &by_id)),
        }
    };

    eprintln!(
        "[paldex] unowned_breeding_options: {} pairings for {target}",
        pairings.len()
    );
    pairings
        .into_iter()
        .map(|p| queries::UnownedPairingView {
            parent_a: side(&p.species_a, p.owned_a),
            parent_b: side(&p.species_b, p.owned_b),
            need: match p.need {
                paldex_model::PairingNeed::SecondSpecimen => "secondSpecimen".to_owned(),
                paldex_model::PairingNeed::OppositeGender => "oppositeGender".to_owned(),
                paldex_model::PairingNeed::OneSpecies => "oneSpecies".to_owned(),
                paldex_model::PairingNeed::TwoSpecies => "twoSpecies".to_owned(),
            },
            // Lowercased to match the gender strings the rest of the app uses,
            // which come from the store rather than the enum's own name.
            blocking_gender: p
                .blocking_gender
                .map(|g| format!("{g:?}").to_ascii_lowercase()),
            missing_species: p
                .missing_species
                .iter()
                .map(|id| {
                    resolve_or_id(reference.species(id).map(|s| s.display_name.as_str()), id)
                })
                .collect(),
        })
        .collect()
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
            queries::pal_roster(&store, snapshot_id, queries::PlayerScope::All, queries::BasePals::Include)
                .ok()?,
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
        let facts = queries::dex_facts(&store, &world.id, snapshot_id, queries::PlayerScope::All)
            .expect("dex_facts");

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

    /// Base pals are in by default, and the player scope has no say in it.
    ///
    /// This replaces a rule that derived the answer from the scope: picking a
    /// player silently dropped unowned base-camp workers from the roster and
    /// analysis tabs while leaving them in breeding. The checkbox is now the
    /// only input, on every tab, which is only true while
    /// `base_pals` ignores the scope entirely — hence a test rather than a
    /// comment.
    #[test]
    fn base_pals_default_to_included_under_every_scope() {
        assert_eq!(base_pals(None), queries::BasePals::Include, "an absent flag is the default");
        assert_eq!(base_pals(Some(true)), queries::BasePals::Include);
        assert_eq!(base_pals(Some(false)), queries::BasePals::Exclude);

        // The signature carries this: `base_pals` cannot see a `PlayerScope`.
        // Both scopes reach it through the same argument.
        for uid in [None, Some("11111111-2222-3333-4444-555555555555")] {
            let scope = player_scope(uid);
            let expected = if uid.is_some() {
                queries::PlayerScope::Only("11111111-2222-3333-4444-555555555555")
            } else {
                queries::PlayerScope::All
            };
            assert_eq!(scope, expected);
            assert_eq!(base_pals(None), queries::BasePals::Include);
        }
    }

    /// Capture progress is per player, and scoping to one must report *that
    /// player's* numbers rather than the world's best.
    ///
    /// The bug this replaces aggregated capture counts without `player_uid`,
    /// allowing one player's progress to inflate another player's results.
    #[test]
    fn per_player_dex_facts_are_not_pooled_across_players() {
        let Some(world) = crate::sync::tests::real_trackable_world() else {
            eprintln!("skipping: need a real save");
            return;
        };
        let mut store = Store::open_in_memory().expect("store");
        let snapshot_id = crate::sync::sync_world(&mut store, &world).expect("sync");

        let players = queries::world_players(&store, snapshot_id).expect("world_players");
        if players.len() < 2 {
            eprintln!("skipping: needs a world with at least two players");
            return;
        }

        let all = queries::dex_facts(&store, &world.id, snapshot_id, queries::PlayerScope::All)
            .expect("all");
        let per_player: Vec<_> = players
            .iter()
            .map(|p| {
                queries::dex_facts(
                    &store,
                    &world.id,
                    snapshot_id,
                    queries::PlayerScope::Only(&p.player_uid),
                )
                .expect("scoped")
            })
            .collect();

        // Every scoped count must be one this player actually has, and the
        // aggregate must be the best of them — never more, never less.
        for (species, pooled) in &all.capture_counts {
            let best = per_player
                .iter()
                .filter_map(|f| f.capture_counts.get(species))
                .copied()
                .max()
                .unwrap_or(0);
            assert_eq!(
                *pooled, best,
                "{species}: All reported {pooled} but no single player has more than {best}"
            );
        }

        // The point of the fix: at least one species where the players differ,
        // proving the scoping is actually doing something on this save.
        let divergent = all
            .capture_counts
            .keys()
            .filter(|species| {
                let counts: Vec<i64> = per_player
                    .iter()
                    .map(|f| f.capture_counts.get(*species).copied().unwrap_or(0))
                    .collect();
                counts.iter().min() != counts.iter().max()
            })
            .count();
        eprintln!(
            "{} of {} species differ between players",
            divergent,
            all.capture_counts.len()
        );
        assert!(
            divergent > 0,
            "two players with identical capture counts for every species means \
             the scoping is not filtering at all"
        );

        // And the headline number the dex screen shows really is per player.
        for (player, facts) in players.iter().zip(&per_player) {
            let complete = facts
                .capture_counts
                .values()
                .filter(|c| **c >= i64::from(paldex_model::CAPTURE_BONUS_AT))
                .count();
            eprintln!("{:?}: {complete} species at the capture bonus", player.name);
        }
    }

    /// The roster's player filter joins on `pals.owner`. Scoping must partition
    /// the roster, with unowned base-camp workers reachable only via `All`.
    #[test]
    fn per_player_roster_partitions_by_owner() {
        let Some(world) = crate::sync::tests::real_trackable_world() else {
            eprintln!("skipping: need a real save");
            return;
        };
        let mut store = Store::open_in_memory().expect("store");
        let snapshot_id = crate::sync::sync_world(&mut store, &world).expect("sync");

        let players = queries::world_players(&store, snapshot_id).expect("world_players");
        let all = queries::pal_roster(
            &store,
            snapshot_id,
            queries::PlayerScope::All,
            queries::BasePals::Include,
        )
        .expect("all");

        let mut scoped_total = 0;
        for p in &players {
            let mine = queries::pal_roster(
                &store,
                snapshot_id,
                queries::PlayerScope::Only(&p.player_uid),
                queries::BasePals::Exclude,
            )
            .expect("scoped");
            assert!(
                mine.iter().all(|pal| pal.owner.as_deref() == Some(p.player_uid.as_str())),
                "a scoped roster must contain only that player's Pals"
            );
            eprintln!("{:?}: {} pals", p.name, mine.len());
            scoped_total += mine.len();
        }

        let unowned = all.iter().filter(|p| p.owner.is_none()).count();
        eprintln!("{} pals total, {unowned} unowned", all.len());
        assert_eq!(
            scoped_total + unowned,
            all.len(),
            "every Pal belongs to exactly one player, or to none"
        );
        assert!(
            unowned > 0,
            "base-camp workers have no owner; if this is 0 the fixture changed"
        );
    }

    /// Phase 1's model-level partition test, re-asserted at the query level:
    /// the bases between them claim every ownerless Pal, exactly once each.
    ///
    /// This is the assertion that catches a break anywhere in the chain —
    /// decode, ingest, or join — rather than only in the decoder.
    #[test]
    fn base_summary_workers_partition_the_unowned_roster() {
        let Some(world) = crate::sync::tests::real_trackable_world() else {
            eprintln!("skipping: need a real save");
            return;
        };
        let mut store = Store::open_in_memory().expect("store");
        let snapshot_id = crate::sync::sync_world(&mut store, &world).expect("sync");

        let bases = queries::base_camp_workers(&store, snapshot_id).expect("base_camp_workers");
        let roster = queries::pal_roster(
            &store,
            snapshot_id,
            queries::PlayerScope::All,
            queries::BasePals::Include,
        )
        .expect("pal_roster");
        let unowned = roster.iter().filter(|v| v.owner.is_none()).count();
        let pals: Vec<paldex_model::Pal> = roster.iter().filter_map(to_model_pal).collect();
        let views: Vec<PalView> = roster
            .into_iter()
            .filter(|v| uuid::Uuid::parse_str(&v.instance_id).is_ok())
            .collect();

        let summary = base_summary_view(&bases, &views, &pals);
        for base in &summary {
            eprintln!("base {} ({}): {} workers", base.number, base.id, base.worker_count);
        }

        let numbers: Vec<u32> = summary.iter().map(|b| b.number).collect();
        let expected: Vec<u32> = (1..=u32::try_from(summary.len()).unwrap()).collect();
        assert_eq!(numbers, expected, "bases should be numbered 1..=n in id order");

        for base in &summary {
            assert_eq!(
                base.worker_count as usize,
                base.workers.len(),
                "worker_count must match the list actually sent",
            );
        }

        let claimed: Vec<&str> =
            summary.iter().flat_map(|b| b.workers.iter().map(|w| w.instance_id.as_str())).collect();
        let distinct: std::collections::HashSet<&str> = claimed.iter().copied().collect();
        assert_eq!(claimed.len(), distinct.len(), "no Pal should work at two bases");
        assert_eq!(
            claimed.len(),
            unowned,
            "the bases should account for every ownerless Pal, and nothing else",
        );
        assert!(unowned > 0, "if this is 0 the fixture changed");
    }

    /// Base-camp Pals have no owner but can still be put in a breeding farm,
    /// so `BasePals::Include` must add them to *every* scope — including a
    /// single player's, where the owner filter would otherwise drop them.
    #[test]
    fn including_base_pals_adds_them_to_every_scope() {
        let Some(world) = crate::sync::tests::real_trackable_world() else {
            eprintln!("skipping: need a real save");
            return;
        };
        let mut store = Store::open_in_memory().expect("store");
        let snapshot_id = crate::sync::sync_world(&mut store, &world).expect("sync");
        let players = queries::world_players(&store, snapshot_id).expect("world_players");

        let roster = |scope, base| {
            queries::pal_roster(&store, snapshot_id, scope, base).expect("pal_roster")
        };

        let all_with = roster(queries::PlayerScope::All, queries::BasePals::Include);
        let all_without = roster(queries::PlayerScope::All, queries::BasePals::Exclude);
        let base_count = all_with.len() - all_without.len();
        eprintln!("{} base pals in this world", base_count);
        assert!(base_count > 0, "expected some unowned base-camp Pals");
        assert!(
            all_without.iter().all(|p| p.owner.is_some()),
            "excluding base pals must leave only owned ones"
        );

        for p in &players {
            let scope = queries::PlayerScope::Only(&p.player_uid);
            let with = roster(scope, queries::BasePals::Include);
            let without = roster(scope, queries::BasePals::Exclude);
            assert_eq!(
                with.len(),
                without.len() + base_count,
                "a player scope with base pals should be their own plus every base pal"
            );
            assert!(
                with.iter().all(|pal| pal.owner.is_none()
                    || pal.owner.as_deref() == Some(p.player_uid.as_str())),
                "no other player's Pals should leak in"
            );
        }
    }

    /// The real roster in both shapes `analysis_roster` produces, without a
    /// running Tauri app to hand it a store.
    fn real_analysis_roster() -> Option<(Vec<PalView>, Vec<paldex_model::Pal>, LoadedReference)> {
        let (Some((mut views, _)), Some(loaded)) = (real_snapshot(), load_reference()) else {
            return None;
        };
        enrich_roster(&mut views, &loaded.index);
        let pals: Vec<paldex_model::Pal> = views.iter().filter_map(to_model_pal).collect();
        views.retain(|v| uuid::Uuid::parse_str(&v.instance_id).is_ok());
        assert_eq!(views.len(), pals.len(), "the two shapes must stay parallel");
        Some((views, pals, loaded))
    }

    #[test]
    fn quality_view_grades_the_real_roster() {
        let Some((views, pals, _)) = real_analysis_roster() else {
            eprintln!("skipping: need both a real save and the game pak");
            return;
        };

        let view = quality_view(&views, &pals);
        eprintln!(
            "quality: {} graded, {} best-of-species, {} condense candidates",
            view.graded.len(),
            view.best_of_species.len(),
            view.condense_candidates.len()
        );

        assert_eq!(view.graded.len(), pals.len(), "every owned Pal is graded");
        assert!(!view.graded.is_empty(), "the real world has Pals");

        // The headline ordering. A screen that claims "best first" and isn't
        // is worse than an unsorted one.
        for pair in view.graded.windows(2) {
            assert!(
                pair[0].composite >= pair[1].composite,
                "graded must be sorted best-first: {} then {}",
                pair[0].composite,
                pair[1].composite
            );
        }

        let ids: std::collections::HashSet<&str> =
            view.graded.iter().map(|g| g.instance_id.as_str()).collect();
        for list in [&view.best_of_species, &view.condense_candidates, &view.passive_ranking] {
            for id in list {
                assert!(ids.contains(id.as_str()), "{id} is not in the graded roster");
            }
        }

        // The plan's criterion, restated against real data: condensing must
        // never eat the specimen worth keeping.
        let best: std::collections::HashSet<&String> = view.best_of_species.iter().collect();
        assert!(
            !view.condense_candidates.iter().any(|id| best.contains(id)),
            "a best-of-species Pal must never be condense fodder"
        );
        assert_eq!(view.passive_ranking.len(), view.graded.len());

        // The composite is a mean of three 0..=100 talents, so it shares their
        // range -- and being the only quality number the UI shows now that the
        // tiers are gone, a nonsense value would go unnoticed.
        for g in &view.graded {
            assert!(
                (0.0..=100.0).contains(&g.composite),
                "composite {} out of range for {}",
                g.composite,
                g.character_id
            );
        }
    }

    /// The target is derived from the roster rather than hardcoded: whatever
    /// two owned species actually produce must come back as a suggestion for
    /// that child. Hardcoding a species would make this test a hostage to
    /// which Pals happen to be in the save.
    #[test]
    fn breeding_view_finds_owned_pairs_for_a_reachable_target() {
        let Some((views, pals, loaded)) = real_analysis_roster() else {
            eprintln!("skipping: need both a real save and the game pak");
            return;
        };
        let index = &loaded.index;

        let mut species: Vec<&str> = pals.iter().map(|p| p.character_id.as_str()).collect();
        species.sort_unstable();
        species.dedup();

        // A pair of *different* owned species that breed, so the target is
        // reachable by something other than self-breeding.
        let Some((a, b, target)) = species.iter().enumerate().find_map(|(i, a)| {
            species[i + 1..]
                .iter()
                .find_map(|b| index.breeding_result(a, b).map(|c| (*a, *b, c.to_owned())))
        }) else {
            eprintln!("skipping: no two owned species breed");
            return;
        };
        eprintln!("breeding: {a} + {b} = {target}");

        let pairs = breeding_view(&views, &pals, &target, index);
        eprintln!("{} owned pairs produce {target}", pairs.len());
        assert!(!pairs.is_empty(), "{a} + {b} are both owned and give {target}");

        let owned: std::collections::HashSet<&str> =
            views.iter().map(|v| v.instance_id.as_str()).collect();
        for pair in &pairs {
            assert!(owned.contains(pair.parent_a.instance_id.as_str()), "parent A must be owned");
            assert!(owned.contains(pair.parent_b.instance_id.as_str()), "parent B must be owned");
            assert_ne!(
                pair.parent_a.instance_id, pair.parent_b.instance_id,
                "a Pal cannot breed with itself"
            );
            assert!(
                !(pair.parent_a.gender == pair.parent_b.gender && pair.parent_a.gender != "unknown"),
                "a farm needs one male and one female, got two {}s",
                pair.parent_a.gender
            );
            // The suggested pair must genuinely produce what was asked for.
            let child = index
                .breeding_result(&pair.parent_a.character_id, &pair.parent_b.character_id)
                .unwrap_or_default();
            assert!(
                child.eq_ignore_ascii_case(&target),
                "{} + {} gives {child}, not {target}",
                pair.parent_a.character_id,
                pair.parent_b.character_id
            );
        }

        for pair in pairs.windows(2) {
            assert!(pair[0].score >= pair[1].score, "pairs must be ranked best-first");
        }

        assert!(
            breeding_view(&views, &pals, "NotASpecies", index).is_empty(),
            "an unreachable target yields nothing rather than a guess"
        );
    }

    /// The "unowned parents" list must never suggest something the owned list
    /// already covers, and must never call an owned species missing.
    #[test]
    fn unowned_view_offers_only_combinations_needing_a_species_you_lack() {
        let Some((views, pals, loaded)) = real_analysis_roster() else {
            eprintln!("skipping: need both a real save and the game pak");
            return;
        };
        let index = &loaded.index;

        let owned: std::collections::HashSet<String> =
            pals.iter().map(|p| p.character_id.to_ascii_lowercase()).collect();

        // A target the roster cannot already breed is where this list earns
        // its keep, so prefer one; fall back to any reachable species.
        //
        // Sorted rather than taken in `species_iter` order: that order is not
        // stable between runs, and picking a different target each time made
        // this test pass or fail depending on which needs the chosen species
        // happened to exercise.
        let mut candidates: Vec<String> = index
            .species_iter()
            .filter(|s| s.dex_number.is_some())
            .map(|s| s.character_id.clone())
            .collect();
        candidates.sort();
        let target = candidates.into_iter().find(|t| {
            breeding_view(&views, &pals, t, index).is_empty()
                && !unowned_view(&views, &pals, t, index).is_empty()
        });
        let Some(target) = target else {
            eprintln!("skipping: the roster can already breed everything reachable");
            return;
        };

        let pairings = unowned_view(&views, &pals, &target, index);
        eprintln!(
            "{}: {} unowned pairings, and no owned pair at all",
            target,
            pairings.len()
        );
        assert!(!pairings.is_empty());

        for p in &pairings {
            let label = format!("{} + {}", p.parent_a.character_id, p.parent_b.character_id);
            // Every row must be waiting on something. Which "something" decides
            // what the rest of the row is allowed to look like: only the
            // species-missing needs populate `missing_species`, and only they
            // leave a side genuinely unowned.
            match p.need.as_str() {
                "oneSpecies" | "twoSpecies" => {
                    assert!(!p.missing_species.is_empty(), "{label}: must name what is missing");
                    assert!(
                        p.parent_a.owned.is_none() || p.parent_b.owned.is_none(),
                        "{label}: at least one side must be the species you lack"
                    );
                    // An owned side is owned, an unowned side is not.
                    for side in [&p.parent_a, &p.parent_b] {
                        let is_owned = owned.contains(&side.character_id.to_ascii_lowercase());
                        assert_eq!(
                            side.owned.is_some(),
                            is_owned,
                            "{} is {} but was reported otherwise",
                            side.character_id,
                            if is_owned { "owned" } else { "unowned" }
                        );
                    }
                }
                "secondSpecimen" => {
                    assert!(
                        p.parent_a.character_id.eq_ignore_ascii_case(&p.parent_b.character_id),
                        "{label}: a second specimen is only ever a species with itself"
                    );
                    assert!(p.parent_a.owned.is_some(), "{label}: the one you have is shown");
                    assert!(p.parent_b.owned.is_none(), "{label}: the slot to fill is empty");
                    assert!(
                        owned.contains(&p.parent_a.character_id.to_ascii_lowercase()),
                        "{label}: the species itself is owned — only a second one is missing"
                    );
                }
                "oppositeGender" => {
                    assert!(
                        p.parent_a.owned.is_some() && p.parent_b.owned.is_some(),
                        "{label}: both sides are owned; only the gender is missing"
                    );
                    assert!(
                        p.missing_species.is_empty(),
                        "{label}: no species is missing from the roster here"
                    );
                }
                other => panic!("{label}: unexpected need {other}"),
            }
            // The pairing must genuinely produce what was asked for.
            let child = index
                .breeding_result(&p.parent_a.character_id, &p.parent_b.character_id)
                .unwrap_or_default();
            assert!(
                child.eq_ignore_ascii_case(&target),
                "{} + {} gives {child}, not {target}",
                p.parent_a.character_id,
                p.parent_b.character_id
            );
        }

        for w in pairings.windows(2) {
            assert!(
                w[0].missing_species.len() <= w[1].missing_species.len(),
                "pairings needing fewer new species must come first"
            );
        }

        assert!(
            unowned_view(&views, &pals, "NotASpecies", index).is_empty(),
            "an unreachable target yields nothing rather than a guess"
        );
    }

    /// Against the real save: the two lists partition the combinations that
    /// produce a target. Every combination is breedable today or waiting on a
    /// Pal that isn't in the box — never both, never neither.
    ///
    /// This is what stops a combination quietly vanishing from the UI, which
    /// is exactly what gender-stuck pairings did before they were surfaced.
    #[test]
    fn the_two_breeding_lists_partition_every_combination() {
        let Some((views, pals, loaded)) = real_analysis_roster() else {
            eprintln!("skipping: need both a real save and the game pak");
            return;
        };
        let index = &loaded.index;

        // Case-folded, because the save spells one species several ways
        // (`GhostAnglerFish` / `GhostAnglerfish`) and the analysis layer folds
        // them into one group. Counting the raw ids would expect combinations
        // that are really the same one twice over.
        let mut owned_species: Vec<String> =
            pals.iter().map(|p| p.character_id.to_ascii_lowercase()).collect();
        owned_species.sort_unstable();
        owned_species.dedup();

        let key = |a: &str, b: &str| {
            let mut pair = [a.to_ascii_lowercase(), b.to_ascii_lowercase()];
            pair.sort();
            pair
        };

        let mut reachable: Vec<String> = owned_species
            .iter()
            .enumerate()
            .flat_map(|(i, a)| {
                owned_species[i..]
                    .iter()
                    .filter_map(|b| index.breeding_result(a, b).map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .collect();
        reachable.sort();
        reachable.dedup();
        let sample: Vec<&String> = reachable.iter().step_by(reachable.len() / 6).take(6).collect();

        for target in sample {
            let breedable: Vec<_> = breeding_view(&views, &pals, target, index)
                .iter()
                .map(|p| key(&p.parent_a.character_id, &p.parent_b.character_id))
                .collect();
            let waiting: Vec<_> = unowned_view(&views, &pals, target, index)
                .iter()
                .map(|p| key(&p.parent_a.character_id, &p.parent_b.character_id))
                .collect();

            let breedable_set: std::collections::HashSet<_> = breedable.iter().collect();
            let waiting_set: std::collections::HashSet<_> = waiting.iter().collect();

            // Every owned-species combination producing the target must be in
            // exactly one of the two lists.
            let mut expected = 0usize;
            for (i, a) in owned_species.iter().enumerate() {
                for b in &owned_species[i..] {
                    if index.breeding_result(a, b).is_some_and(|c| c.eq_ignore_ascii_case(target)) {
                        expected += 1;
                        let k = key(a, b);
                        assert!(
                            breedable_set.contains(&k) != waiting_set.contains(&k),
                            "{target}: {a} + {b} must be in exactly one list \
                             (breedable: {}, waiting: {})",
                            breedable_set.contains(&k),
                            waiting_set.contains(&k)
                        );
                    }
                }
            }

            // `waiting` also holds combinations involving species the roster
            // lacks, so it is only bounded below by the owned-combination count.
            let owned_waiting = waiting
                .iter()
                .filter(|k| {
                    owned_species.contains(&k[0]) && owned_species.contains(&k[1])
                })
                .count();
            eprintln!(
                "{target}: {expected} owned combinations = {} breedable + {owned_waiting} waiting \
                 ({} waiting rows in total)",
                breedable.len(),
                waiting.len()
            );
            assert_eq!(
                breedable.len() + owned_waiting,
                expected,
                "{target}: the two lists must cover every owned combination exactly once"
            );
        }
    }

    /// The save spells some species more than one way, and the analysis layer
    /// has to treat those as one species.
    ///
    /// This world holds `GhostAnglerFish` *and* `GhostAnglerfish`, plus
    /// `SheepBall` and `Sheepball`. Grouping on the exact string offered the
    /// same combination twice and, worse, reported pairings as gender-blocked
    /// while the mate sat under the other spelling.
    #[test]
    fn species_spelled_two_ways_are_treated_as_one() {
        let Some((views, pals, loaded)) = real_analysis_roster() else {
            eprintln!("skipping: need both a real save and the game pak");
            return;
        };

        let mut by_lower: HashMap<String, std::collections::BTreeSet<String>> = HashMap::new();
        for p in &pals {
            by_lower
                .entry(p.character_id.to_ascii_lowercase())
                .or_default()
                .insert(p.character_id.clone());
        }
        let variants: Vec<_> = by_lower.values().filter(|s| s.len() > 1).collect();
        eprintln!("species ids differing only by case: {variants:?}");
        assert!(
            !variants.is_empty(),
            "this world is known to spell species inconsistently; without that \
             the rest of this test proves nothing"
        );

        // No target may offer the same species combination twice.
        let target = variants[0].iter().next().expect("a variant species");
        let child = loaded
            .index
            .breeding_result(target, target)
            .expect("a breedable species")
            .to_owned();
        let pairs = breeding_view(&views, &pals, &child, &loaded.index);

        let mut seen = std::collections::HashSet::new();
        for p in &pairs {
            let mut k = [
                p.parent_a.character_id.to_ascii_lowercase(),
                p.parent_b.character_id.to_ascii_lowercase(),
            ];
            k.sort();
            assert!(
                seen.insert(k.clone()),
                "{child}: {k:?} offered twice — case variants were not folded"
            );
        }
        eprintln!("{child}: {} pairs, all distinct species combinations", pairs.len());
    }

    /// The target picker must only offer species breeding can actually produce.
    ///
    /// Two kinds have to be absent: the ones the game bars from breeding, and
    /// variant forms shadowed by their base species. Both would otherwise sit
    /// in the list promising a result that no pairing can deliver.
    #[test]
    fn the_target_picker_offers_only_producible_species() {
        let Some(loaded) = load_reference() else {
            eprintln!("skipping: need the game pak");
            return;
        };
        let index = &loaded.index;

        let targets: std::collections::HashSet<String> = index
            .species_iter()
            .filter(|s| {
                index
                    .breeding_result(&s.character_id, &s.character_id)
                    .is_some_and(|c| c.eq_ignore_ascii_case(&s.character_id))
            })
            .map(|s| s.character_id.to_ascii_lowercase())
            .collect();
        eprintln!("{} producible species offered as targets", targets.len());
        assert!(targets.len() > 200, "most species should be breedable into");

        // No pairing produces these, so they cannot be asked for.
        for id in [
            "KingWhale",                   // Panthalus
            "WorldTreeDragon",             // Astralym
            "RAID_YakushimaBoss001_Green", // True Eye of Cthulhu
            "RAID_YakushimaBoss002",       // Moon Lord
        ] {
            assert!(
                !targets.contains(&id.to_ascii_lowercase()),
                "{id} cannot be bred into, so it must not be offered as a target"
            );
        }

        // The legendaries must survive the filter. They sit outside the generic
        // candidate pool, which is easy to mistake for "cannot breed" — doing
        // so silently drops a third of the endgame from the picker.
        for (id, name) in [
            ("IceHorse", "Frostallion"),
            ("IceHorse_Dark", "Frostallion Noct"),
            ("JetDragon", "Jetragon"),
            ("SaintCentaur", "Paladius"),
            ("BlackCentaur", "Necromus"),
            ("NightLady", "Bellanoir"),
        ] {
            assert!(
                targets.contains(&id.to_ascii_lowercase()),
                "{name} breeds true through a unique combo and must be offered"
            );
            assert!(
                index.breeding_result(id, "SheepBall").is_some(),
                "{name} must also still work as a parent"
            );
        }

        // Breedable as a parent, but nothing produces them: a pair of these
        // yields the base species they are shadowed by. This is the case that
        // must *not* be lumped in with the barred ones — they still breed.
        for (id, base) in [
            ("PlantSlime_Flower", "PlantSlime"),
            ("Quest_Farmer03_SheepBall", "SheepBall"),
            ("Quest_Farmer03_PinkCat", "PinkCat"),
        ] {
            assert!(
                !targets.contains(&id.to_ascii_lowercase()),
                "{id} is never produced by breeding, so it must not be a target"
            );
            assert_eq!(
                index.breeding_result(id, id).map(str::to_ascii_lowercase),
                Some(base.to_ascii_lowercase()),
                "{id} + {id} should give {base}, which is what makes it unproducible"
            );
        }

        // The everyday case must survive the filter.
        for id in ["SheepBall", "ChickenPal", "Anubis", "PinkCat"] {
            assert!(
                targets.contains(&id.to_ascii_lowercase()),
                "{id} is breedable and must still be offered"
            );
        }
    }

    /// Every owned species must be a candidate parent in the acquisition list,
    /// whether or not it carries a Paldeck number.
    ///
    /// The unowned search draws its universe from the Paldeck, so a species
    /// owned but *not* in it — a quest or variant form — would never appear.
    /// That matters most for the second-specimen case: owning exactly one of
    /// such a species would drop its self-pairing out of every list.
    #[test]
    fn owned_species_outside_the_paldeck_are_still_candidates() {
        let Some((views, pals, loaded)) = real_analysis_roster() else {
            eprintln!("skipping: need both a real save and the game pak");
            return;
        };
        let index = &loaded.index;

        let paldeck: std::collections::HashSet<String> = index
            .species_iter()
            .filter(|s| s.dex_number.is_some())
            .map(|s| s.character_id.to_ascii_lowercase())
            .collect();

        let mut counts: HashMap<String, usize> = HashMap::new();
        for p in &pals {
            *counts.entry(p.character_id.to_ascii_lowercase()).or_default() += 1;
        }

        let outside: Vec<&String> = counts.keys().filter(|k| !paldeck.contains(*k)).collect();
        eprintln!("{} owned species carry no Paldeck number: {outside:?}", outside.len());

        // Any owned species with exactly one specimen whose self-pairing
        // produces something must show up as a second-specimen need.
        for (species, _) in counts.iter().filter(|(_, n)| **n == 1) {
            let Some(child) = index.breeding_result(species, species) else {
                continue;
            };
            let child = child.to_owned();
            let found = unowned_view(&views, &pals, &child, index).into_iter().any(|p| {
                p.need == "secondSpecimen"
                    && p.parent_a.character_id.eq_ignore_ascii_case(species)
            });
            assert!(
                found,
                "{species} (one owned, breeds into {child}) is missing from the \
                 acquisition list — it is in neither the blocked nor the unowned tab"
            );
        }
    }

    /// A combination reported as waiting on the opposite gender must really
    /// have no usable pair, and the named gender must be the one every owned
    /// candidate shares.
    #[test]
    fn gender_stuck_combinations_really_have_no_usable_pair() {
        let Some((views, pals, loaded)) = real_analysis_roster() else {
            eprintln!("skipping: need both a real save and the game pak");
            return;
        };
        let index = &loaded.index;

        let mut checked = 0usize;
        for target in index
            .species_iter()
            .filter(|s| s.dex_number.is_some())
            .map(|s| s.character_id.clone())
        {
            for p in unowned_view(&views, &pals, &target, index) {
                if p.need != "oppositeGender" {
                    continue;
                }
                checked += 1;
                let (a, b) = (&p.parent_a.character_id, &p.parent_b.character_id);

                // Every owned individual of each side, not a sample.
                let side = |id: &String| {
                    pals.iter()
                        .filter(|x| x.character_id.eq_ignore_ascii_case(id))
                        .collect::<Vec<_>>()
                };
                let (side_a, side_b) = (side(a), side(b));
                assert!(
                    !side_a.is_empty() && !side_b.is_empty(),
                    "{a} + {b}: both species must be owned for a gender problem"
                );
                assert!(
                    p.parent_a.owned.is_some() && p.parent_b.owned.is_some(),
                    "{a} + {b}: both sides must show the specimen already owned"
                );

                let usable = side_a.iter().any(|x| {
                    side_b.iter().any(|y| {
                        x.instance_id != y.instance_id
                            && !matches!(
                                (x.gender, y.gender),
                                (paldex_model::Gender::Male, paldex_model::Gender::Male)
                                    | (paldex_model::Gender::Female, paldex_model::Gender::Female)
                            )
                    })
                });
                assert!(!usable, "{target}: {a} + {b} is stuck but a usable pair exists");

                let gender = p.blocking_gender.as_deref().unwrap_or_default();
                assert!(
                    ["male", "female"].contains(&gender),
                    "{a} + {b}: expected a real gender, got {gender:?}"
                );
                assert!(
                    side_a
                        .iter()
                        .chain(side_b.iter())
                        .all(|x| format!("{:?}", x.gender).to_ascii_lowercase() == gender),
                    "{a} + {b}: not every owned candidate is {gender}"
                );
            }
        }
        eprintln!("{checked} gender-stuck combinations verified against the full roster");
        assert!(checked > 0, "this world is known to have gender-stuck combinations");
    }

    /// Picking a different species must actually change the answer.
    ///
    /// The per-pair assertions in the test above would all still pass if the
    /// target filter were broken in a way that returned every breedable pair
    /// for everything, so this pins the property those checks cannot see: two
    /// different targets give different pair sets, and no target returns the
    /// whole cross product.
    #[test]
    fn different_targets_give_different_pairs() {
        let Some((views, pals, loaded)) = real_analysis_roster() else {
            eprintln!("skipping: need both a real save and the game pak");
            return;
        };
        let index = &loaded.index;

        let mut species: Vec<&str> = pals.iter().map(|p| p.character_id.as_str()).collect();
        species.sort_unstable();
        species.dedup();

        // Every distinct child the owned roster can actually produce.
        let mut targets: Vec<String> = species
            .iter()
            .enumerate()
            .flat_map(|(i, a)| {
                species[i..]
                    .iter()
                    .filter_map(|b| index.breeding_result(a, b).map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .collect();
        targets.sort();
        targets.dedup();
        eprintln!("{} distinct reachable targets from {} owned species", targets.len(), species.len());
        assert!(targets.len() > 20, "a 296-species roster should reach many children");

        // The total number of species pairs that breed at all — the number a
        // broken filter would return for every target.
        let all_breedable = species
            .iter()
            .enumerate()
            .map(|(i, a)| {
                species[i..]
                    .iter()
                    .filter(|b| index.breeding_result(a, b).is_some())
                    .count()
            })
            .sum::<usize>();

        let sample: Vec<&String> = targets.iter().step_by(targets.len() / 8).take(8).collect();
        let mut signatures = Vec::new();
        for target in &sample {
            let pairs = breeding_view(&views, &pals, target, index);
            let signature: Vec<(String, String)> = pairs
                .iter()
                .map(|p| (p.parent_a.instance_id.clone(), p.parent_b.instance_id.clone()))
                .collect();
            eprintln!("{target}: {} pairs", signature.len());
            assert!(
                signature.len() < all_breedable,
                "{target} returned {} of {all_breedable} breedable pairs — the target filter is not filtering",
                signature.len()
            );
            signatures.push((target.to_string(), signature));
        }

        for (i, (a, sig_a)) in signatures.iter().enumerate() {
            for (b, sig_b) in &signatures[i + 1..] {
                assert_ne!(sig_a, sig_b, "{a} and {b} returned an identical pair list");
            }
        }
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
            let facts = queries::dex_facts(&store, &world.id, snapshot_id, queries::PlayerScope::All).ok()?;
            let players = queries::player_progress(&store, snapshot_id).ok()?;
            let summary = queries::snapshot_summary(&store, snapshot_id).ok()?;
            let bases = queries::base_camp_workers(&store, snapshot_id).ok()?;
            Some((dex_with_reference(&facts, &loaded.index), players, summary, bases))
        });

        // Roster species *plus* every dex entry: the dex grid shows uncaught
        // species too, and those never appear in the roster. Dumping only
        // roster species left the "missing" tiles artwork-less in the harness
        // while the real app renders them fine — a preview that lies about the
        // screen is worse than no preview.
        let mut species: std::collections::BTreeSet<String> =
            roster.iter().map(|p| p.character_id.clone()).collect();
        if let Some((dex, _, _, _)) = &derived {
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

        // The analysis tab. `breeding_options` takes a target, so it gets one
        // fixture per target — *every* species the owned roster can actually
        // breed, not a sample. An earlier version captured eight and let the
        // harness fall back to one of them for everything else, which showed
        // one species' pairs under another species' name: the preview has to
        // answer per target or it is worse than no preview. A target with no
        // file is one nothing owned can produce, and the harness reads that
        // absence as the empty list the real command returns.
        let pal_count = roster.len();
        let pals: Vec<paldex_model::Pal> = roster.iter().filter_map(to_model_pal).collect();
        let views: Vec<PalView> =
            roster.into_iter().filter(|v| uuid::Uuid::parse_str(&v.instance_id).is_ok()).collect();
        write("pal_quality", serde_json::to_value(quality_view(&views, &pals)).unwrap());

        let mut owned_species: Vec<&str> = pals.iter().map(|p| p.character_id.as_str()).collect();
        owned_species.sort_unstable();
        owned_species.dedup();
        let mut targets: Vec<String> = owned_species
            .iter()
            .enumerate()
            .flat_map(|(i, a)| {
                owned_species[i..]
                    .iter()
                    .filter_map(|b| loaded.index.breeding_result(a, b).map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .collect();
        targets.sort();
        targets.dedup();
        // The "include base pals" checkbox is a second axis, so each target is
        // captured both ways. Base-camp Pals have no owner but can still be
        // bred, so unchecking it genuinely changes the pairs — a harness that
        // answered the same either way would misrepresent the control.
        let no_base: Vec<PalView> =
            views.iter().filter(|v| v.owner.is_some()).cloned().collect();
        let no_base_pals: Vec<paldex_model::Pal> =
            no_base.iter().filter_map(to_model_pal).collect();

        // The roster and analysis tabs read that same axis, so they need the
        // unticked capture too. Their default files were written above from the
        // base-inclusive roster, which is what the checkbox starts on.
        write("pal_roster__nobase", serde_json::to_value(&no_base).unwrap());
        write(
            "pal_quality__nobase",
            serde_json::to_value(quality_view(&no_base, &no_base_pals)).unwrap(),
        );

        let mut pair_total = 0usize;
        for target in &targets {
            let pairs = breeding_view(&views, &pals, target, &loaded.index);
            pair_total += pairs.len();
            write(&format!("breeding_options__{target}"), serde_json::to_value(&pairs).unwrap());
            write(
                &format!("breeding_options__{target}__nobase"),
                serde_json::to_value(breeding_view(&no_base, &no_base_pals, target, &loaded.index))
                    .unwrap(),
            );
        }
        eprintln!("fixture: {} breeding targets, {pair_total} pairs total", targets.len());

        // The unowned-parents tab. Its universe is every species the picker can
        // offer rather than the roster, so it answers for targets the owned
        // list cannot reach at all — the whole reason the tab exists.
        // Same filter as `breeding_targets`, which cannot be called here
        // because it needs Tauri state the test has no way to build.
        let producible: Vec<&paldex_data::Species> = loaded
            .index
            .species_iter()
            .filter(|s| {
                loaded
                    .index
                    .breeding_result(&s.character_id, &s.character_id)
                    .is_some_and(|c| c.eq_ignore_ascii_case(&s.character_id))
            })
            .collect();
        let mut picker: Vec<queries::BreedingTargetView> = producible
            .iter()
            .map(|s| queries::BreedingTargetView {
                character_id: s.character_id.clone(),
                display_name: s.display_name.clone(),
                dex_label: s.dex_label(),
            })
            .collect();
        picker.sort_by(|a, b| {
            (a.dex_label.is_none(), &a.dex_label, &a.display_name)
                .cmp(&(b.dex_label.is_none(), &b.dex_label, &b.display_name))
        });
        eprintln!("fixture: {} producible targets in the picker", picker.len());
        write("breeding_targets", serde_json::to_value(&picker).unwrap());

        let unowned_targets: Vec<String> =
            producible.iter().map(|s| s.character_id.clone()).collect();
        let mut unowned_total = 0usize;
        let mut unowned_files = 0usize;
        for target in &unowned_targets {
            let pairings = unowned_view(&views, &pals, target, &loaded.index);
            if pairings.is_empty() {
                continue;
            }
            unowned_total += pairings.len();
            unowned_files += 1;
            write(
                &format!("unowned_breeding_options__{target}"),
                serde_json::to_value(&pairings).unwrap(),
            );
            let without = unowned_view(&no_base, &no_base_pals, target, &loaded.index);
            if !without.is_empty() {
                write(
                    &format!("unowned_breeding_options__{target}__nobase"),
                    serde_json::to_value(&without).unwrap(),
                );
            }
        }
        eprintln!("fixture: {unowned_files} unowned targets, {unowned_total} pairings total");

        if let Some((dex, players, summary, bases)) = &derived {
            write("dex_progress", serde_json::to_value(dex).unwrap());
            write("player_progress", serde_json::to_value(players).unwrap());
            // The Bases tab. One file, not twelve: `base_summary` takes
            // neither the player nor the base-pals axis, because a base
            // belongs to the guild rather than to a player.
            let base_view = base_summary_view(bases, &views, &pals);
            let worker_total: u32 = base_view.iter().map(|b| b.worker_count).sum();
            write("base_summary", serde_json::to_value(&base_view).unwrap());
            eprintln!("fixture: {} bases, {worker_total} workers total", base_view.len());
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

        // The player selector. Off by default: every screen is per player, so
        // an honest per-player preview means re-capturing the per-target
        // breeding dump once per player, which multiplies an already ~34 MB,
        // ~3 minute capture by the player count. Without the flag the harness
        // gets an empty player list, so the selector doesn't render and the
        // preview shows the world-level view — the same thing the real app
        // shows for a single-player world, rather than a selector whose
        // options all resolve to the same fixtures.
        let players = std::env::var("PALDEX_FIXTURE_PLAYERS").is_ok();
        let world_players = if players {
            crate::sync::tests::real_trackable_world()
                .and_then(|world| {
                    let mut store = Store::open_in_memory().ok()?;
                    let snapshot_id = crate::sync::sync_world(&mut store, &world).ok()?;
                    let listed = queries::world_players(&store, snapshot_id).ok()?;
                    for p in &listed {
                        dump_for_player(&write, &world, snapshot_id, p, &loaded)?;
                    }
                    Some(listed)
                })
                .unwrap_or_default()
        } else {
            eprintln!("fixture: player selector omitted (set PALDEX_FIXTURE_PLAYERS=1 to capture it)");
            Vec::new()
        };
        write("world_players", serde_json::to_value(&world_players).unwrap());

        eprintln!("fixture: {pal_count} pals, {} icons -> {out}", species.len());
    }

    /// Everything the four tabs request for one selected player, under the
    /// `__player_<uid>` suffix `preview/mock-core.ts` looks for.
    fn dump_for_player(
        write: &impl Fn(&str, serde_json::Value),
        world: &paldex_locate::World,
        snapshot_id: i64,
        player: &queries::WorldPlayerView,
        loaded: &LoadedReference,
    ) -> Option<()> {
        let mut store = Store::open_in_memory().ok()?;
        let snapshot_id_check = crate::sync::sync_world(&mut store, world).ok()?;
        debug_assert_eq!(snapshot_id, snapshot_id_check);

        let uid = &player.player_uid;
        let scope = queries::PlayerScope::Only(uid);
        let suffix = format!("__player_{uid}");

        let facts = queries::dex_facts(&store, &world.id, snapshot_id_check, scope).ok()?;
        write(
            &format!("dex_progress{suffix}"),
            serde_json::to_value(dex_with_reference(&facts, &loaded.index)).unwrap(),
        );

        // Every tab reads the same two axes now — the selected player, and the
        // "include base pals" checkbox — so each roster-derived fixture is
        // captured both ways. The unsuffixed file is the checkbox's default
        // (on), and `__nobase` is the unticked state.
        let scoped_roster = |base| -> Option<(Vec<PalView>, Vec<paldex_model::Pal>)> {
            let mut rows = queries::pal_roster(&store, snapshot_id_check, scope, base).ok()?;
            enrich_roster(&mut rows, &loaded.index);
            let pals: Vec<paldex_model::Pal> = rows.iter().filter_map(to_model_pal).collect();
            let views: Vec<PalView> = rows
                .into_iter()
                .filter(|v| uuid::Uuid::parse_str(&v.instance_id).is_ok())
                .collect();
            Some((views, pals))
        };
        let (views, pals) = scoped_roster(queries::BasePals::Exclude)?;
        let (base_views, base_pals_model) = scoped_roster(queries::BasePals::Include)?;

        for (variant, v, p) in [("", &base_views, &base_pals_model), ("__nobase", &views, &pals)] {
            write(&format!("pal_roster{variant}{suffix}"), serde_json::to_value(v).unwrap());
            write(
                &format!("pal_quality{variant}{suffix}"),
                serde_json::to_value(quality_view(v, p)).unwrap(),
            );
        }

        // Breeding is per target, per player *and* per checkbox state, which
        // is the combination that makes this capture expensive. Targets are
        // taken from the wider roster so nothing reachable with base Pals is
        // missing a file; a target with no file is one neither pool can reach,
        // and resolves to the empty list as in the unscoped case.
        let mut owned: Vec<&str> =
            base_pals_model.iter().map(|p| p.character_id.as_str()).collect();
        owned.sort_unstable();
        owned.dedup();
        let mut targets: Vec<String> = owned
            .iter()
            .enumerate()
            .flat_map(|(i, a)| {
                owned[i..]
                    .iter()
                    .filter_map(|b| loaded.index.breeding_result(a, b).map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .collect();
        targets.sort();
        targets.dedup();
        for target in &targets {
            for (variant, v, p) in [
                ("", &base_views, &base_pals_model),
                ("__nobase", &views, &pals),
            ] {
                write(
                    &format!("breeding_options__{target}{variant}{suffix}"),
                    serde_json::to_value(breeding_view(v, p, target, &loaded.index)).unwrap(),
                );
                let pairings = unowned_view(v, p, target, &loaded.index);
                if !pairings.is_empty() {
                    write(
                        &format!("unowned_breeding_options__{target}{variant}{suffix}"),
                        serde_json::to_value(&pairings).unwrap(),
                    );
                }
            }
        }
        eprintln!(
            "fixture: player {:?} -> {} pals, {} breeding targets",
            player.name,
            views.len(),
            targets.len()
        );
        Some(())
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

