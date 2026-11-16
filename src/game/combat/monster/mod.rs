//! How a monster decides (`docs/rules/monsters.md`). Each decision is a
//! function of the monster's body, of the plain data of its kind (its
//! temper, its special attack, its spells) and of what it makes out
//! ([`sight::Sight`]); one that rolls takes the random-number generator; none
//! changes anything. [`deliberate`], the observer of [`Deliberates`], reads
//! the kind's components and asks the decisions in the order of section 4;
//! the observers of [`super::turn`], [`super::engage`] and
//! [`super::round_end`] make happen what they decide.
//!
//! - [`conduct`]: run, withdraw, hold back, or fight on (4)
//! - [`special`]: a breath or a summoning (4, 7)
//! - [`spell`]: which spell, at whom and how hard (5)
//! - [`guard`]: guarding a fellow, holding back warily (4)
//! - [`close`]: reaching, closing in, withdrawing when it can strike nobody,
//!   and how far a move goes (4, 6)
//! - [`ailments`]: what a monster's conditions do to its action (8)
//! - [`coming`]: groups coming into a fight and leaving it (3)
//! - [`noticing`]: whether a watcher makes out someone watched (2)

pub mod ailments;
pub mod close;
pub mod coming;
pub mod conduct;
pub mod guard;
pub mod noticing;
pub mod sight;
pub mod special;
pub mod spell;

use bevy::prelude::*;

pub use self::ailments::Interrupt;
use self::conduct::Conduct;
use self::guard::Watching;
use self::sight::{Fray, Sight};
use super::turn::{Breathes, Decided, Decision, Fights, Fleeing, Retreats, SpecialCooldown};
use super::{Battle, Combatants};
use crate::content::spells::SpellId;
use crate::game::data::Catalogs;
use crate::game::magic::casting::{MagicChoice, Source};
use crate::game::magic::reach::Aim;
use crate::game::npcs::Embodies;
use crate::game::rng::GameRng;
use crate::game::world::population::{Special, Spellcasting, Temper};

/// A monster with no orders decides what to do with its moment.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Deliberates {
    #[event_target]
    pub monster: Entity,
}

/// A monster decides what to do with its moment, in the order of section 4:
/// its conduct, which takes the moment when it is not to fight; its special
/// attack, a breath aimed as a cone spell is; a spell; withdrawing when it
/// can strike nobody; guarding a fellow or holding back; and otherwise
/// striking, which is closing in when nothing reaches. Whether it runs for
/// its life from now on is marked on it.
pub(super) fn deliberate(
    deliberates: On<Deliberates>,
    sight: Sight,
    kinds: Query<(
        &Temper,
        &Special,
        &Spellcasting,
        Option<&SpecialCooldown>,
        Has<Embodies>,
        Has<Fleeing>,
    )>,
    fight: Single<Option<&Combatants>, With<Battle>>,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    let monster = deliberates.monster;
    let (Ok(own), Ok((temper, special, casting, cooldown, npc, fleeing))) =
        (sight.bodies.get(monster), kinds.get(monster))
    else {
        return;
    };
    let (temper, special) = (&temper.0, &special.0);
    let cooldown = cooldown.map_or(0, |cooldown| cooldown.0);
    let combatants = fight.map_or(&[][..], Combatants::entities);
    let fray = Fray { bodies: combatants, fighting: true, sees_party: sight.sees_party(monster) };
    let gap = sight.party_gap(monster);
    let conducted = conduct::conduct(&mut rng, &own, temper, (npc, fleeing), gap);
    match (fleeing, conducted.fleeing) {
        (false, true) => {
            commands.entity(monster).insert(Fleeing);
        }
        (true, false) => {
            commands.entity(monster).remove::<Fleeing>();
        }
        _ => {}
    }
    let decided = |decision| Decided { actor: monster, decision, own_will: true };
    let retreat = |conduct| Decision::Retreat(Retreats { conduct, quiet: false });
    if conducted.conduct != Conduct::Fight {
        return commands.trigger(decided(retreat(conducted.conduct)));
    }
    let breath = match content.spells.get(SpellId::SPECIAL_ATTACK_CONE) {
        Some(cone) if !special::summons(special) => spell::aims(&sight, &own, cone, fray),
        _ => Vec::new(),
    };
    let strength = crate::game::hits::specials::strength(own.level.0 as u8, own.vitals.stamina);
    let making = (temper, special, cooldown);
    if let Some(made) = special::makes(&mut rng, &own, making, gap, fray.fighting, &breath) {
        let kind = special.kind.unwrap();
        let toward = made.aim.and_then(Aim::body).unwrap_or(monster);
        return commands.trigger(decided(Decision::Special(Breathes {
            special: Some(kind),
            summons: crate::game::hits::specials::special(kind).summons,
            strength,
            toward: sight.places.of(toward).feet,
            cost: made.cost,
            cooldown: special.cooldown,
        })));
    }
    if let Some(cast) = spell::casts(&mut rng, &sight, &own, (temper, casting), gap, &content, fray)
    {
        let source = Source::Innate {
            spell: cast.spell,
            power: cast.power,
            budget: own.budget.map_or(0, |budget| budget.0),
            level: own.level.0 as u8,
        };
        return commands.trigger(decided(Decision::Magic(MagicChoice { source, aim: cast.aim })));
    }
    if close::cannot_strike(&own, gap) {
        return commands.trigger(decided(retreat(Conduct::Withdraw)));
    }
    let fights = |waits| Decision::Fight(Fights { waits, berserk: false });
    commands.trigger(decided(
        match guard::guards(&mut rng, &sight, &own, temper, gap, combatants) {
            Some(Watching::Guards(ward)) => Decision::Protect(ward),
            Some(Watching::HoldsBack) => fights(true),
            None => fights(false),
        },
    ));
}
