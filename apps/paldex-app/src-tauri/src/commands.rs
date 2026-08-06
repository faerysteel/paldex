//! Commands exposed to the frontend.

use std::path::PathBuf;

use paldex_locate::{discover, resolve_manual, SaveRoot, World};
use serde::Serialize;

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
