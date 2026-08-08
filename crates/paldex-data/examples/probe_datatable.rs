//! Dev-only: decode a cooked `DataTable` from the pak using the `.usmap`.
//!
//! Usage: `probe_datatable <pak> <usmap> [entry-base-path]`
use std::path::Path;

use paldex_data::datatable;
use paldex_data::unversioned::Value;
use paldex_data::usmap::Usmap;

fn main() {
    let mut args = std::env::args().skip(1);
    let pak_path = args.next().expect("usage: probe_datatable <pak> <usmap> [entry]");
    let usmap_path = args.next().expect("usmap path");
    let base = args.next().unwrap_or_else(|| {
        "Pal/Content/Pal/DataTable/Character/DT_PalMonsterParameter".to_owned()
    });

    let usmap_bytes = std::fs::read(&usmap_path).expect("read usmap");
    let usmap = Usmap::parse(&usmap_bytes).expect("parse usmap");
    println!(
        "usmap: {} names, {} enums, {} structs",
        usmap.name_count(),
        usmap.enum_count(),
        usmap.struct_count()
    );

    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");
    let uasset = pak.read(&format!("{base}.uasset")).expect("read uasset");
    let uexp = pak.read(&format!("{base}.uexp")).expect("read uexp");

    let table = datatable::read(&uasset, &uexp, &usmap).expect("decode data table");
    println!("\n{base}");
    println!("  row struct : {}", table.row_struct);
    println!("  rows       : {}", table.rows.len());

    let dex: Vec<_> = table
        .rows
        .iter()
        .filter(|r| r.properties.get("ZukanIndex").and_then(Value::as_i32).unwrap_or(0) > 0)
        .collect();
    println!("  ZukanIndex > 0: {}", dex.len());

    for row in table.rows.iter().take(3) {
        dump(&row.name, row);
    }
    if let Some(row) = table.row("Anubis") {
        println!();
        dump("Anubis", row);
    }
}

fn dump(label: &str, row: &datatable::Row) {
    let g = |k: &str| row.properties.get(k);
    println!("\n  {label}:");
    println!(
        "    dex={:?}{} rarity={:?} elements={:?}/{:?}",
        g("ZukanIndex").and_then(Value::as_i32),
        g("ZukanIndexSuffix").and_then(Value::as_str).unwrap_or(""),
        g("Rarity").and_then(Value::as_i32),
        g("ElementType1").and_then(Value::as_str),
        g("ElementType2").and_then(Value::as_str),
    );
    println!(
        "    hp={:?} melee={:?} shot={:?} def={:?} craft={:?}",
        g("Hp").and_then(Value::as_i32),
        g("MeleeAttack").and_then(Value::as_i32),
        g("ShotAttack").and_then(Value::as_i32),
        g("Defense").and_then(Value::as_i32),
        g("CraftSpeed").and_then(Value::as_i32),
    );
    let work: Vec<String> = row
        .properties
        .iter()
        .filter_map(|(k, v)| {
            let suit = k.strip_prefix("WorkSuitability_")?;
            let level = v.as_i32()?;
            (level > 0).then(|| format!("{suit}={level}"))
        })
        .collect();
    println!("    work: {}", work.join(" "));
}
