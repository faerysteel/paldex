//! Dev-only: print discovered save roots and game paks on this machine.
fn main() {
    for root in paldex_locate::discover() {
        println!("save root: {} ({})", root.path.display(), root.source);
    }
    for pak in paldex_locate::discover_paks() {
        println!("pak: {}", pak.display());
    }
}
