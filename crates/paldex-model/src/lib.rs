//! Domain types and `RawData` decoders projecting `paldex-gvas`'s generic
//! property tree onto Palworld-specific structures — Pals, players, guilds,
//! bases, and containers.

mod gvas_ext;
pub mod rawdata;
pub mod types;

pub use rawdata::base_camp::{decode_base_camp_map, BaseCamp};
pub use rawdata::character::{decode_character_map, CharacterMapResult};
pub use rawdata::guild::{decode_group_map, GroupKind, Guild};
pub use rawdata::player::decode_player;
pub use types::{
    BossFlags, Collectibles, Gender, Ivs, MiscCounters, Pal, PalLocation, PalLocationKind,
    PlayerProgress, QuestState, SoulUpgrades,
};

/// Resolve each Pal's [`PalLocation::kind`] against a known player's
/// container IDs — `SlotId` alone only carries an opaque container UUID (see
/// the `character` decoder's module docs); this is the join that turns it
/// into `Party`/`Box`.
///
/// Call once per known player and it accumulates correctly: a Pal whose
/// container doesn't match any player checked so far stays `Other` until one
/// does (or forever, if it belongs to a base camp's own storage — base
/// container resolution isn't implemented yet).
pub fn resolve_locations(pals: &mut [Pal], player: &PlayerProgress) {
    for pal in pals.iter_mut() {
        let Some(location) = pal.location.as_mut() else {
            continue;
        };
        if Some(location.container_id) == player.party_container_id {
            location.kind = PalLocationKind::Party;
        } else if Some(location.container_id) == player.box_container_id {
            location.kind = PalLocationKind::Box;
        }
    }
}
