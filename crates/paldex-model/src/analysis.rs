//! Derived-insight layer over decoded [`Pal`]s — the reason to use this over
//! the in-game menus, per the plan.
//!
//! Everything here works on Phase 2's decoded save data alone, with one
//! exception: [`breeding_suggestions`] needs the pak's breeding table, which
//! lives in `paldex-data`. Rather than depend on that crate — this one is the
//! save decoder and has no business reading the game's install — the caller
//! injects the lookup as a closure. The app layer, where both crates are
//! already in scope, passes `ReferenceIndex::breeding_result`; the tests below
//! pass a hand-built table.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::types::{Gender, Ivs, Pal};

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

/// Owned Pals ranked by passive quality — passive *count*, with ties broken
/// by composite IV score.
///
/// Count is a deliberate placeholder for real per-passive tiering. The pak
/// does now supply passive definitions (`ReferenceIndex::passive`), so the
/// data is no longer the blocker; what is missing is a defensible ranking of
/// one passive against another, which the shipped data does not state and
/// which would otherwise be an invented tier list presented as fact.
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

/// How many individuals of each species are considered as a parent.
///
/// Owning 40 Lamballs would otherwise make the pair search quadratic in the
/// roster for no benefit — the 41st-best Lamball is never the one to breed.
/// Candidates are taken best-first, so the top pair is unaffected.
const CANDIDATES_PER_SPECIES: usize = 8;

/// The most passives a bred Pal can end up with, which caps how much a large
/// combined parent pool is worth.
const MAX_CHILD_PASSIVES: usize = 4;

/// How much one inheritable passive is worth against one point of average
/// parent IV, when ranking pairs.
///
/// A judgment call, like [`tier_for`]'s thresholds — the game states no
/// exchange rate between the two. At 5.0 a full four-passive pool is worth 20
/// IV points, so passives break ties between comparable parents without a
/// weak-but-well-passived pair outranking a near-perfect one.
const PASSIVE_WEIGHT: f32 = 5.0;

/// A suggested breeding pair from the player's own Pals, with the inputs that
/// produced its ranking so the UI can show *why* it was suggested.
#[derive(Debug, Clone, Serialize)]
pub struct BreedingPair<'a> {
    pub parent_a: &'a Pal,
    pub parent_b: &'a Pal,
    /// Mean of the two parents' composite IV scores.
    pub parent_iv_average: f32,
    /// Every distinct passive across both parents — the pool the child draws
    /// its own (at most [`MAX_CHILD_PASSIVES`]) passives from.
    pub inherited_passives: Vec<String>,
    /// [`parent_iv_average`] plus [`PASSIVE_WEIGHT`] per inheritable passive,
    /// counting at most [`MAX_CHILD_PASSIVES`] of them.
    ///
    /// [`parent_iv_average`]: BreedingPair::parent_iv_average
    pub score: f32,
}

/// Owned pairs that breed into `target`, best first.
///
/// `child_of` answers "what do these two species produce" — inject
/// `ReferenceIndex::breeding_result`; see this module's docs for why it is a
/// parameter rather than a dependency. It is called once per distinct species
/// pair, not once per pair of individuals: breeding keys on species (really on
/// tribe, which the lookup owns), so a roster of 2,000 Pals spanning 200
/// species costs ~20,000 lookups rather than two million.
///
/// At most one pair is returned per species combination — the best-scoring
/// individuals available for it. Ten ways to say "Lamball + Chikipi" is not
/// ten suggestions.
///
/// Species ids are compared case-insensitively throughout, because `FName`s
/// are and the shipped data is inconsistent about it (`Sheepball` vs
/// `SheepBall`).
#[must_use]
pub fn breeding_suggestions<'a, F>(pals: &'a [Pal], target: &str, child_of: F) -> Vec<BreedingPair<'a>>
where
    F: Fn(&str, &str) -> Option<String>,
{
    // Best-first within each species, so crossing the truncated lists still
    // finds the best pair for that combination.
    let mut by_species: HashMap<&str, Vec<&Pal>> = HashMap::new();
    for pal in pals {
        by_species.entry(pal.character_id.as_str()).or_default().push(pal);
    }
    for group in by_species.values_mut() {
        // The same ordering `is_better` encodes, as a comparator: composite IV,
        // then level, then instance id so the result never depends on the order
        // the store handed the roster over in.
        group.sort_by(|a, b| {
            grade_ivs(&b.ivs)
                .composite
                .total_cmp(&grade_ivs(&a.ivs).composite)
                .then_with(|| b.level.cmp(&a.level))
                .then_with(|| b.instance_id.cmp(&a.instance_id))
        });
        group.truncate(CANDIDATES_PER_SPECIES);
    }

    // Sorted so the species pairs are visited deterministically; the resulting
    // order only matters for ties, but a suggestion list that reshuffles
    // between identical snapshots looks broken.
    let mut species: Vec<&str> = by_species.keys().copied().collect();
    species.sort_unstable();

    let mut pairs = Vec::new();
    for (i, a) in species.iter().enumerate() {
        // `a..` rather than `a+1..`: a species can breed with itself, and X + X
        // is the standard way to hold a line.
        for b in &species[i..] {
            if !child_of(a, b).is_some_and(|child| child.eq_ignore_ascii_case(target)) {
                continue;
            }
            if let Some(pair) = best_pair(by_species[a].as_slice(), by_species[b].as_slice(), a == b) {
                pairs.push(pair);
            }
        }
    }

    pairs.sort_by(|x, y| {
        y.score
            .total_cmp(&x.score)
            .then_with(|| x.parent_a.instance_id.cmp(&y.parent_a.instance_id))
            .then_with(|| x.parent_b.instance_id.cmp(&y.parent_b.instance_id))
    });
    pairs
}

/// The best-scoring breedable pair drawn from two species' candidates, or
/// `None` when no two of them can actually breed together.
fn best_pair<'a>(a_side: &[&'a Pal], b_side: &[&'a Pal], same_species: bool) -> Option<BreedingPair<'a>> {
    let mut best: Option<BreedingPair<'a>> = None;
    for (i, a) in a_side.iter().enumerate() {
        // Within one species both sides are the same list, so skipping the
        // already-visited prefix avoids offering (x, y) and (y, x) as if they
        // were different pairs.
        let others = if same_species { &b_side[i + 1..] } else { b_side };
        for b in others {
            if !can_breed(a, b) {
                continue;
            }
            let pair = score_pair(a, b);
            if best.as_ref().is_none_or(|current| pair.score > current.score) {
                best = Some(pair);
            }
        }
    }
    best
}

/// Whether two Pals could actually be put in a breeding farm together.
///
/// A farm needs one male and one female, so two Pals of the same known gender
/// are rejected. [`Gender::Unknown`] pairs with anything: it means the save
/// did not state a gender, and silently dropping those Pals would be a worse
/// failure than suggesting a pair the player can see is wrong.
fn can_breed(a: &Pal, b: &Pal) -> bool {
    if a.instance_id == b.instance_id {
        return false;
    }
    !matches!(
        (a.gender, b.gender),
        (Gender::Male, Gender::Male) | (Gender::Female, Gender::Female)
    )
}

fn score_pair<'a>(a: &'a Pal, b: &'a Pal) -> BreedingPair<'a> {
    let parent_iv_average = (grade_ivs(&a.ivs).composite + grade_ivs(&b.ivs).composite) / 2.0;

    // Deduplicated but order-preserving: a passive both parents carry is one
    // passive the child might inherit, not two, and the player reads this list
    // against the two rosters it came from.
    let mut seen = HashSet::new();
    let inherited_passives: Vec<String> = a
        .passives
        .iter()
        .chain(b.passives.iter())
        .filter(|p| seen.insert(p.as_str()))
        .cloned()
        .collect();

    let passive_credit = inherited_passives.len().min(MAX_CHILD_PASSIVES) as f32;
    BreedingPair {
        parent_a: a,
        parent_b: b,
        parent_iv_average,
        inherited_passives,
        score: parent_iv_average + PASSIVE_WEIGHT * passive_credit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SoulUpgrades;
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

    fn gendered(character_id: &str, ivs: Ivs, level: u8, gender: Gender) -> Pal {
        Pal { gender, ..pal(character_id, ivs, level) }
    }

    fn ivs(v: u8) -> Ivs {
        Ivs { hp: v, shot: v, defense: v }
    }

    /// A stand-in for the pak's table, symmetric like the real one. Only the
    /// shape matters here — the real table's correctness is `paldex-data`'s
    /// problem, and is tested there against the game's own data.
    fn fake_table(a: &str, b: &str) -> Option<String> {
        let mut key = [a, b];
        key.sort_unstable();
        match key {
            ["Chikipi", "Lamball"] | ["Foxparks", "Lamball"] => Some("Vixy".to_owned()),
            ["Lamball", "Lamball"] => Some("Lamball".to_owned()),
            _ => None,
        }
    }

    /// The plan's Phase 6 criterion: a suggestion the player cannot act on is
    /// worse than no suggestion.
    #[test]
    fn breeding_suggestions_only_reference_owned_pals() {
        let pals = vec![
            pal("Lamball", ivs(50), 10),
            pal("Chikipi", ivs(50), 10),
            pal("Foxparks", ivs(50), 10),
        ];
        let owned: std::collections::HashSet<Uuid> = pals.iter().map(|p| p.instance_id).collect();

        let suggestions = breeding_suggestions(&pals, "Vixy", fake_table);
        assert!(!suggestions.is_empty(), "Lamball pairs with two owned species that give Vixy");
        for s in &suggestions {
            assert!(owned.contains(&s.parent_a.instance_id), "parent A must be owned");
            assert!(owned.contains(&s.parent_b.instance_id), "parent B must be owned");
        }
    }

    #[test]
    fn breeding_suggestions_rank_better_parents_first() {
        // Both combinations give Vixy, so only parent quality separates them.
        let pals = vec![
            pal("Lamball", ivs(90), 10),
            pal("Chikipi", ivs(90), 10),
            pal("Foxparks", ivs(10), 10),
        ];

        let suggestions = breeding_suggestions(&pals, "Vixy", fake_table);
        assert_eq!(suggestions.len(), 2, "one pair per species combination");
        let species = |p: &BreedingPair<'_>| {
            let mut s = [p.parent_a.character_id.clone(), p.parent_b.character_id.clone()];
            s.sort();
            s
        };
        assert_eq!(species(&suggestions[0]), ["Chikipi", "Lamball"]);
        assert!(suggestions[0].score > suggestions[1].score);
    }

    #[test]
    fn breeding_suggestions_pick_the_best_specimen_of_each_species() {
        let weak = pal("Lamball", ivs(10), 5);
        let strong = pal("Lamball", ivs(95), 5);
        let mate = pal("Chikipi", ivs(50), 5);
        let pals = vec![weak.clone(), strong.clone(), mate.clone()];

        let suggestions = breeding_suggestions(&pals, "Vixy", fake_table);
        assert_eq!(suggestions.len(), 1, "three Pals span one species combination");
        let parents = [
            suggestions[0].parent_a.instance_id,
            suggestions[0].parent_b.instance_id,
        ];
        assert!(parents.contains(&strong.instance_id), "the better Lamball should be offered");
        assert!(!parents.contains(&weak.instance_id), "the worse Lamball should not be");
    }

    #[test]
    fn breeding_suggestions_reject_two_pals_of_the_same_gender() {
        let pals = vec![
            gendered("Lamball", ivs(50), 10, Gender::Male),
            gendered("Chikipi", ivs(50), 10, Gender::Male),
        ];
        assert!(
            breeding_suggestions(&pals, "Vixy", fake_table).is_empty(),
            "a breeding farm needs one male and one female"
        );

        let mixed = vec![
            gendered("Lamball", ivs(50), 10, Gender::Male),
            gendered("Chikipi", ivs(50), 10, Gender::Female),
        ];
        assert_eq!(breeding_suggestions(&mixed, "Vixy", fake_table).len(), 1);
    }

    #[test]
    fn breeding_suggestions_pair_a_species_with_itself_but_never_a_pal_with_itself() {
        let two = vec![pal("Lamball", ivs(50), 10), pal("Lamball", ivs(60), 10)];
        let suggestions = breeding_suggestions(&two, "Lamball", fake_table);
        assert_eq!(suggestions.len(), 1, "X + X is a real pairing");
        assert_ne!(
            suggestions[0].parent_a.instance_id, suggestions[0].parent_b.instance_id,
            "a Pal cannot breed with itself"
        );

        let lone = vec![pal("Lamball", ivs(50), 10)];
        assert!(
            breeding_suggestions(&lone, "Lamball", fake_table).is_empty(),
            "one Lamball is not a breeding pair"
        );
    }

    /// The store's row order is not part of the answer. A suggestion list that
    /// reshuffled between identical snapshots would look like the app changing
    /// its mind.
    #[test]
    fn breeding_suggestions_do_not_depend_on_roster_order() {
        let pals = vec![
            pal("Lamball", ivs(90), 10),
            pal("Lamball", ivs(30), 10),
            pal("Chikipi", ivs(70), 10),
            pal("Foxparks", ivs(70), 10),
        ];
        let mut reversed = pals.clone();
        reversed.reverse();

        let ids = |list: Vec<BreedingPair<'_>>| {
            list.iter()
                .map(|p| (p.parent_a.instance_id, p.parent_b.instance_id))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(breeding_suggestions(&pals, "Vixy", fake_table)),
            ids(breeding_suggestions(&reversed, "Vixy", fake_table))
        );
    }

    #[test]
    fn breeding_suggestions_pool_passives_without_double_counting() {
        let mut a = pal("Lamball", ivs(50), 10);
        a.passives = vec!["Swift".to_owned(), "Legend".to_owned()];
        let mut b = pal("Chikipi", ivs(50), 10);
        b.passives = vec!["Legend".to_owned(), "Artisan".to_owned()];

        let pals = [a, b];
        let suggestions = breeding_suggestions(&pals, "Vixy", fake_table);
        let pair = &suggestions[0];
        // Derived from the pair rather than hardcoded: which parent lands in
        // `parent_a` follows the species sort, which is not what this test is
        // about. Parent A's passives come first, then whatever B adds.
        let mut expected = pair.parent_a.passives.clone();
        for p in &pair.parent_b.passives {
            if !expected.contains(p) {
                expected.push(p.clone());
            }
        }
        assert_eq!(
            pair.inherited_passives, expected,
            "a passive both parents carry is one the child might inherit, not two"
        );
        assert_eq!(pair.inherited_passives.len(), 3, "Legend is shared, so three distinct passives");
        // 50 average IVs plus three inheritable passives at PASSIVE_WEIGHT each.
        assert!((pair.parent_iv_average - 50.0).abs() < f32::EPSILON);
        assert!((pair.score - (50.0 + 3.0 * PASSIVE_WEIGHT)).abs() < f32::EPSILON);
    }

    #[test]
    fn breeding_suggestions_are_empty_for_an_unreachable_target() {
        let pals = vec![pal("Lamball", ivs(50), 10), pal("Chikipi", ivs(50), 10)];
        assert!(breeding_suggestions(&pals, "Anubis", fake_table).is_empty());
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
