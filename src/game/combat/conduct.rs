//! What a monster in a fight does when its turn comes, before it picks
//! whom to strike (retail `UpdateMonsterAI`, `MonsterAdvanceChance`).
//!
//! Retail names two of these moves `ADVANCE` and `RETURN_TO_START`; both
//! plan a path away from the party's navigator (`LinkToNavigator` with an
//! explicit target scores nodes by distance from it), with their own
//! notices: one runs for its life, one withdraws.

use crate::content::species::{Reach, Species};
use crate::game::rng::GameRng;
use crate::game::world::hostility::Stance;
use crate::game::world::population::Monster;

/// A monster's choice at its turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Conduct {
    /// It runs for its life, and keeps running in later rounds while the
    /// party stays off.
    Flee,
    /// It moves off from the party.
    Withdraw,
    /// It does nothing this round.
    Hold,
    /// It attacks, or closes in to.
    Fight,
}

/// The chance (%) it runs this turn, and whether a flight already under
/// way goes on (retail `MonsterAdvanceChance`): a badly hurt NPC not
/// hostile always runs; a running one keeps running while the party is
/// beyond short reach; others by their own flee chance, or always when
/// craven.
fn flee_chance(
    monster: &Monster,
    kind: &Species,
    stance: Stance,
    fleeing: bool,
    nearest: f32,
) -> (u32, bool) {
    let health = monster.hp * 100 / monster.max_hp.max(1);
    if kind.npc.is_some() && stance != Stance::Hostile && health <= 33 {
        return (100, fleeing);
    }
    if fleeing {
        return if nearest > Reach::Short.metres() { (100, true) } else { (0, false) };
    }
    match kind.temper.flee_chance {
        0 if kind.temper.craven => (100, false),
        chance => (u32::from(chance), false),
    }
}

/// Decide a monster's conduct this turn. `fleeing` is whether it ran in an
/// earlier round; it is updated. `nearest` is how far the party is.
pub(super) fn decide(
    rng: &mut GameRng,
    monster: &Monster,
    kind: &Species,
    stance: Stance,
    fleeing: &mut bool,
    nearest: f32,
) -> Conduct {
    if monster.species == crate::game::world::hostility::BYSTANDER_SPECIES {
        return Conduct::Hold;
    }
    let (chance, still) = flee_chance(monster, kind, stance, *fleeing, nearest);
    *fleeing = still;
    if rng.below(100) < chance {
        *fleeing = true;
        return Conduct::Flee;
    }
    if stance == Stance::Neutral {
        return if kind.temper.camouflaged { Conduct::Hold } else { Conduct::Withdraw };
    }
    Conduct::Fight
}

/// What a monster's conditions did to its action as it began (retail
/// `GetConditionInterrupt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interrupt {
    /// A web held it.
    Webbed,
    /// Fear froze it.
    Afraid,
    Blinded,
    Nauseated,
    /// Insanity took it, in one of three fits.
    Insane {
        fit: u8,
    },
    /// It broke out of its web.
    Freed,
    /// Insanity turned it on its friends; it attacks them.
    Berserk,
    /// Turncoat holds it; its action against its new foes stands.
    Turncoat,
    /// Fear sent it running.
    Panics,
}

impl Interrupt {
    /// Retail's interrupt number, which numbers its notice.
    pub fn number(self) -> u8 {
        match self {
            Interrupt::Webbed => 0,
            Interrupt::Afraid => 1,
            Interrupt::Blinded => 2,
            Interrupt::Nauseated => 3,
            Interrupt::Insane { fit } => 4 + fit,
            Interrupt::Freed => 7,
            Interrupt::Berserk => 8,
            Interrupt::Turncoat => 9,
            Interrupt::Panics => 12,
        }
    }
}

/// What a monster's conditions do to its action as it begins (retail
/// `GetConditionInterrupt` for a monster): nausea, a web held or broken by
/// Strength, blindness (not a Monk's), insanity (or a berserk turn on its
/// friends), fear (or flight), and Turncoat letting an action against its
/// new foes stand.
pub(super) fn interrupt(
    rng: &mut GameRng,
    monster: &Monster,
    kind: &Species,
    (fleeing, strikes_friend, berserk, against_foes): (bool, bool, bool, bool),
) -> Option<Interrupt> {
    use crate::game::characters::Condition;
    let has = |condition| monster.conditions.iter().any(|&(c, _)| c == condition);
    if has(Condition::Nauseated) && rng.below(100) < 25 {
        return Some(Interrupt::Nauseated);
    }
    if has(Condition::Webbed) {
        let freed = rng.below(100) < u32::from(kind.attributes.strength) / 3;
        return Some(if freed { Interrupt::Freed } else { Interrupt::Webbed });
    }
    if has(Condition::Blind)
        && rng.below(100) < 20
        && kind.kind != crate::content::species::MonsterKind::Monk
    {
        return Some(Interrupt::Blinded);
    }
    if has(Condition::Insane) && rng.below(100) < 80 {
        if strikes_friend && !berserk && rng.below(100) < 40 {
            return Some(Interrupt::Berserk);
        }
        return Some(Interrupt::Insane { fit: rng.below(3) as u8 });
    }
    if has(Condition::Afraid) && rng.below(100) < 25 && !fleeing {
        return Some(Interrupt::Afraid);
    }
    if has(Condition::Afraid) && rng.below(100) < 40 {
        return Some(Interrupt::Panics);
    }
    (has(Condition::Turncoat) && against_foes).then_some(Interrupt::Turncoat)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::content::species::Temper;

    pub(crate) fn monster(hp: u32) -> Monster {
        Monster {
            enchantments: default(),
            location: 0,
            species: 0,
            hp,
            max_hp: 100,
            stamina: 0,
            max_stamina: 0,
            conditions: vec![],
            spell_points: 0,
            body: crate::game::world::population::ScaledBody {
                scale: 1.0,
                radius: 0.5,
                eye_height: 1.5,
                speed: 1.0,
            },
        }
    }

    use bevy::prelude::default;

    /// A web holds a weak monster; nothing interrupts a clear-headed one,
    /// and Turncoat lets an action against its new foes stand.
    #[test]
    fn conditions_interrupt_a_monsters_action() {
        use crate::game::characters::{Condition, Lasting};
        let mut rng = GameRng::seeded(4);
        let kind = Species::default();
        let mut webbed = monster(100);
        webbed.conditions.push((Condition::Webbed, Lasting::Rounds(3)));
        assert_eq!(
            interrupt(&mut rng, &webbed, &kind, (false, false, false, true)),
            Some(Interrupt::Webbed)
        );
        assert_eq!(interrupt(&mut rng, &monster(100), &kind, (false, false, false, true)), None);
        let mut turned = monster(100);
        turned.conditions.push((Condition::Turncoat, Lasting::Rounds(3)));
        assert_eq!(
            interrupt(&mut rng, &turned, &kind, (false, false, false, true)),
            Some(Interrupt::Turncoat)
        );
        assert_eq!(interrupt(&mut rng, &turned, &kind, (false, false, false, false)), None);
    }

    /// A craven species without its own chance always runs, and keeps
    /// running until the party closes to short reach; a steady one fights;
    /// a neutral one withdraws, or holds when camouflaged.
    #[test]
    fn monsters_run_withdraw_hold_or_fight_by_their_temper() {
        let mut rng = GameRng::seeded(7);
        let steady = Species::default();
        let craven = Species { temper: Temper { craven: true, ..default() }, ..default() };
        let mut fleeing = false;
        let hostile = Stance::Hostile;
        assert_eq!(
            decide(&mut rng, &monster(100), &steady, hostile, &mut fleeing, 10.0),
            Conduct::Fight
        );
        assert_eq!(
            decide(&mut rng, &monster(100), &craven, hostile, &mut fleeing, 10.0),
            Conduct::Flee
        );
        assert!(fleeing);
        let brave = Species { temper: Temper { flee_chance: 0, ..default() }, ..default() };
        assert_eq!(
            decide(&mut rng, &monster(100), &brave, hostile, &mut fleeing, 10.0),
            Conduct::Flee,
            "a flight goes on"
        );
        assert_eq!(
            decide(&mut rng, &monster(100), &brave, hostile, &mut fleeing, 3.0),
            Conduct::Fight,
            "cornered"
        );
        assert!(!fleeing);
        let neutral = Stance::Neutral;
        assert_eq!(
            decide(&mut rng, &monster(100), &steady, neutral, &mut fleeing, 10.0),
            Conduct::Withdraw
        );
        let hidden = Species { temper: Temper { camouflaged: true, ..default() }, ..default() };
        assert_eq!(
            decide(&mut rng, &monster(100), &hidden, neutral, &mut fleeing, 10.0),
            Conduct::Hold
        );
        let npc = Species { npc: Some(crate::content::npcs::NpcId(1)), ..default() };
        assert_eq!(
            decide(&mut rng, &monster(30), &npc, neutral, &mut fleeing, 10.0),
            Conduct::Flee
        );
        assert_eq!(decide(&mut rng, &monster(30), &npc, hostile, &mut false, 10.0), Conduct::Fight);
    }
}
