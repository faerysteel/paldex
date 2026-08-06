//! Dev-only: search every localized text `DataTable` in the pak — all
//! languages — for given substrings, to confirm whether an id has a display
//! name anywhere at all.
use std::env;
use std::path::Path;

fn main() {
    let mut args = env::args().skip(1);
    let pak_path = args.next().expect("usage: search_text_tables <pak> <needle>...");
    let needles: Vec<String> = args.collect();
    assert!(!needles.is_empty(), "provide at least one needle");

    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");
    let tables: Vec<String> = pak
        .files()
        .into_iter()
        .filter(|f| f.contains("DataTable/Text") && f.ends_with(".uexp"))
        .collect();
    println!("searching {} text tables across all languages", tables.len());

    let mut total_hits = 0;
    for needle in &needles {
        let mut hits = Vec::new();
        for table in &tables {
            let Ok(bytes) = pak.read(table) else { continue };
            if bytes
                .windows(needle.len())
                .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
            {
                hits.push(table.clone());
            }
        }
        total_hits += hits.len();
        println!("\n{needle:?}: {} table(s)", hits.len());
        for h in hits.iter().take(20) {
            println!("  {h}");
        }
    }
    println!("\ntotal hits across all needles: {total_hits}");
}
