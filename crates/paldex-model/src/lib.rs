//! Domain types and `RawData` decoders projecting `paldex-gvas`'s generic
//! property tree onto Palworld-specific structures — Pals, players, guilds,
//! bases, and containers.

pub mod analysis;
mod gvas_ext;
pub mod rawdata;
pub mod types;

pub use analysis::{
    best_of_species, breeding_suggestions, condense_candidates, grade_ivs, rank_by_passives,
    unowned_pairings, BreedingPair, PairingNeed, UnownedPairing,
};
pub use rawdata::base_camp::{decode_base_camp_map, BaseCamp};
pub use rawdata::character::{decode_character_map, CharacterMapResult};
pub use rawdata::guild::{decode_group_map, GroupKind, Guild};
pub use rawdata::player::decode_player;
pub use types::{
    BossFlags, Collectibles, Gender, Ivs, MiscCounters, Pal, PalLocation, PalLocationKind,
    PlayerIdentity, PlayerProgress, QuestState, SoulUpgrades, CAPTURE_BONUS_AT,
};

/// Resolve each Pal's [`PalLocation::kind`] against a known player's
/// container IDs — `SlotId` alone only carries an opaque container UUID (see
/// the `character` decoder's module docs); this is the join that turns it
/// into `Party`/`Box`.
///
/// Call once per known player and it accumulates correctly: a Pal whose
/// container doesn't match any player checked so far stays `Other` until one
/// does, or until [`resolve_base_locations`] claims it for a base camp.
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

/// Resolve each remaining Pal's [`PalLocation::kind`] to `Base` against the
/// worker containers of every decoded base camp — the other half of the join
/// [`resolve_locations`] starts.
///
/// **Order matters: call this after [`resolve_locations`] has run for every
/// known player.** A Pal already resolved to `Party` or `Box` is left alone,
/// so a player container that somehow also appeared as a worker container
/// would keep the player's answer rather than being silently reassigned. The
/// two sets are disjoint in decoded saves, so this guard should never fire — it is here so
/// that if the assumption ever breaks, it degrades instead of corrupting.
pub fn resolve_base_locations(pals: &mut [Pal], bases: &[BaseCamp]) {
    let worker_containers: std::collections::HashSet<_> =
        bases.iter().filter_map(|b| b.worker_container_id).collect();
    if worker_containers.is_empty() {
        return;
    }

    for pal in pals.iter_mut() {
        let Some(location) = pal.location.as_mut() else {
            continue;
        };
        if location.kind == PalLocationKind::Other
            && worker_containers.contains(&location.container_id)
        {
            location.kind = PalLocationKind::Base;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Gender, Ivs, PalLocation, SoulUpgrades};
    use uuid::Uuid;

    fn pal_in(container_id: Uuid, kind: PalLocationKind) -> Pal {
        Pal {
            instance_id: Uuid::new_v4(),
            character_id: "Lamball".to_owned(),
            owner: None,
            level: 1,
            rank: 1,
            souls: SoulUpgrades::default(),
            ivs: Ivs { hp: 0, shot: 0, defense: 0 },
            passives: Vec::new(),
            equipped_moves: Vec::new(),
            mastered_moves: Vec::new(),
            gender: Gender::Unknown,
            is_lucky: false,
            is_boss: false,
            is_predator: false,
            nickname: None,
            location: Some(PalLocation { container_id, slot_index: 0, kind }),
        }
    }

    fn base_with(worker_container_id: Option<Uuid>) -> BaseCamp {
        BaseCamp {
            id: Uuid::new_v4(),
            guild_id: None,
            worker_container_id,
        }
    }

    #[test]
    fn an_unresolved_pal_in_a_worker_container_becomes_base() {
        let container = Uuid::from_u128(7);
        let mut pals = vec![pal_in(container, PalLocationKind::Other)];
        resolve_base_locations(&mut pals, &[base_with(Some(container))]);
        assert_eq!(pals[0].location.unwrap().kind, PalLocationKind::Base);
    }

    #[test]
    fn a_pal_already_in_a_party_stays_in_the_party() {
        let container = Uuid::from_u128(7);
        let mut pals = vec![pal_in(container, PalLocationKind::Party)];
        resolve_base_locations(&mut pals, &[base_with(Some(container))]);
        assert_eq!(
            pals[0].location.unwrap().kind,
            PalLocationKind::Party,
            "base resolution must never overwrite a container a player already claimed",
        );
    }

    #[test]
    fn a_pal_in_no_known_container_stays_other() {
        let mut pals = vec![pal_in(Uuid::from_u128(7), PalLocationKind::Other)];
        resolve_base_locations(&mut pals, &[base_with(Some(Uuid::from_u128(8)))]);
        assert_eq!(pals[0].location.unwrap().kind, PalLocationKind::Other);
    }

    #[test]
    fn a_base_whose_container_did_not_decode_claims_nothing() {
        let mut pals = vec![pal_in(Uuid::from_u128(7), PalLocationKind::Other)];
        resolve_base_locations(&mut pals, &[base_with(None)]);
        assert_eq!(pals[0].location.unwrap().kind, PalLocationKind::Other);
    }
}
