//! Dev-only: open the real game pak, print header facts, and list a sample
//! of entry paths — ground truth for the extraction pipeline.
use std::env;
use std::path::Path;
use std::time::Instant;

fn main() {
    let path = env::args().nth(1).expect("usage: inspect_pak <path/to/Pal-Windows.pak>");
    let t0 = Instant::now();
    let mut pak = paldex_data::Pak::open(Path::new(&path)).expect("open pak");
    println!("opened index in {:?}", t0.elapsed());
    println!("version: {:?}", pak.version());
    println!("mount point: {}", pak.mount_point());
    println!("encrypted index: {}", pak.encrypted_index());

    let files = pak.files();
    println!("entries: {}", files.len());
    for f in files.iter().take(10) {
        println!("  {f}");
    }

    if let Some(sample) = files.iter().find(|f| f.to_lowercase().contains("monsterparameter")) {
        println!("\nsample DataTable-ish path: {sample}");
        let t1 = Instant::now();
        let bytes = pak.read(sample).expect("read entry");
        println!("read + decompressed {} bytes in {:?}", bytes.len(), t1.elapsed());
        println!("first 64 bytes: {:02x?}", &bytes[..bytes.len().min(64)]);
    }
}
