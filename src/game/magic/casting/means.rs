//! What a cast is cast with (`docs/rules/casting.md` sections 2 to 4 and 7):
//! a learned spell, an item, or a monster's slot, each a castable. What
//! differs between them is answered here, by [`Means`]: whether it may be
//! cast, its power and how likely it fails, what it tires its caster by and
//! what it trains.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::MagicChoice;
use crate::content::characters::Skill;
use crate::content::items::UseType;
use crate::content::spells::{PowerChoice, SpellDefinition, SpellId, Working};
use crate::game::actors::{CanFight, KeepsAlchemy, Modifiers, Skills, Traits, UsableSkills};
use crate::game::camp::Camp;
use crate::game::characters::{
    Birth, Calling, Condition, Conditions, Profession, Training, Vitals,
};
use crate::game::combat::monster::slot::{Castable, SlotOf};
use crate::game::combat::{Battle, Round};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::items::{Burden, Casts, Items};
use crate::game::magic::castable::{Barred, Boost, FailsAt, Learned, StrengthFigure};
use crate::game::magic::may::{self, Refusal, UseContext};
use crate::game::magic::occasion::{Occupied, SpokenTo, Unsteady};
use crate::game::magic::reach::Aim;
use crate::game::magic::roll::{self, Pace};
use crate::game::magic::{SpellPoints, Strength, cost, power};
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;
use crate::game::world::population::SpellBudget;

/// Whether an auto cast's caster can go again: stamina past the spell's
/// level and points for it at power 1.
pub fn goes_again((stamina, pool): (u32, &SpellPoints), spell: &SpellDefinition) -> bool {
    spell.realm.is_some_and(|realm| {
        power::goes_again(stamina, spell.level, pool.available(realm), spell.cost)
    })
}

/// What a cast goes off at: its power, its strength, how likely it fails,
/// whether a failure passes without notice, how likely a blind aim holds.
#[derive(Clone, Copy, Debug)]
pub struct Powered {
    pub power: u32,
    pub strength: Strength,
    pub failure: u32,
    pub quiet: bool,
    pub sees_blind: u32,
}

/// An instrument or gadget played too badly to sound: it tires and trains
/// its player all the same.
#[derive(Clone, Copy, Debug)]
pub struct PlayedBadly {
    pub stamina: u32,
    pub trains: Option<Skill>,
}

/// What an actor casts with, and how.
#[derive(SystemParam)]
pub struct Means<'w, 's> {
    #[allow(clippy::type_complexity)]
    castables: Query<
        'w,
        's,
        (
            Has<Learned>,
            Has<SlotOf>,
            Has<Castable>,
            Option<&'static Barred>,
            Option<&'static FailsAt>,
            Option<&'static StrengthFigure>,
            Option<&'static Boost>,
            Option<&'static crate::game::magic::castable::SeesBlind>,
        ),
    >,
    #[allow(clippy::type_complexity)]
    casters:
        Query<'w, 's, (Has<CanFight>, Has<PartyMember>, &'static Conditions, Has<KeepsAlchemy>)>,
    #[allow(clippy::type_complexity)]
    users: Query<
        'w,
        's,
        (
            (&'static Calling, &'static Birth),
            &'static crate::game::actors::Attributes,
            &'static Skills,
            &'static Traits,
        ),
    >,
    trained: Query<'w, 's, (&'static Calling, &'static Training, &'static UsableSkills)>,
    burdens: Query<'w, 's, (&'static Burden, &'static Modifiers)>,
    pools: Query<'w, 's, &'static SpellPoints>,
    monsters: Query<'w, 's, (&'static SpellBudget, &'static crate::game::actors::Level)>,
    victims: Query<'w, 's, (&'static Vitals, &'static Conditions, Has<PartyMember>)>,
    held: Items<'w, 's>,
    #[allow(clippy::type_complexity)]
    party: Single<
        'w,
        's,
        (&'static UseContext, Has<Occupied>, Has<SpokenTo>, Has<Unsteady>),
        With<PartyRoster>,
    >,
    camp: Option<Res<'w, Camp>>,
    round: Option<Single<'w, 's, &'static Round, With<Battle>>>,
    difficulty: Res<'w, Difficulty>,
}

impl Means<'_, '_> {
    /// How the round paces a roll now: a moment late in it fails more often.
    fn pace(&self) -> Pace {
        let moment = self.round.as_deref().map_or(0, |round| round.moment());
        Pace { difficulty: *self.difficulty, moment }
    }

    /// What a caster has to go again with: stamina and points.
    pub fn pools_of(&self, caster: Entity) -> Option<(&Vitals, &SpellPoints)> {
        let (vitals, ..) = self.victims.get(caster).ok()?;
        Some((vitals, self.pools.get(caster).ok()?))
    }

    /// Whether a member casts it: a learned spell or an item.
    pub fn by_member(&self, castable: Entity) -> bool {
        !self.castables.get(castable).is_ok_and(|(_, slot, ..)| slot)
    }

    /// Whether it is a learned spell.
    pub fn learned(&self, castable: Entity) -> bool {
        self.castables.get(castable).is_ok_and(|(learned, ..)| learned)
    }

    /// The item it is, if it is one.
    pub fn item(&self, castable: Entity) -> Option<&Casts> {
        self.held.casts(castable)
    }

    /// What may stop it now (section 2, in its order).
    ///
    /// A learned spell: what bars it ([`Barred`]), asked at no power, or
    /// "repeat until whole" in a fight. A monster's slot: one not able to
    /// fight, a silence (its [`Castable`] marker), a spell the game does not
    /// resolve. An item: by a member able to fight; out of the pool only
    /// outside a fight; an instrument not silenced but for Alchemy kept; the
    /// item still there, carried by the caster or pooled; casting a spell at
    /// some power, or played; one its user may set off; with a charge left;
    /// a spell the game resolves; at a moment it suits.
    pub fn refusal(
        &self,
        caster: Entity,
        choice: MagicChoice,
        spell: &SpellDefinition,
    ) -> Option<Refusal> {
        let Ok((learned, slot, castable, barred, ..)) = self.castables.get(choice.castable) else {
            return Some(Refusal::NoItem);
        };
        let (&context, occupied, spoken_to, unsteady) = *self.party;
        if learned {
            let repeats = choice.power == power::AUTO && spell.power == PowerChoice::ChosenOrRepeat;
            return match barred {
                Some(&Barred(refusal)) => Some(refusal),
                None if choice.power == 0 => Some(Refusal::CannotUse),
                None if repeats && context == UseContext::Combat => Some(Refusal::WrongTime),
                None => None,
            };
        }
        let Ok((can_fight, member, conditions, keeps)) = self.casters.get(caster) else {
            return Some(Refusal::CannotAct);
        };
        if slot {
            return if !can_fight {
                Some(Refusal::CannotAct)
            } else if !castable {
                Some(Refusal::Silenced)
            } else if !resolvable(spell) {
                Some(Refusal::Unsupported)
            } else {
                None
            };
        }
        if !can_fight || !member {
            return Some(Refusal::CannotAct);
        }
        let item = choice.castable;
        let held = &self.held;
        let pooled = held.pooled(item);
        let there = (pooled || held.bearer(item) == Some(caster)) && held.id(item).is_some();
        let casts = held.casts(item).filter(|_| there);
        let instrument = casts.is_some_and(|casts| casts.by == UseType::Instrument);
        let worn = held.slot(item).is_some_and(|slot| slot.is_active());
        let user = |casts: &Casts| {
            self.users.get(caster).is_ok_and(|(user, attributes, skills, _)| {
                may::activates(casts, worn, user, attributes, skills)
            })
        };
        let suits = |spell| {
            may::suits(spell, context, occupied, self.camp.is_some(), spoken_to, unsteady, false)
        };
        if pooled && context == UseContext::Combat {
            Some(Refusal::CannotUse)
        } else if conditions.has(Condition::Silenced)
            && (instrument || !there)
            && may::silence_stops(spell, keeps)
        {
            Some(Refusal::Silenced)
        } else if !there {
            Some(Refusal::NoItem)
        } else if let Some(casts) = casts
            && (casts.power != 0 || matches!(casts.by, UseType::Instrument | UseType::Gadget))
            && user(casts)
        {
            if held.charges(item) == Some(0) {
                Some(Refusal::NoCharges)
            } else if !resolvable(spell) {
                Some(Refusal::Unsupported)
            } else if !suits(spell) {
                Some(Refusal::WrongTime)
            } else {
                None
            }
        } else {
            Some(Refusal::CannotUse)
        }
    }

    /// Its power and how it goes off (section 3).
    ///
    /// A learned spell: the power asked for as far as the points reach now
    /// (an auto cast going again has just paid), or, left to the caster,
    /// worked out for the one aimed at; as strong as its figure, boosted by
    /// Power Cast; failing as its castable says, raised late in the round and
    /// by what the spell adds cast on oneself. An item: its own power, but an
    /// instrument or gadget is played, as high as its chance to fail (without
    /// the round's pace, and as if identified) allows; as strong as its
    /// user's level, or their Bard or Gadgeteer levels; failing by the skill
    /// it is used with. A monster's slot: the power it chose, as strong as
    /// its level, failing by its budget alone.
    pub fn power(
        &self,
        caster: Entity,
        choice: MagicChoice,
        spell: &SpellDefinition,
        rng: &mut GameRng,
    ) -> Result<Powered, PlayedBadly> {
        let pace = self.pace();
        let Ok((learned, slot, _, _, fails, figure, boost, sees)) =
            self.castables.get(choice.castable)
        else {
            return Err(PlayedBadly { stamina: 0, trains: None });
        };
        if learned {
            let (fails, figure, boost) = (
                fails.copied().unwrap_or_default(),
                figure.map_or(0, |f| f.0),
                boost.map_or(0, |b| b.0),
            );
            let affordable = self.pools.get(caster).map_or(0, |points| {
                spell
                    .realm
                    .map_or(0, |realm| power::affordable(points.available(realm), spell.cost))
            });
            let power = match choice.power {
                power::AUTO => {
                    let identify = match (spell.facts.working, choice.aim) {
                        (Some(Working::Identify), Aim::Item(target)) => {
                            self.held.obscurity(target).map(|difficulty| (difficulty, boost))
                        }
                        _ => None,
                    };
                    let victim = choice.aim.body().and_then(|aimed| self.victims.get(aimed).ok());
                    let row = crate::game::hits::definitions::row(spell);
                    let failure = std::array::from_fn(|power| roll::paced(fails.0[power], pace));
                    power::auto(
                        spell,
                        (row.and_then(|row| row.restores), row.map_or(&[], |row| &row.outlasts)),
                        victim,
                        affordable,
                        failure,
                        identify,
                    )
                }
                asked => power::chosen(asked, affordable, spell.power),
            };
            let harder = match choice.aim.body() == Some(caster) {
                true => spell.facts.harder_on_oneself,
                false => 0,
            };
            return Ok(Powered {
                power,
                strength: power::strength(figure, power, boost, spell),
                failure: roll::paced(fails.at(power), pace) + harder,
                quiet: false,
                sees_blind: sees.map_or(0, |s| s.0),
            });
        }
        if slot {
            let (budget, level) = self.monsters.get(caster).map_or((0, 0), |(b, l)| (b.0, l.0));
            let power = choice.power;
            return Ok(Powered {
                power,
                strength: power::strength(level as i32, power, 0, spell),
                failure: roll::spell_failure(budget, spell, power),
                quiet: false,
                sees_blind: 0,
            });
        }
        let item = choice.castable;
        let casts = self.held.casts(item).ok_or(PlayedBadly { stamina: 0, trains: None })?;
        let Ok(((calling, _), _, skills, traits)) = self.users.get(caster) else {
            return Err(PlayedBadly { stamina: 0, trains: None });
        };
        let identified = self.held.known(item).is_some_and(|known| known.identified);
        let power = match casts.by {
            UseType::Instrument | UseType::Gadget => {
                let unpaced = Pace { difficulty: Difficulty::Normal, moment: 0 };
                let chances = std::array::from_fn(|power| {
                    roll::item(calling, skills, (casts, true), spell, power as u32 + 1, unpaced).0
                });
                power::played(rng, chances)
            }
            _ => casts.power,
        };
        if power == 0 {
            let stamina = self.toll(caster, choice, spell, 0, rng);
            return Err(PlayedBadly { stamina, trains: casts.by.skill() });
        }
        let figure = match casts.by {
            UseType::Instrument => calling.levels[Profession::Bard],
            UseType::Gadget => calling.levels[Profession::Gadgeteer],
            _ => calling.level(),
        };
        let (failure, quiet) = roll::item(calling, skills, (casts, identified), spell, power, pace);
        let sees_blind =
            match traits.0.contains(crate::content::characters::Trait::EffectiveWhileBlind) {
                true => calling.scale(50.0) as u32,
                false => 0,
            };
        Ok(Powered {
            power,
            strength: power::strength(i32::from(figure), power, 0, spell),
            failure,
            quiet,
            sees_blind,
        })
    }

    /// What a cast tires its caster by (section 7): a member's toll scaled by
    /// what they carry, a quarter less slowed, otherwise a quarter more
    /// hasted; a monster's unscaled.
    pub fn toll(
        &self,
        caster: Entity,
        choice: MagicChoice,
        spell: &SpellDefinition,
        power: u32,
        rng: &mut GameRng,
    ) -> u32 {
        let used = self.held.casts(choice.castable).map(|casts| casts.by);
        let diseased = self.casters.get(caster).is_ok_and(|(.., c, _)| c.has(Condition::Diseased));
        let toll = cost::stamina(rng, used, spell, power, diseased);
        match (self.burdens.get(caster), self.casters.get(caster)) {
            (Ok((burden, modifiers)), Ok((.., conditions, _)))
                if self.by_member(choice.castable) =>
            {
                let slowed = conditions.has(Condition::Slowed);
                burden.load.scale_fatigue(toll, slowed, modifiers.laid.stride > 0.0)
            }
            _ => toll,
        }
    }

    /// What a member's cast trains (section 7): a learned spell its book,
    /// realm and Power Cast; an item the skill it is used with. A monster
    /// trains nothing.
    pub fn practice(
        &self,
        caster: Entity,
        choice: MagicChoice,
        spell: &SpellDefinition,
        power: u32,
    ) -> Vec<(Skill, u32)> {
        if !self.by_member(choice.castable) {
            return Vec::new();
        }
        let used = self.held.casts(choice.castable).map(|casts| casts.by);
        self.trained.get(caster).map_or_else(
            |_| Vec::new(),
            |(calling, training, usable)| {
                let trained = (calling, training, usable.0.contains(Skill::PowerCast));
                cost::practice(trained, used, spell, power)
            },
        )
    }
}

/// Whether the game resolves a spell it is asked about: it has a row, or is a
/// utility spell (what menus offer).
pub fn resolvable(spell: &SpellDefinition) -> bool {
    crate::game::hits::definitions::row(spell).is_some() || spell.facts.working.is_some()
}

/// The spells and items an actor could use: what menus offer. Execution
/// validates again.
pub fn usable_items(
    member: Entity,
    (user, attributes, skills): (
        (&Calling, &Birth),
        &crate::game::actors::Attributes,
        &crate::game::actors::Skills,
    ),
    held: &Items,
    content: &Catalogs,
) -> Vec<(Entity, SpellId)> {
    held.carried(member)
        .filter_map(|item| {
            let worn = held.slot(item).is_some_and(|slot| slot.is_active());
            Some((item, usable_item((user, attributes, skills), item, held, content, worn)?))
        })
        .collect()
}

/// The spell an item casts for `character`, if they can use it now: one
/// they may activate (worn, if it must be), with a charge left, whose
/// effect the game resolves.
pub fn usable_item(
    (user, attributes, skills): (
        (&Calling, &Birth),
        &crate::game::actors::Attributes,
        &crate::game::actors::Skills,
    ),
    item: Entity,
    held: &Items,
    content: &Catalogs,
    worn: bool,
) -> Option<SpellId> {
    let cast = held.casts(item)?;
    (may::activates(cast, worn, user, attributes, skills)
        && held.charges(item) != Some(0)
        && resolvable(content.spells.get(cast.spell)?))
    .then_some(cast.spell)
}
