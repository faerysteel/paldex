//! Dev-only: probe a cooked `.uasset`'s package summary and name table.
//!
//! The load-bearing question for Phase 3 is whether the package uses
//! *unversioned* property serialization (which needs a `.usmap` schema) or
//! ordinary tagged properties (which don't). The authoritative signal is the
//! `PKG_UnversionedProperties` bit in `FPackageFileSummary.PackageFlags`, not
//! the file-version fields.
use std::env;
use std::path::Path;

const PKG_UNVERSIONED_PROPERTIES: u32 = 0x0000_2000;
const PACKAGE_FILE_TAG: u32 = 0x9E2A_83C1;

struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn u32(&mut self) -> u32 {
        let v = u32::from_le_bytes(self.b[self.p..self.p + 4].try_into().unwrap());
        self.p += 4;
        v
    }
    fn i32(&mut self) -> i32 {
        self.u32() as i32
    }
    fn skip(&mut self, n: usize) {
        self.p += n;
    }
    fn fstring(&mut self) -> String {
        let len = self.i32();
        if len == 0 {
            return String::new();
        }
        if len > 0 {
            let n = len as usize;
            let s = String::from_utf8_lossy(&self.b[self.p..self.p + n - 1]).into_owned();
            self.p += n;
            s
        } else {
            let n = (-len) as usize;
            let mut u = Vec::with_capacity(n);
            for i in 0..n {
                let o = self.p + i * 2;
                u.push(u16::from_le_bytes([self.b[o], self.b[o + 1]]));
            }
            self.p += n * 2;
            String::from_utf16_lossy(&u[..n.saturating_sub(1)])
        }
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let pak_path = args.next().expect("usage: probe_uasset <pak> [substring]");
    let needle = args.next().unwrap_or_else(|| "DT_PalMonsterParameter".to_string());

    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");
    let files = pak.files();
    println!("pak entries: {}", files.len());

    let usmaps: Vec<_> = files.iter().filter(|f| f.ends_with(".usmap")).collect();
    println!("\n.usmap entries in pak: {}", usmaps.len());
    for u in usmaps.iter().take(10) {
        println!("  {u}");
    }

    let icons: Vec<_> = files.iter().filter(|f| f.contains("T_PalIcon")).collect();
    println!("\nT_PalIcon* entries: {}", icons.len());
    for i in icons.iter().take(5) {
        println!("  {i}");
    }

    let matches: Vec<_> = files.iter().filter(|f| f.contains(&needle)).collect();
    println!("\nentries matching {needle:?}: {}", matches.len());
    for m in matches.iter().take(10) {
        println!("  {m}");
    }

    let Some(target) = matches.iter().find(|f| f.ends_with(".uasset")) else {
        println!("no .uasset match; stopping");
        return;
    };
    let target = (*target).clone();
    println!("\n=== probing {target} ===");
    let bytes = pak.read(&target).expect("read entry");
    println!("size: {} bytes", bytes.len());

    let mut c = Cur { b: &bytes, p: 0 };
    let tag = c.u32();
    println!("tag: {tag:#010x} (expect {PACKAGE_FILE_TAG:#010x}) match={}", tag == PACKAGE_FILE_TAG);
    let legacy = c.i32();
    println!("legacy file version: {legacy}");
    if legacy != -4 {
        println!("legacy UE3 version: {}", c.i32());
    }
    let ue4 = c.i32();
    println!("FileVersionUE4: {ue4}");
    let ue5 = if legacy <= -8 {
        let v = c.i32();
        println!("FileVersionUE5: {v}");
        v
    } else {
        0
    };
    println!("FileVersionLicenseeUE4: {}", c.i32());
    let custom_count = c.i32();
    println!("custom versions: {custom_count}");
    c.skip((custom_count.max(0) as usize) * 20);
    println!("TotalHeaderSize: {}", c.i32());
    println!("FolderName: {:?}", c.fstring());
    let flags = c.u32();
    println!("PackageFlags: {flags:#010x}");
    println!(
        ">>> PKG_UnversionedProperties ({PKG_UNVERSIONED_PROPERTIES:#x}): {}",
        if flags & PKG_UNVERSIONED_PROPERTIES != 0 {
            "SET  -> needs a .usmap"
        } else {
            "NOT SET -> tagged properties, usmap NOT required"
        }
    );
    let name_count = c.i32();
    let name_offset = c.i32();
    println!("NameCount: {name_count}, NameOffset: {name_offset}");
    let _ = ue4;
    let _ = ue5;

    if name_count > 0 && name_offset > 0 && (name_offset as usize) < bytes.len() {
        let mut n = Cur { b: &bytes, p: name_offset as usize };
        println!("\n--- first 60 names ---");
        for i in 0..name_count.min(60) {
            let s = n.fstring();
            n.skip(4); // hashes
            println!("  [{i}] {s}");
        }
        // full scan for interesting field-ish names
        let mut n2 = Cur { b: &bytes, p: name_offset as usize };
        let mut all = Vec::new();
        for _ in 0..name_count {
            let s = n2.fstring();
            n2.skip(4);
            all.push(s);
        }
        println!("\ntotal names: {}", all.len());
        for kw in ["HP", "Element", "Rarity", "ZukanIndex", "Melee", "Volume"] {
            let hits: Vec<_> = all.iter().filter(|s| s.contains(kw)).take(6).collect();
            println!("  names containing {kw:?}: {hits:?}");
        }
    }
}
