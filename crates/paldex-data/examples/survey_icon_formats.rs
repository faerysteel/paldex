//! Dev-only: survey pixel formats and dimensions across every Pal icon
//! texture, so the decoder only has to cover formats that actually occur.
use std::collections::BTreeMap;
use std::path::Path;

fn main() {
    let pak_path = std::env::args().nth(1).expect("usage: survey_icon_formats <pak>");
    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");

    let icons: Vec<String> = pak
        .files()
        .into_iter()
        .filter(|f| f.contains("Texture/PalIcon") && f.ends_with(".uasset"))
        .collect();
    println!("{} icon textures", icons.len());

    let mut formats: BTreeMap<String, usize> = BTreeMap::new();
    let mut dims: BTreeMap<String, usize> = BTreeMap::new();
    let mut has_ubulk = 0usize;
    let mut failures = Vec::new();

    for icon in &icons {
        let base = icon.trim_end_matches(".uasset");
        let Ok(uexp) = pak.read(&format!("{base}.uexp")) else {
            failures.push(format!("{base}: no uexp"));
            continue;
        };
        match paldex_data::texture::parse(&uexp) {
            Ok(t) => {
                *formats.entry(format!("{:?}", t.format)).or_default() += 1;
                *dims.entry(format!("{}x{}", t.width, t.height)).or_default() += 1;
                if t.mip0_in_bulk() {
                    has_ubulk += 1;
                }
            }
            Err(e) => failures.push(format!("{base}: {e}")),
        }
    }

    println!("\nformats: {formats:?}");
    println!("dimensions: {dims:?}");
    println!("mip0 in .ubulk: {has_ubulk}/{}", icons.len());
    println!("failures: {}", failures.len());
    for f in failures.iter().take(10) {
        println!("  {f}");
    }
}
