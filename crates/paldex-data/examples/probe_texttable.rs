//! Dev-only: probe a cooked text DataTable — dump its `.uasset` name table and
//! hexdump/string-scan the paired `.uexp`, to see how much is recoverable
//! without a `.usmap`.
use std::env;
use std::path::Path;

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

fn names_of(bytes: &[u8]) -> Vec<String> {
    let mut c = Cur { b: bytes, p: 0 };
    c.skip(4); // tag
    let legacy = c.i32();
    if legacy != -4 {
        c.skip(4);
    }
    c.skip(4); // ue4
    if legacy <= -8 {
        c.skip(4); // ue5
    }
    c.skip(4); // licensee
    let custom = c.i32();
    c.skip((custom.max(0) as usize) * 20);
    c.skip(4); // total header size
    c.fstring(); // folder name
    c.skip(4); // package flags
    let count = c.i32();
    let offset = c.i32();
    let mut out = Vec::new();
    if count > 0 && offset > 0 && (offset as usize) < bytes.len() {
        let mut n = Cur { b: bytes, p: offset as usize };
        for _ in 0..count {
            out.push(n.fstring());
            n.skip(4);
        }
    }
    out
}

fn main() {
    let pak_path = env::args().nth(1).expect("usage: probe_texttable <pak> <entry-without-ext>");
    let base = env::args()
        .nth(2)
        .unwrap_or_else(|| "Pal/Content/L10N/en/Pal/DataTable/Text/DT_PalNameText_Common".to_string());
    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");

    let uasset = pak.read(&format!("{base}.uasset")).expect("read uasset");
    let names = names_of(&uasset);
    println!("uasset: {} bytes, {} names", uasset.len(), names.len());
    for (i, n) in names.iter().take(30).enumerate() {
        println!("  [{i}] {n}");
    }

    let uexp = pak.read(&format!("{base}.uexp")).expect("read uexp");
    println!("\nuexp: {} bytes", uexp.len());
    println!("first 128 bytes: {:02x?}", &uexp[..uexp.len().min(128)]);

    // Scan for plausible length-prefixed FStrings.
    println!("\n--- FString scan ---");
    let mut i = 0usize;
    let mut found = 0;
    let limit: usize = std::env::var("SCAN_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
    while i + 4 < uexp.len() && found < limit {
        let len = i32::from_le_bytes(uexp[i..i + 4].try_into().unwrap());
        if (2..=200).contains(&len) && i + 4 + len as usize <= uexp.len() {
            let raw = &uexp[i + 4..i + 4 + len as usize];
            if raw[raw.len() - 1] == 0 && raw[..raw.len() - 1].iter().all(|b| (0x20..0x7f).contains(b)) {
                println!("  @{i}: {:?}", String::from_utf8_lossy(&raw[..raw.len() - 1]));
                found += 1;
                i += 4 + len as usize;
                continue;
            }
        }
        if len < 0 && len > -200 {
            let n = (-len) as usize;
            if i + 4 + n * 2 <= uexp.len() {
                let mut u = Vec::with_capacity(n);
                for k in 0..n {
                    let o = i + 4 + k * 2;
                    u.push(u16::from_le_bytes([uexp[o], uexp[o + 1]]));
                }
                if u.last() == Some(&0) && u[..n - 1].iter().all(|&ch| ch >= 0x20) {
                    println!("  @{i} (utf16): {:?}", String::from_utf16_lossy(&u[..n - 1]));
                    found += 1;
                    i += 4 + n * 2;
                    continue;
                }
            }
        }
        i += 1;
    }
}
