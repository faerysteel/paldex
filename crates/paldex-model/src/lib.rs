//! Domain types and `RawData` decoders projecting `paldex-gvas`'s generic
//! property tree onto Palworld-specific structures — Pals, players, guilds,
//! bases, and containers.

pub mod rawdata;
pub mod types;

pub use rawdata::character::{decode_character_map, CharacterMapResult};
pub use types::{Gender, Ivs, Pal, PalLocation, SoulUpgrades};
