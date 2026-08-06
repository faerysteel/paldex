//! Dev-only: probe a cooked `UTexture2D`'s `.uexp` to work out where
//! `FTexturePlatformData` starts and how its mips are stored.
//!
//! The properties are unversioned, but `FTexturePlatformData` writes its pixel
//! format as a plain `FString` (`PF_BC7`, `PF_DXT1`, …), which is findable by
//! byte search. `SizeX`/`SizeY`/`PackedData` sit immediately before it.
use std::env;
use std::path::Path;

fn find_pf(bytes: &[u8]) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut i = 0;
    while i + 8 < bytes.len() {
        let len = i32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        if (4..=32).contains(&len) && i + 4 + len as usize <= bytes.len() {
            let raw = &bytes[i + 4..i + 4 + len as usize];
            if raw.starts_with(b"PF_") && raw.last() == Some(&0) {
                hits.push(i);
            }
        }
        i += 1;
    }
    hits
}

fn main() {
    let pak_path = env::args().nth(1).expect("usage: probe_texture <pak> [base]");
    let base = env::args().nth(2).unwrap_or_else(|| {
        "Pal/Content/Pal/Texture/PalIcon/Normal/T_Anubis_icon_normal".to_string()
    });
    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");

    let uasset = pak.read(&format!("{base}.uasset")).expect("read uasset");
    let summary = paldex_data::uasset::parse_summary(&uasset).expect("summary");
    println!("uasset {} bytes, flags {:#010x}, unversioned={}",
        uasset.len(), summary.package_flags, summary.has_unversioned_properties());
    println!("names: {:?}", &summary.names[..summary.names.len().min(12)]);

    let uexp = pak.read(&format!("{base}.uexp")).expect("read uexp");
    println!("\nuexp {} bytes", uexp.len());

    match pak.read(&format!("{base}.ubulk")) {
        Ok(b) => println!("ubulk {} bytes", b.len()),
        Err(_) => println!("ubulk: (none)"),
    }

    for at in find_pf(&uexp) {
        let len = i32::from_le_bytes(uexp[at..at + 4].try_into().unwrap()) as usize;
        let name = String::from_utf8_lossy(&uexp[at + 4..at + 4 + len - 1]);
        println!("\n--- PixelFormatName {name:?} at {at} ---");
        if at >= 12 {
            let size_x = i32::from_le_bytes(uexp[at - 12..at - 8].try_into().unwrap());
            let size_y = i32::from_le_bytes(uexp[at - 8..at - 4].try_into().unwrap());
            let packed = u32::from_le_bytes(uexp[at - 4..at].try_into().unwrap());
            println!("  preceding 12 bytes -> SizeX={size_x} SizeY={size_y} PackedData={packed:#x}");
            println!("  bytes before that:  {:02x?}", &uexp[at.saturating_sub(28)..at - 12]);
        }
        let after = at + 4 + len;
        let end = (after + 64).min(uexp.len());
        println!("  next {} bytes: {:02x?}", end - after, &uexp[after..end]);
    }
}
