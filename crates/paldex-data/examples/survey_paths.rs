//! Dev-only: survey pak entry paths by keyword/extension to find where
//! reference data actually lives (localization, icons, datatables).
use std::collections::BTreeMap;
use std::env;
use std::path::Path;

fn main() {
    let pak_path = env::args().nth(1).expect("usage: survey_paths <pak> [keywords...]");
    let keywords: Vec<String> = env::args().skip(2).collect();
    let pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");
    let files = pak.files();
    println!("entries: {}", files.len());

    let mut by_ext: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &files {
        let ext = f.rsplit_once('.').map_or("<none>", |(_, e)| e);
        *by_ext.entry(ext).or_default() += 1;
    }
    println!("\n--- extensions ---");
    let mut exts: Vec<_> = by_ext.iter().collect();
    exts.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for (e, n) in exts.iter().take(20) {
        println!("  {e:>12}  {n}");
    }

    for kw in &keywords {
        let hits: Vec<_> = files.iter().filter(|f| f.to_lowercase().contains(&kw.to_lowercase())).collect();
        println!("\n--- {} entries matching {kw:?} ---", hits.len());
        let limit: usize = env::var("SURVEY_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(25);
        for h in hits.iter().take(limit) {
            println!("  {h}");
        }
    }
}
