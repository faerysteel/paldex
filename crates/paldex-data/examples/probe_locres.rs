//! Dev-only: parse a `.locres` from the pak and dump namespaces/keys.
//!
//! `.locres` is a standalone Unreal localization container — plain FStrings,
//! no property schema — so it is readable without a `.usmap`.
use std::env;
use std::path::Path;

const MAGIC: [u8; 16] = [
    0x0E, 0x14, 0x74, 0x75, 0x67, 0x4A, 0x03, 0xFC, 0x4A, 0x15, 0x90, 0x9D, 0xC3, 0x37, 0x7F, 0x1B,
];

struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

impl Cur<'_> {
    fn u32(&mut self) -> u32 {
        let v = u32::from_le_bytes(self.b[self.p..self.p + 4].try_into().unwrap());
        self.p += 4;
        v
    }
    fn i32(&mut self) -> i32 {
        self.u32() as i32
    }
    fn i64(&mut self) -> i64 {
        let v = i64::from_le_bytes(self.b[self.p..self.p + 8].try_into().unwrap());
        self.p += 8;
        v
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
    let pak_path = env::args().nth(1).expect("usage: probe_locres <pak> [entry]");
    let entry = env::args()
        .nth(2)
        .unwrap_or_else(|| "Pal/Content/Localization/Game/en/Game.locres".to_string());
    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");
    let bytes = pak.read(&entry).expect("read locres");
    println!("{entry}: {} bytes", bytes.len());

    let mut c = Cur { b: &bytes, p: 0 };
    let magic = &bytes[..16];
    println!("magic match: {}", magic == MAGIC);
    c.p = 16;
    let version = bytes[c.p];
    c.p += 1;
    println!("version: {version}");
    let string_array_offset = c.i64();
    println!("string array offset: {string_array_offset}");
    if version >= 3 {
        println!("localized string count: {}", c.u32());
    }

    // string array first
    let mut sa = Cur { b: &bytes, p: string_array_offset as usize };
    let sa_count = sa.i32();
    println!("string array count: {sa_count}");
    let mut strings = Vec::with_capacity(sa_count.max(0) as usize);
    for _ in 0..sa_count {
        let s = sa.fstring();
        if version >= 3 {
            sa.u32(); // refcount
        }
        strings.push(s);
    }

    let ns_count = c.u32();
    println!("namespaces: {ns_count}");
    let mut total = 0usize;
    let mut pal_names: Vec<(String, String)> = Vec::new();
    for _ in 0..ns_count {
        c.u32(); // ns hash
        let ns = c.fstring();
        let key_count = c.u32();
        for _ in 0..key_count {
            c.u32(); // key hash
            let key = c.fstring();
            c.u32(); // source hash
            let idx = c.i32();
            total += 1;
            let val = strings.get(idx as usize).cloned().unwrap_or_default();
            if key.contains("PAL_NAME") || ns.contains("PAL_NAME") {
                pal_names.push((key, val));
            }
        }
        if ns_count < 20 {
            println!("  namespace {ns:?} ({key_count} keys)");
        }
    }
    println!("total entries: {total}");
    println!("\nPAL_NAME-ish entries: {}", pal_names.len());
    for (k, v) in pal_names.iter().take(25) {
        println!("  {k} = {v}");
    }
}
