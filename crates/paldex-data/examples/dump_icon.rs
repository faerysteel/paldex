//! Dev-only: extract one Pal icon from the pak and write it out as a PNG, so
//! the decoder can be checked by eye against the in-game artwork.
use std::path::Path;

fn main() {
    let pak_path = std::env::args().nth(1).expect("usage: dump_icon <pak> <base> <out.png>");
    let base = std::env::args().nth(2).expect("pak entry base path");
    let out = std::env::args().nth(3).unwrap_or_else(|| "icon.png".to_string());

    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");
    let uexp = pak.read(&format!("{base}.uexp")).expect("read uexp");
    let ubulk = pak.read(&format!("{base}.ubulk")).ok();

    let info = paldex_data::texture::parse(&uexp).expect("parse texture");
    println!("{}x{} {:?} mip0={:?}", info.width, info.height, info.format, info.mip0);

    let data = paldex_data::texture::mip0_bytes(&info, &uexp, ubulk.as_deref()).expect("mip bytes");
    let rgba = paldex_data::texture::decode_rgba(&info, data).expect("decode");
    let png = paldex_data::texture::encode_png(info.width, info.height, &rgba).expect("encode");
    std::fs::write(&out, &png).expect("write png");
    println!("wrote {out} ({} bytes)", png.len());
}
