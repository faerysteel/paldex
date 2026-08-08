//! Dev-only: dump raw pak entries to files for offline probing.
//!
//! Usage: `dump_entry <pak> <out_dir> <entry-path>...`
use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let pak_path = args
        .next()
        .expect("usage: dump_entry <pak> <out_dir> <entry>...");
    let out_dir = args.next().expect("out_dir");
    let entries: Vec<String> = args.collect();

    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");
    std::fs::create_dir_all(&out_dir).expect("mkdir");

    for entry in &entries {
        match pak.read(entry) {
            Ok(bytes) => {
                let name = entry.rsplit('/').next().unwrap_or(entry);
                let out = Path::new(&out_dir).join(name);
                std::fs::write(&out, &bytes).expect("write");
                println!("{} -> {} ({} bytes)", entry, out.display(), bytes.len());
            }
            Err(e) => println!("{entry}: FAILED {e}"),
        }
    }
}
