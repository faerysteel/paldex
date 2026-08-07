//! Dev-only: extract the reference index from the real pak and print a
//! summary, so extraction can be eyeballed against the in-game Paldeck.
use std::env;
use std::path::Path;
use std::time::Instant;

use paldex_data::ReferenceData;

fn main() {
    let pak_path = env::args().nth(1).expect("usage: dump_reference <pak> [lang]");
    let lang = env::args().nth(2).unwrap_or_else(|| "en".to_string());

    let t0 = Instant::now();
    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");
    println!("pak index: {:?}", t0.elapsed());

    let t1 = Instant::now();
    let index = paldex_data::ReferenceIndex::extract(&mut pak, &lang).expect("extract");
    println!("extraction: {:?}", t1.elapsed());

    println!("\nlanguage: {}", index.language());
    println!("species:      {}", index.species_count());
    println!("passives:     {}", index.passive_count());
    println!("technologies: {}", index.technology_count());
    println!("items:        {}", index.item_count());
    println!("map objects:  {}", index.map_object_count());
    println!("icons:        {}", index.icon_count());

    println!("\n--- spot checks ---");
    for id in [
        "AmaterasuWolf",
        "BOSS_AmaterasuWolf",
        "Anubis",
        "BadCatgirl",
        "SheepBall",
        "PinkCat",
    ] {
        match index.species(id) {
            Some(s) => println!("  {id:24} -> {}", s.display_name),
            None => println!("  {id:24} -> (unresolved)"),
        }
    }

    println!("\n--- icon paths ---");
    for id in ["SheepBall", "BOSS_SheepBall", "PinkCat", "AmaterasuWolf"] {
        println!("  {id:20} -> {:?}", index.icon_path(id));
    }

    println!("\n--- NPC classification ---");
    for id in ["Hunter_Rifle", "Viking", "Believer_CrossBow", "AmaterasuWolf"] {
        println!(
            "  {id:24} human_npc={} known={}",
            index.is_human_npc(id),
            index.is_known(id)
        );
    }

    println!("\n--- technology / item samples ---");
    for id in ["COPPER", "PENGUIN_LAUNCHER"] {
        println!("  tech {id:20} -> {:?}", index.technology(id));
    }
    for id in ["Accessory_NormalResist_1", "Wood"] {
        println!("  item {id:20} -> {:?}", index.item(id));
    }

    // `DUMP_SPECIES=1` lists every species as `character_id<TAB>display_name`,
    // for joining the pak's own species set against external data.
    if env::var_os("DUMP_SPECIES").is_some() {
        println!("\n--- all species ---");
        let mut all: Vec<_> = index.species_iter().collect();
        all.sort_by(|a, b| a.character_id.cmp(&b.character_id));
        for s in all {
            println!("SPECIES\t{}\t{}", s.character_id, s.display_name);
        }
    }
}
