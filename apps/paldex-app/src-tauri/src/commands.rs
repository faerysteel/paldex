//! Commands exposed to the frontend.

use std::path::PathBuf;
use std::sync::Mutex;

use paldex_locate::{discover, resolve_manual, SaveRoot, World};
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

/// The currently selected world's full Pal roster.
///
/// # Errors
///
/// A display-ready message if no world is selected, or the query fails.
#[tauri::command]
pub fn pal_roster(app: AppHandle, state: State<AppState>) -> Result<Vec<PalView>, String> {
    let snapshot_id = selected_snapshot_id(&state)?;
    with_store(&app, &state, |store| queries::pal_roster(store, snapshot_id))
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
