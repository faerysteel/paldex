//! Derived-insight layer over decoded [`Pal`]s — the reason to use this over
//! the in-game menus, per the plan.
//!
//! Breeding suggestions (pair → offspring species, ranked by expected
//! passive/IV inheritance) need the breeding-combo reference table from
//! Phase 3, which is still a stub (see `paldex-data::ReferenceData`'s docs) —
//! not implemented here for the same reason. Everything below only needs
//! data already in hand from Phase 2's decoders.

use std::collections::HashMap;

use serde::Serialize;

use crate::types::{Ivs, Pal};

/// A coarse quality tier from a Pal's composite IV score (the mean of its
/// three talents). Thresholds are a documented, arbitrary judgment call —
/// not an in-game mechanic — chosen to roughly match community convention
/// (100 across the board is the universally recognized "perfect" Pal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum IvTier {
    D,
    C,
    B,
    A,
    S,
    Perfect,
}

fn tier_for(composite: f32) -> IvTier {
    if composite >= 100.0 {
        IvTier::Perfect
    } else if composite >= 90.0 {
        IvTier::S
    } else if composite >= 80.0 {
        IvTier::A
    } else if composite >= 70.0 {
        IvTier::B
    } else if composite >= 60.0 {
        IvTier::C
    } else {
        IvTier::D
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct IvGrade {
    pub composite: f32,
    pub tier: IvTier,
}

/// Grade a set of IVs by their composite (mean) score.
///
/// Monotonic by construction: a mean can never decrease when one of its
/// inputs increases and the others hold steady, so a Pal that is at least as
/// good as another in every talent, and strictly better in one, always
/// grades at least as high — see `tests::grading_is_monotonic` for the
/// property test the plan calls for.
#[must_use]
pub fn grade_ivs(ivs: &Ivs) -> IvGrade {
    let composite = (f32::from(ivs.hp) + f32::from(ivs.shot) + f32::from(ivs.defense)) / 3.0;
    IvGrade { composite, tier: tier_for(composite) }
}

/// The single best specimen of each species present in `pals`, by composite
/// IV score (ties broken by higher level, then arbitrarily-but-deterministically
/// by instance id so the result doesn't depend on input order).
#[must_use]
pub fn best_of_species(pals: &[Pal]) -> HashMap<&str, &Pal> {
    let mut best: HashMap<&str, &Pal> = HashMap::new();
    for pal in pals {
        best.entry(pal.character_id.as_str())
            .and_modify(|current| {
                if is_better(pal, current) {
                    *current = pal;
                }
            })
            .or_insert(pal);
    }
    best
}

fn is_better(candidate: &Pal, current: &Pal) -> bool {
    let candidate_score = grade_ivs(&candidate.ivs).composite;
    let current_score = grade_ivs(&current.ivs).composite;
    (candidate_score, candidate.level, candidate.instance_id)
        > (current_score, current.level, current.instance_id)
}

/// Duplicate Pals worth condensing for souls: every owned Pal of a species
/// that has more than one, except the best-of-species specimen (per
/// [`best_of_species`]) — condensing should never consume the one worth
/// keeping. A species with only one owned Pal has no fodder and contributes
/// nothing here.
#[must_use]
pub fn condense_candidates(pals: &[Pal]) -> Vec<&Pal> {
    let best = best_of_species(pals);
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for pal in pals {
        *counts.entry(pal.character_id.as_str()).or_default() += 1;
    }

    pals.iter()
        .filter(|pal| counts.get(pal.character_id.as_str()).copied().unwrap_or(0) > 1)
        .filter(|pal| best.get(pal.character_id.as_str()).map(|b| b.instance_id) != Some(pal.instance_id))
        .collect()
}

/// Owned Pals ranked by passive quality — currently just passive *count* as
/// a placeholder ordering (more passives generally means more value), since
/// real per-passive tiering needs Phase 3's passive-skill reference data
/// (still a stub). Ties broken by composite IV score.
#[must_use]
pub fn rank_by_passives(pals: &[Pal]) -> Vec<&Pal> {
    let mut ranked: Vec<&Pal> = pals.iter().collect();
    ranked.sort_by(|a, b| {
        b.passives
            .len()
            .cmp(&a.passives.len())
            .then(grade_ivs(&b.ivs).composite.total_cmp(&grade_ivs(&a.ivs).composite))
    });
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Gender, SoulUpgrades};
    use uuid::Uuid;

    fn pal(character_id: &str, ivs: Ivs, level: u8) -> Pal {
        Pal {
            instance_id: Uuid::new_v4(),
            character_id: character_id.to_owned(),
            owner: None,
            level,
            rank: 1,
            souls: SoulUpgrades::default(),
            ivs,
            passives: Vec::new(),
            equipped_moves: Vec::new(),
            mastered_moves: Vec::new(),
            gender: Gender::Unknown,
            is_lucky: false,
            is_boss: false,
            is_predator: false,
            nickname: None,
            location: None,
        }
    }

    #[test]
    fn perfect_ivs_grade_as_perfect() {
        let grade = grade_ivs(&Ivs { hp: 100, shot: 100, defense: 100 });
        assert_eq!(grade.tier, IvTier::Perfect);
    }

    #[test]
    fn zero_ivs_grade_as_d() {
        let grade = grade_ivs(&Ivs { hp: 0, shot: 0, defense: 0 });
        assert_eq!(grade.tier, IvTier::D);
    }

    #[test]
    fn grading_is_monotonic() {
        // A small deterministic PRNG (splitmix64) rather than pulling in a
        // proptest dependency for one property test.
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        for _ in 0..10_000 {
            let base = Ivs {
                hp: (next() % 101) as u8,
                shot: (next() % 101) as u8,
                defense: (next() % 101) as u8,
            };
            // Bump one talent up (saturating at 100), hold the others steady.
            let bumped = match next() % 3 {
                0 => Ivs { hp: base.hp.saturating_add(1).min(100), ..base },
                1 => Ivs { shot: base.shot.saturating_add(1).min(100), ..base },
                _ => Ivs { defense: base.defense.saturating_add(1).min(100), ..base },
            };
            let base_grade = grade_ivs(&base);
            let bumped_grade = grade_ivs(&bumped);
            assert!(
                bumped_grade.composite >= base_grade.composite,
                "bumping a talent should never lower the composite score: {base:?} -> {bumped:?}"
            );
            assert!(
                bumped_grade.tier >= base_grade.tier,
                "bumping a talent should never lower the tier: {base:?} ({base_grade:?}) -> {bumped:?} ({bumped_grade:?})"
            );
        }
    }

    #[test]
    fn best_of_species_picks_the_highest_composite_score() {
        let weak = pal("Kitsun", Ivs { hp: 10, shot: 10, defense: 10 }, 5);
        let strong = pal("Kitsun", Ivs { hp: 90, shot: 90, defense: 90 }, 5);
        let other_species = pal("Anubis", Ivs { hp: 100, shot: 100, defense: 100 }, 5);
        let pals = vec![weak.clone(), strong.clone(), other_species.clone()];

        let best = best_of_species(&pals);
        assert_eq!(best.len(), 2);
        assert_eq!(best["Kitsun"].instance_id, strong.instance_id);
        assert_eq!(best["Anubis"].instance_id, other_species.instance_id);
    }

    #[test]
    fn condense_candidates_never_include_the_best_of_species() {
        let weak = pal("Kitsun", Ivs { hp: 10, shot: 10, defense: 10 }, 5);
        let mid = pal("Kitsun", Ivs { hp: 50, shot: 50, defense: 50 }, 5);
        let strong = pal("Kitsun", Ivs { hp: 90, shot: 90, defense: 90 }, 5);
        let lone_other = pal("Anubis", Ivs { hp: 100, shot: 100, defense: 100 }, 5);
        let pals = vec![weak.clone(), mid.clone(), strong.clone(), lone_other.clone()];

        let candidates = condense_candidates(&pals);
        let candidate_ids: Vec<_> = candidates.iter().map(|p| p.instance_id).collect();

        assert!(candidate_ids.contains(&weak.instance_id));
        assert!(candidate_ids.contains(&mid.instance_id));
        assert!(!candidate_ids.contains(&strong.instance_id), "best-of-species must never be a candidate");
        assert!(!candidate_ids.contains(&lone_other.instance_id), "a species with only one owned pal has no fodder");
    }

    #[test]
    fn rank_by_passives_sorts_more_passives_first() {
        let mut few = pal("Kitsun", Ivs { hp: 50, shot: 50, defense: 50 }, 5);
        few.passives = vec!["A".to_owned()];
        let mut many = pal("Anubis", Ivs { hp: 10, shot: 10, defense: 10 }, 5);
        many.passives = vec!["A".to_owned(), "B".to_owned(), "C".to_owned()];

        let candidates = [few.clone(), many.clone()];
        let ranked = rank_by_passives(&candidates);
        assert_eq!(ranked[0].instance_id, many.instance_id);
        assert_eq!(ranked[1].instance_id, few.instance_id);
    }
}
