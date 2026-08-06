//! Dev-only: classify every unlocked technology in the real save that fails to
//! resolve to a display name, so the residual is understood rather than
//! hand-waved.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use paldex_data::text_table::{self, TextRef};

fn main() {
    let pak_path = std::env::args().nth(1).expect("usage: classify_unresolved_tech <pak> [lang]");
    let lang = std::env::args().nth(2).unwrap_or_else(|| "en".to_string());
    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");
    let index = paldex_data::ReferenceIndex::extract(&mut pak, &lang).expect("extract");

    // Raw rows of the technology name table, keyed by tech id.
    let root = if lang == "ja" {
        "Pal/Content/Pal/DataTable/Text".to_string()
    } else {
        format!("Pal/Content/L10N/{lang}/Pal/DataTable/Text")
    };
    let raw = pak
        .read(&format!("{root}/DT_TechnologyNameText_Common.uexp"))
        .expect("read tech table");
    let rows: BTreeMap<String, String> = text_table::parse(&raw, "DT_TechnologyNameText_Common")
        .expect("parse")
        .into_iter()
        .filter_map(|e| {
            e.key
                .strip_prefix("NAME_RECIPE_")
                .map(|id| (id.to_ascii_lowercase(), e.source))
        })
        .collect();
    println!("{} rows in the technology name table", rows.len());

    // Unlocked technologies across every player in the real save.
    let root_dir = paldex_locate::discover().into_iter().next().expect("save root");
    let world = root_dir
        .worlds()
        .into_iter()
        .find(paldex_locate::World::is_trackable)
        .expect("world");
    let mut unlocked = BTreeSet::new();
    for uid in &world.players {
        let path = world.path.join("Players").join(format!("{}.sav", uid.0));
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok((gvas, _)) = paldex_sav::decompress(&bytes) else { continue };
        let Ok(parsed) = paldex_gvas::parse(&gvas) else { continue };
        if let Ok(progress) = paldex_model::decode_player(&parsed) {
            unlocked.extend(progress.unlocked_tech);
        }
    }
    println!("{} distinct unlocked technologies in save", unlocked.len());

    let mut no_row = Vec::new();
    let mut dangling = Vec::new();
    let mut placeholder = Vec::new();
    for id in &unlocked {
        if index.technology(id).is_some() {
            continue;
        }
        match rows.get(&id.to_ascii_lowercase()) {
            None => no_row.push(id.clone()),
            Some(source) => match text_table::text_reference(source) {
                Some(TextRef::Item(x)) => dangling.push(format!("{id} -> item {x}")),
                Some(TextRef::MapObject(x)) => dangling.push(format!("{id} -> mapobject {x}")),
                None => placeholder.push(format!("{id} = {source:?}")),
            },
        }
    }

    println!("\n--- unresolved, classified ---");
    println!("no row in the technology name table: {}", no_row.len());
    for x in no_row.iter().take(8) {
        println!("    {x}");
    }
    println!("row is a dangling reference:         {}", dangling.len());
    for x in dangling.iter().take(8) {
        println!("    {x}");
    }
    println!("row is an untranslated placeholder:  {}", placeholder.len());
    for x in placeholder.iter().take(8) {
        println!("    {x}");
    }
}
