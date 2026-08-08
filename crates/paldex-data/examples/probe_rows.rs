//! Dev-only: dump raw decoded rows of any cooked `DataTable` in the pak.
//!
//! Usage: `probe_rows <pak> <entry-base-path> [row-limit]`
use std::path::Path;

use paldex_data::{datatable, usmap::Usmap, BUNDLED_MAPPINGS};

fn main() {
    let mut args = std::env::args().skip(1);
    let pak_path = args
        .next()
        .expect("usage: probe_rows <pak> <entry> [limit]");
    let base = args.next().expect("entry base path");
    let limit: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);

    let usmap = Usmap::parse(BUNDLED_MAPPINGS).expect("parse usmap");
    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");
    let uasset = pak.read(&format!("{base}.uasset")).expect("read uasset");
    let uexp = pak.read(&format!("{base}.uexp")).expect("read uexp");

    let table = datatable::read(&uasset, &uexp, &usmap).expect("decode");
    println!("{base}");
    println!("  row struct: {}", table.row_struct);
    println!("  rows      : {}", table.rows.len());

    for row in table.rows.iter().take(limit) {
        let mut fields: Vec<String> = row
            .properties
            .iter()
            .map(|(k, v)| format!("{k}={v:?}"))
            .collect();
        fields.sort();
        println!("  {:<24} {}", row.name, fields.join(" "));
    }
}
