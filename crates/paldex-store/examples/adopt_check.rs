//! Dev-only: open a real database file repeatedly to confirm the migration
//! guard adopts pre-versioning files without losing data.
fn main() {
    let path = std::path::PathBuf::from(std::env::args().nth(1).expect("path"));
    for attempt in 1..=3 {
        match paldex_store::Store::open(&path) {
            Ok(s) => {
                let worlds: i64 = s.conn().query_row("SELECT COUNT(*) FROM worlds", [], |r| r.get(0)).unwrap();
                let pals: i64 = s.conn().query_row("SELECT COUNT(*) FROM pals", [], |r| r.get(0)).unwrap();
                println!("open #{attempt}: ok — {worlds} worlds, {pals} pal rows preserved");
            }
            Err(e) => println!("open #{attempt}: FAILED — {e}"),
        }
    }
}
