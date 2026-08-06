//! Dev-only helper: decompress a real `.sav` file to a plain `.gvas` file on
//! disk for offline inspection while building the GVAS reader.
use std::env;
use std::fs;

fn main() {
    let mut args = env::args().skip(1);
    let input = args.next().expect("usage: decompress_to_file <in> <out>");
    let output = args.next().expect("usage: decompress_to_file <in> <out>");

    let raw = fs::read(&input).unwrap_or_else(|e| panic!("reading {input}: {e}"));
    let (gvas, compression) =
        paldex_sav::decompress(&raw).unwrap_or_else(|e| panic!("decompressing {input}: {e}"));
    println!("{input}: {compression:?}, {} -> {} bytes", raw.len(), gvas.len());
    fs::write(&output, gvas).unwrap_or_else(|e| panic!("writing {output}: {e}"));
}
