//! Derived analysis over decoded [`Pal`] records.
//!
//! Save-derived analysis with an injected breeding-result lookup.
//! The app supplies pak-derived results; this crate does not access game files.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::types::{Gender, Ivs, Pal};

/// Unweighted mean of HP, shot, and defense IVs; 0–100 for valid model inputs.
/// Increasing one input while holding the others fixed cannot decrease the score.
#[must_use]
pub fn grade_ivs(ivs: &Ivs) -> f32 {
    (f32::from(ivs.hp) + f32::from(ivs.shot) + f32::from(ivs.defense)) / 3.0
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
    let candidate_score = grade_ivs(&candidate.ivs);
    let current_score = grade_ivs(&current.ivs);
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
            .then(grade_ivs(&b.ivs).total_cmp(&grade_ivs(&a.ivs)))
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
/// A judgment call — the game states no exchange rate between the two, so
/// this weight is ours, not its. Kept (unlike the IV tiers, which were
/// removed) because it does not present itself as a fact: it only orders
/// suggestions, and the UI shows the IV average and the passive pool that
/// produced each one. At 5.0 a full four-passive pool is worth 20
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
    let mut groups = sorted_species_groups(pals);
    for (_, group) in &mut groups {
        truncate_per_gender(group);
    }

    let mut pairs = Vec::new();
    for (i, (id_a, side_a)) in groups.iter().enumerate() {
        // `i..` rather than `i + 1..`: a species can breed with itself, and
        // X + X is the standard way to hold a line.
        for (j, (id_b, side_b)) in groups.iter().enumerate().skip(i) {
            if !child_of(id_a, id_b).is_some_and(|child| child.eq_ignore_ascii_case(target)) {
                continue;
            }
            if let Some(pair) = best_pair(side_a, side_b, i == j) {
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

/// A self-pairing the player owns exactly one of. A Pal cannot breed with
/// itself, so this needs a second specimen rather than a different gender.
fn needs_second_specimen(side: &[&Pal], same_species: bool) -> bool {
    same_species && side.len() < 2
}

/// The gender every owned candidate shares, when that is what stops these two
/// sides pairing, or `None` if a farm can be filled.
///
/// Exact rather than sampled: it reads the genders present across every owned
/// individual. [`Gender::Unknown`] pairs with anything (see [`can_breed`]), so
/// its presence on either side always unblocks the combination.
fn blocking_gender(a: &[&Pal], b: &[&Pal]) -> Option<Gender> {
    let genders: HashSet<Gender> = a.iter().chain(b.iter()).map(|pal| pal.gender).collect();
    match genders.iter().copied().collect::<Vec<_>>()[..] {
        [only] if only != Gender::Unknown => Some(only),
        _ => None,
    }
}

/// A pairing that needs at least one species the player does not own.
///
/// Unlike [`BreedingPair`] this is a *species* combination, not two specific
/// Pals: there is no individual to name for a species nobody owns, so there
/// are no IVs, no gender and no passives to rank on. Whichever side is owned
/// carries its best specimen so the half you already have is still concrete.
#[derive(Debug, Clone, Serialize)]
pub struct UnownedPairing<'a> {
    pub species_a: String,
    pub species_b: String,
    /// The best owned specimen of `species_a`, or `None` if it is unowned.
    ///
    /// For [`PairingNeed::SecondSpecimen`] the one owned Pal sits in `owned_a`
    /// and `owned_b` is `None` — the empty slot is the second Pal that has to
    /// be obtained, not a species the player lacks. For
    /// [`PairingNeed::OppositeGender`] *both* are filled: the player owns
    /// everything, just not in two genders.
    pub owned_a: Option<&'a Pal>,
    pub owned_b: Option<&'a Pal>,
    pub need: PairingNeed,
    /// The gender every owned candidate shares, for
    /// [`PairingNeed::OppositeGender`]; `None` for every other need.
    pub blocking_gender: Option<Gender>,
    /// Species missing from the roster entirely — empty when the player owns
    /// both sides and merely needs another Pal of one of them.
    pub missing_species: Vec<String>,
}

/// What a pairing would cost the player to make possible.
///
/// Every variant is a Pal the player does not currently have, which is what
/// makes them one list: a female Lamball you don't own is as much an errand as
/// a Lamball you don't own. Declaration order is effort order, and the list is
/// sorted by it — another of something already in the box is the smallest ask,
/// two brand-new species the largest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum PairingNeed {
    /// Both slots are one species already owned, but only one specimen is —
    /// and a Pal cannot breed with itself, so a second one is needed.
    SecondSpecimen,
    /// Every owned candidate across both sides is the same gender, so a farm
    /// cannot be filled until an opposite-gender one turns up.
    OppositeGender,
    /// One parent species is missing from the roster.
    OneSpecies,
    /// Neither parent species is owned.
    TwoSpecies,
}

/// Everything for `target` the player cannot breed today, and what each one is
/// waiting on — smallest ask first.
///
/// The counterpart to [`breeding_suggestions`]: between them they cover every
/// combination that produces the target, with no overlap. A combination is
/// either breedable right now or it is here, waiting on a Pal that is not in
/// the box — a species never caught, a second of something owned singly, or
/// one of the opposite gender.
///
/// `species` is the candidate universe for the *unowned* half — inject the
/// Paldeck, since a parent the player cannot recognise is not a usable
/// suggestion. Owned species are always considered whether or not they are in
/// it, so a quest form nobody has a Paldeck entry for cannot fall through.
///
/// Gender is judged over *every* owned individual, not the truncated candidate
/// lists [`breeding_suggestions`] ranks with: telling the player a combination
/// is stuck when a usable specimen merely fell outside the top few would be
/// worse than saying nothing.
#[must_use]
pub fn unowned_pairings<'a, F>(
    pals: &'a [Pal],
    species: &[&str],
    target: &str,
    child_of: F,
) -> Vec<UnownedPairing<'a>>
where
    F: Fn(&str, &str) -> Option<String>,
{
    // Keyed lowercase: `FName`s are case-insensitive and the shipped data is
    // inconsistent (`Sheepball` vs `SheepBall`), so a case mismatch between
    // the roster and the species list would report an owned species as
    // missing — the one error this whole screen must not make.
    let owned = group_by_species(pals);
    let mut owned: HashMap<String, Vec<&Pal>> = owned;
    for group in owned.values_mut() {
        group.sort_by(best_first);
    }

    // Owned species join the universe whether or not the caller listed them,
    // so a form with no Paldeck entry can still be a parent.
    let mut candidates: Vec<&str> = species.to_vec();
    candidates.extend(pals.iter().map(|pal| pal.character_id.as_str()));
    candidates.sort_unstable_by_key(|s| s.to_ascii_lowercase());
    candidates.dedup_by_key(|s| s.to_ascii_lowercase());

    let mut pairings = Vec::new();
    for (i, a) in candidates.iter().enumerate() {
        for b in &candidates[i..] {
            if !child_of(a, b).is_some_and(|child| child.eq_ignore_ascii_case(target)) {
                continue;
            }
            let same_species = b.eq_ignore_ascii_case(a);
            let side_a = owned.get(&a.to_ascii_lowercase());
            let side_b = owned.get(&b.to_ascii_lowercase());

            let (need, blocking, missing_species) = match (side_a, side_b) {
                // Both species owned: the player needs another Pal only if the
                // ones in the box cannot be paired. Otherwise it is breedable
                // today and belongs to `breeding_suggestions`.
                (Some(group_a), Some(group_b)) => {
                    if needs_second_specimen(group_a, same_species) {
                        (PairingNeed::SecondSpecimen, None, Vec::new())
                    } else if let Some(gender) = blocking_gender(group_a, group_b) {
                        (PairingNeed::OppositeGender, Some(gender), Vec::new())
                    } else {
                        continue;
                    }
                }
                (Some(_), None) => (PairingNeed::OneSpecies, None, vec![(*b).to_owned()]),
                (None, Some(_)) => (PairingNeed::OneSpecies, None, vec![(*a).to_owned()]),
                (None, None) if same_species => {
                    (PairingNeed::OneSpecies, None, vec![(*a).to_owned()])
                }
                (None, None) => (
                    PairingNeed::TwoSpecies,
                    None,
                    vec![(*a).to_owned(), (*b).to_owned()],
                ),
            };

            let best = |side: Option<&Vec<&'a Pal>>| side.map(|group| group[0]);
            pairings.push(UnownedPairing {
                species_a: (*a).to_owned(),
                species_b: (*b).to_owned(),
                owned_a: best(side_a),
                // For a second-specimen need the empty slot is B: the one Pal
                // owned always lands in `owned_a`, and B is what to go and get.
                owned_b: if need == PairingNeed::SecondSpecimen { None } else { best(side_b) },
                need,
                blocking_gender: blocking,
                missing_species,
            });
        }
    }

    pairings.sort_by(|x, y| {
        x.need
            .cmp(&y.need)
            .then_with(|| owned_score(y).total_cmp(&owned_score(x)))
            .then_with(|| x.species_a.cmp(&y.species_a))
            .then_with(|| x.species_b.cmp(&y.species_b))
    });
    pairings
}

/// The composite IV score of whichever parent is already owned, or 0 when
/// neither is — used only to order pairings that imply the same amount of work.
fn owned_score(pairing: &UnownedPairing<'_>) -> f32 {
    pairing
        .owned_a
        .into_iter()
        .chain(pairing.owned_b)
        .map(|pal| grade_ivs(&pal.ivs))
        .fold(0.0, f32::max)
}

/// Owned Pals grouped by species, keyed **lowercase**.
///
/// The key has to be case-folded because the save itself is inconsistent: a
/// real roster holds both `GhostAnglerFish` and `GhostAnglerfish`, and both
/// `SheepBall` and `Sheepball`, for what the game treats as one species.
/// Grouping on the exact string splits a species in two, which offers the same
/// combination twice in the suggestions and — worse — can report a pairing as
/// gender-blocked while the mate sits in the other spelling's group.
/// Owned Pals grouped by species, each group sorted best-first, and the groups
/// themselves ordered so a pair search visits combinations deterministically —
/// a suggestion list that reshuffles between identical snapshots looks broken.
///
/// Each entry is `(id, pals)` where `id` is the species as the save spells it,
/// taken from the group's best specimen. Callers pass that to the breeding
/// lookup rather than the folded key, so the closure always sees an id in the
/// form the game's own data uses.
fn sorted_species_groups(pals: &[Pal]) -> Vec<(&str, Vec<&Pal>)> {
    let mut by_species = group_by_species(pals);
    for group in by_species.values_mut() {
        group.sort_by(best_first);
    }

    let mut keys: Vec<String> = by_species.keys().cloned().collect();
    keys.sort_unstable();
    keys.into_iter()
        .map(|key| {
            let group = by_species.remove(&key).expect("key came from this map");
            (group[0].character_id.as_str(), group)
        })
        .collect()
}

fn group_by_species(pals: &[Pal]) -> HashMap<String, Vec<&Pal>> {
    let mut by_species: HashMap<String, Vec<&Pal>> = HashMap::new();
    for pal in pals {
        by_species
            .entry(pal.character_id.to_ascii_lowercase())
            .or_default()
            .push(pal);
    }
    by_species
}

/// The ordering [`is_better`] encodes, as a comparator: composite IV, then
/// level, then instance id so a result never depends on the order the store
/// handed the roster over in.
fn best_first(a: &&Pal, b: &&Pal) -> std::cmp::Ordering {
    grade_ivs(&b.ivs)
        .total_cmp(&grade_ivs(&a.ivs))
        .then_with(|| b.level.cmp(&a.level))
        .then_with(|| b.instance_id.cmp(&a.instance_id))
}

/// Keep the best [`CANDIDATES_PER_SPECIES`] of *each* gender rather than the
/// best that many overall.
///
/// A breeding farm needs one of each, so truncating a male-heavy species to
/// its top few by IV can throw away the only female and make a perfectly
/// breedable combination look impossible — and, now that blocked pairings are
/// reported, actively accuse the player of a gender problem they don't have.
/// Expects `group` already sorted best-first, and leaves it that way.
fn truncate_per_gender(group: &mut Vec<&Pal>) {
    if group.len() <= CANDIDATES_PER_SPECIES {
        return;
    }
    let mut kept: Vec<&Pal> = Vec::new();
    for gender in [Gender::Male, Gender::Female, Gender::Unknown] {
        kept.extend(
            group
                .iter()
                .filter(|pal| pal.gender == gender)
                .take(CANDIDATES_PER_SPECIES)
                .copied(),
        );
    }
    kept.sort_by(best_first);
    *group = kept;
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
    let parent_iv_average = (grade_ivs(&a.ivs) + grade_ivs(&b.ivs)) / 2.0;

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
    fn the_composite_spans_the_same_range_as_the_talents() {
        assert_eq!(grade_ivs(&Ivs { hp: 100, shot: 100, defense: 100 }), 100.0);
        assert_eq!(grade_ivs(&Ivs { hp: 0, shot: 0, defense: 0 }), 0.0);
    }

    #[test]
    fn the_composite_is_the_unweighted_mean() {
        // No talent counts for more than another: the same three values in
        // any arrangement score identically.
        assert_eq!(grade_ivs(&Ivs { hp: 90, shot: 60, defense: 30 }), 60.0);
        assert_eq!(grade_ivs(&Ivs { hp: 30, shot: 90, defense: 60 }), 60.0);
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
                bumped_grade >= base_grade,
                "bumping a talent should never lower the composite score: \
                 {base:?} ({base_grade}) -> {bumped:?} ({bumped_grade})"
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

    /// Suggestions must reference Pals present in the supplied owned roster.
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

    /// The whole Paldeck as far as `fake_table` is concerned.
    const ALL_SPECIES: [&str; 4] = ["Chikipi", "Foxparks", "Lamball", "Vixy"];

    #[test]
    fn an_all_male_combination_needs_an_opposite_gender_pal() {
        let pals = vec![
            gendered("Lamball", ivs(50), 10, Gender::Male),
            gendered("Chikipi", ivs(60), 10, Gender::Male),
        ];

        let pairings = unowned_pairings(&pals, &ALL_SPECIES, "Vixy", fake_table);
        let stuck = pairings
            .iter()
            .find(|p| p.need == PairingNeed::OppositeGender)
            .expect("two males of two species cannot pair");
        assert_eq!(stuck.blocking_gender, Some(Gender::Male));
        assert!(
            stuck.owned_a.is_some() && stuck.owned_b.is_some(),
            "both species are owned; only the gender is missing"
        );
        assert!(
            stuck.missing_species.is_empty(),
            "no species is missing from the roster here"
        );
        assert!(
            breeding_suggestions(&pals, "Vixy", fake_table).is_empty(),
            "the same combination must not also appear as a usable pair"
        );
    }

    /// Owning one of something is a different errand from owning two of one
    /// gender, and the list says which.
    #[test]
    fn owning_only_one_asks_for_a_second_specimen() {
        let pals = vec![gendered("Lamball", ivs(50), 10, Gender::Male)];

        let pairings = unowned_pairings(&pals, &ALL_SPECIES, "Lamball", fake_table);
        let self_pair = pairings
            .iter()
            .find(|p| p.species_a == "Lamball" && p.species_b == "Lamball")
            .expect("Lamball + Lamball should ask for a second Lamball");
        assert_eq!(self_pair.need, PairingNeed::SecondSpecimen);
        assert_eq!(self_pair.blocking_gender, None, "gender is not the problem here");
        assert!(self_pair.owned_a.is_some(), "the one you have is shown");
        assert!(self_pair.owned_b.is_none(), "the slot you must fill is empty");
    }

    /// Owning two of a species means the self-pairing is no longer about
    /// numbers — if it still cannot be bred, it is about gender.
    #[test]
    fn two_of_one_gender_asks_for_the_other_gender() {
        let pals = vec![
            gendered("Lamball", ivs(50), 10, Gender::Male),
            gendered("Lamball", ivs(40), 10, Gender::Male),
        ];

        let pairings = unowned_pairings(&pals, &ALL_SPECIES, "Lamball", fake_table);
        let self_pair = pairings
            .iter()
            .find(|p| p.species_a == "Lamball" && p.species_b == "Lamball")
            .expect("two male Lamballs still cannot pair");
        assert_eq!(self_pair.need, PairingNeed::OppositeGender);
        assert_eq!(self_pair.blocking_gender, Some(Gender::Male));
    }

    /// The needs escalate: another of something owned, then the other gender,
    /// then a new species, then two.
    #[test]
    fn needs_are_ordered_by_how_much_work_they_imply() {
        let pals = vec![gendered("Lamball", ivs(50), 10, Gender::Male)];
        let all = unowned_pairings(&pals, &ALL_SPECIES, "Lamball", fake_table);
        for w in all.windows(2) {
            assert!(w[0].need <= w[1].need, "needs must escalate down the list");
        }
        assert_eq!(all[0].need, PairingNeed::SecondSpecimen);
        assert!(PairingNeed::SecondSpecimen < PairingNeed::OppositeGender);
        assert!(PairingNeed::OppositeGender < PairingNeed::OneSpecies);
        assert!(PairingNeed::OneSpecies < PairingNeed::TwoSpecies);
    }

    #[test]
    fn a_breedable_combination_is_never_listed_as_needing_anything() {
        let pals = vec![
            gendered("Lamball", ivs(50), 10, Gender::Male),
            gendered("Chikipi", ivs(60), 10, Gender::Female),
        ];
        let pairings = unowned_pairings(&pals, &ALL_SPECIES, "Vixy", fake_table);
        assert!(
            !pairings
                .iter()
                .any(|p| p.species_a == "Chikipi" && p.species_b == "Lamball"),
            "this pair can be bred today"
        );
        assert_eq!(breeding_suggestions(&pals, "Vixy", fake_table).len(), 1);
    }

    /// An unknown gender pairs with anything, so it cannot be what a
    /// combination is waiting on.
    #[test]
    fn an_unknown_gender_never_blocks() {
        let pals = vec![
            gendered("Lamball", ivs(50), 10, Gender::Male),
            gendered("Chikipi", ivs(60), 10, Gender::Unknown),
        ];
        assert!(
            !unowned_pairings(&pals, &ALL_SPECIES, "Vixy", fake_table)
                .iter()
                .any(|p| p.need == PairingNeed::OppositeGender)
        );
    }

    /// Regression: candidates used to be truncated to the best few by IV
    /// *before* the gender check, so a species whose top specimens are all one
    /// gender could hide the only mate further down the list — reporting a
    /// perfectly breedable combination as impossible.
    #[test]
    fn a_mate_below_the_candidate_cutoff_is_still_found() {
        let mut pals: Vec<Pal> = (0..CANDIDATES_PER_SPECIES + 4)
            .map(|i| gendered("Lamball", ivs(90 - i as u8), 10, Gender::Male))
            .collect();
        // The only female is the worst Lamball owned, far below the cutoff.
        pals.push(gendered("Lamball", ivs(1), 1, Gender::Female));

        assert!(
            !unowned_pairings(&pals, &ALL_SPECIES, "Lamball", fake_table)
                .iter()
                .any(|p| p.species_a == "Lamball" && p.species_b == "Lamball"),
            "a female exists, so this combination is not waiting on anything"
        );
        let pairs = breeding_suggestions(&pals, "Lamball", fake_table);
        assert_eq!(pairs.len(), 1, "the pair must still be offered");
        let genders = [pairs[0].parent_a.gender, pairs[0].parent_b.gender];
        assert!(
            genders.contains(&Gender::Female) && genders.contains(&Gender::Male),
            "the pair must be one of each, got {genders:?}"
        );
    }

    /// The two lists partition the combinations that produce a target: every
    /// one is breedable now or waiting on a Pal — never both, never neither.
    #[test]
    fn every_combination_lands_in_exactly_one_list() {
        let pals = vec![
            gendered("Lamball", ivs(50), 10, Gender::Male),
            gendered("Chikipi", ivs(60), 10, Gender::Male),
        ];

        for target in ["Vixy", "Lamball", "Chikipi"] {
            let breedable: HashSet<(String, String)> = breeding_suggestions(&pals, target, fake_table)
                .iter()
                .map(|p| species_key(&p.parent_a.character_id, &p.parent_b.character_id))
                .collect();
            let waiting: HashSet<(String, String)> =
                unowned_pairings(&pals, &ALL_SPECIES, target, fake_table)
                    .iter()
                    .map(|p| species_key(&p.species_a, &p.species_b))
                    .collect();

            // Every combination of Paldeck species that gives this target.
            let mut expected: HashSet<(String, String)> = HashSet::new();
            for (i, a) in ALL_SPECIES.iter().enumerate() {
                for b in &ALL_SPECIES[i..] {
                    if fake_table(a, b).is_some_and(|c| c == target) {
                        expected.insert(species_key(a, b));
                    }
                }
            }

            let union: HashSet<_> = breedable.union(&waiting).cloned().collect();
            assert_eq!(union, expected, "{target}: some combination fell through both lists");
            assert!(
                breedable.is_disjoint(&waiting),
                "{target}: a combination is both breedable and waiting"
            );
        }
    }

    fn species_key(a: &str, b: &str) -> (String, String) {
        let mut pair = [a.to_owned(), b.to_owned()];
        pair.sort();
        (pair[0].clone(), pair[1].clone())
    }

    #[test]
    fn unowned_pairings_exclude_combinations_you_can_already_breed() {
        // Both parents owned, so this is the owned tab's business, not ours.
        let pals = vec![pal("Lamball", ivs(50), 10), pal("Chikipi", ivs(50), 10)];

        let pairings = unowned_pairings(&pals, &ALL_SPECIES, "Vixy", fake_table);
        assert!(
            pairings.iter().all(|p| !p.missing_species.is_empty()),
            "every pairing here must need at least one unowned species"
        );
        assert!(
            !pairings
                .iter()
                .any(|p| p.species_a == "Chikipi" && p.species_b == "Lamball"),
            "the fully-owned combination belongs to breeding_suggestions"
        );
        // Foxparks + Lamball also gives Vixy, and Foxparks is unowned.
        let foxparks = pairings
            .iter()
            .find(|p| p.species_a == "Foxparks" || p.species_b == "Foxparks")
            .expect("Foxparks + Lamball should be offered");
        assert_eq!(foxparks.missing_species, ["Foxparks"]);
    }

    #[test]
    fn unowned_pairings_keep_the_best_specimen_of_the_owned_side() {
        let weak = pal("Lamball", ivs(10), 5);
        let strong = pal("Lamball", ivs(95), 5);
        let pals = vec![weak.clone(), strong.clone()];

        let pairings = unowned_pairings(&pals, &ALL_SPECIES, "Vixy", fake_table);
        let pairing = pairings.first().expect("Lamball pairs into Vixy twice over");
        let owned = pairing.owned_a.or(pairing.owned_b).expect("Lamball is owned");
        assert_eq!(owned.instance_id, strong.instance_id);
        assert!(
            pairing.owned_a.is_none() || pairing.owned_b.is_none(),
            "one side must be the unowned species"
        );
    }

    /// Needing one new species is less work than needing two, and the list is
    /// sorted so the actionable rows come first.
    #[test]
    fn unowned_pairings_rank_one_missing_species_above_two() {
        // Nothing owned at all: every pairing needs two species except the
        // self-pairing, which needs one.
        let pairings = unowned_pairings(&[], &ALL_SPECIES, "Vixy", fake_table);
        assert!(!pairings.is_empty());
        for window in pairings.windows(2) {
            assert!(
                window[0].missing_species.len() <= window[1].missing_species.len(),
                "pairings needing fewer new species must come first"
            );
        }
        assert!(
            pairings.iter().all(|p| p.owned_a.is_none() && p.owned_b.is_none()),
            "an empty roster owns nothing"
        );
    }

    #[test]
    fn an_unowned_species_paired_with_itself_needs_only_that_species() {
        let pairings = unowned_pairings(&[], &ALL_SPECIES, "Lamball", fake_table);
        let self_pair = pairings
            .iter()
            .find(|p| p.species_a == "Lamball" && p.species_b == "Lamball")
            .expect("Lamball + Lamball gives Lamball");
        assert_eq!(
            self_pair.missing_species,
            ["Lamball"],
            "you need the one species, not two of it listed twice"
        );
    }

    /// A case mismatch between the roster and the species list must not report
    /// an owned species as missing — the one error this screen cannot make.
    #[test]
    fn ownership_matching_ignores_case() {
        let pals = vec![pal("lamball", ivs(50), 10)];
        let pairings = unowned_pairings(&pals, &ALL_SPECIES, "Vixy", fake_table);
        assert!(
            pairings.iter().all(|p| !p.missing_species.iter().any(|m| m.eq_ignore_ascii_case("Lamball"))),
            "Lamball is owned under a different spelling and must not be listed as missing"
        );
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
