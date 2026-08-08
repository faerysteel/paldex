//! Dev-only: time reference extraction, to keep app startup honest.
use std::path::Path;
use std::time::Instant;

fn main() {
    let pak_path = std::env::args().nth(1).expect("usage: time_extract <pak>");

    let t = Instant::now();
    let usmap = paldex_data::usmap::Usmap::parse(paldex_data::BUNDLED_MAPPINGS).expect("usmap");
    println!("usmap parse        {:>8.1?}  ({} structs)", t.elapsed(), usmap.struct_count());

    let t = Instant::now();
    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");
    println!("pak open + index   {:>8.1?}", t.elapsed());

    let t = Instant::now();
    let index = paldex_data::ReferenceIndex::extract(&mut pak, "en").expect("extract");
    println!("full extract       {:>8.1?}", t.elapsed());
    println!(
        "  {} species, {} with a dex number, {} warnings",
        index.species_count(),
        index.dex_entry_count(),
        index.warnings().len()
    );
}
