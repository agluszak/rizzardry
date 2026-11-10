//! Casting a learned spell, using an item's spell, or a monster's own spell
//! (`docs/rules/casting.md`). An accepted cast ([`Cast`]) is carried out here
//! as one moment: the observer asks the rules of [`super::may`],
//! [`super::reach`], [`super::power`], [`super::roll`] and [`super::cost`] in
//! the order of the page, of the caster, what the cast comes from
//! ([`means::Means`]) and the spell, and makes happen what they decide: the cast is refused, or provokes, is rolled, paid for and
//! released as a deed that lands ([`crate::game::hits`]), or is told to the
//! utility spell that does its own thing ([`Worked`]).
//!
//! Choosing never spends anything. A refused cast spends nothing; one that is
//! rolled pays even when it fizzles or backfires: spell points for a learned
//! spell, one use for an item, nothing for a monster's own. Its deed keeps its
//! own copy of the spell and victims, so using the last potion still heals, and
//! a missile in flight still lands after its caster falls.

mod means;

use bevy::prelude::*;

pub use self::means::{Around, goes_again, reaches, resolvable, usable_item, usable_items};
use self::means::{ItemUse, Means, Payment, Stage};
use super::may::{self, Refusal};
use super::reach::{self, Aim, Falls, Marked};
pub use super::roll::learned as learned_chance;
use super::{Outcome, Strength, cost, power, roll};
use crate::content::spells::{PowerChoice, SpellId, Targeting, Working};
use crate::game::characters::Practise;
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::definitions::Definitions;
use crate::game::hits::effects::{Body, Strike};
use crate::game::hits::reach::Recipients;
use crate::game::hits::spells::{Leech, Spell, owners_strength};
use crate::game::hits::{Act, Deed, Effect, Release};
use crate::game::items::Spend;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::hostility::Provocation;

/// What pays for a cast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A learned spell at the chosen power, or [`power::AUTO`]; it is
    /// clamped to what the caster can afford when it runs.
    Spell { spell: SpellId, power: u32 },
    /// An item the actor carries or the party's pool holds, identified, not
    /// reserved.
    Item(Entity),
    /// A monster's own casting, which costs no points or uses but tires it:
    /// its spell budget stands in for a caster's skill, its level sets the
    /// strength.
    Innate { spell: SpellId, power: u32, budget: u32, level: u8 },
}

impl Source {
    /// Resolve an item source by its stable stack identity. This is routing,
    /// not acceptance: the cast revalidates eligibility and payment.
    pub fn spell_id(self, held: &crate::game::items::Items) -> Option<SpellId> {
        match self {
            Self::Spell { spell, .. } | Self::Innate { spell, .. } => Some(spell),
            Self::Item(item) => held.casts(item).map(|casts| casts.spell),
        }
    }
}

/// An accepted magic action: the source and the aim, nothing resolved yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MagicChoice {
    pub source: Source,
    pub aim: Aim,
}

/// What became of a magic action, for whoever tells the player.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum MagicNews {
    Done(Report),
    /// An indirect trap source has no character caster or payment.
    Triggered {
        spell: SpellId,
        effects: Vec<Effect>,
    },
    /// A missile reached its victims.
    Landed {
        caster: Entity,
        spell: SpellId,
        effects: Vec<Effect>,
    },
    Refused {
        caster: Entity,
        refusal: Refusal,
    },
}

/// A cast was refused, by whoever checked it: the caster is told
/// ([`MagicNews::Refused`]), and a member who was ordered to cast defends
/// instead.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Refused {
    #[event_target]
    pub caster: Entity,
    pub refusal: Refusal,
}

/// The caster is told their cast was refused.
pub fn refuse(refused: On<Refused>, mut news: MessageWriter<MagicNews>) {
    news.write(MagicNews::Refused { caster: refused.caster, refusal: refused.refusal });
}

/// What a cast did.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub caster: Entity,
    pub spell: SpellId,
    pub power: u32,
    /// The item it came from, already spent.
    pub item: Option<String>,
    pub outcome: Outcome,
    /// What landed at once; missiles land later as [`MagicNews::Landed`].
    pub effects: Vec<Effect>,
}

/// A cast's practice, owed once its deed has landed: its caster practises
/// these skills if the cast affected anyone (retail `SpellAffectedTarget`),
/// or whatever came of it once a missile is on its way.
#[derive(Clone, Debug)]
pub struct Practice {
    pub caster: Entity,
    pub skills: Vec<(crate::content::characters::Skill, u32)>,
    pub sure: bool,
}

/// An auto recast (retail's "repeat" power): once it has landed, its caster
/// casts again while its victims are not whole and stamina and points last.
#[derive(Clone, Debug)]
pub struct Recast {
    pub caster: Entity,
    pub choice: MagicChoice,
    pub victims: Vec<Entity>,
    /// It goes on while a victim is short of stamina rather than of health.
    pub watches_stamina: bool,
}

/// An accepted cast: the spell, and what the caster chose. [`cast`] carries it
/// out or refuses it.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct Cast {
    #[event_target]
    pub caster: Entity,
    pub spell: SpellId,
    pub choice: MagicChoice,
}

/// A utility spell has been paid for and rolled: it now does what it is for
/// (`docs/rules/casting.md` section 8). The observer of its kind answers; the
/// others let it be.
#[derive(EntityEvent, Clone, Debug)]
pub struct Worked {
    #[event_target]
    pub caster: Entity,
    pub working: Working,
    pub report: Report,
    pub strength: Strength,
    pub marked: Marked,
}

/// Carry out an accepted cast (retail `CastSpellFromSource`): ask the rules
/// whether it may be made, whom it reaches, at what power, how likely it fails;
/// refuse it with a word to the player, or provoke whom a member aims at, roll
/// it, pay for it whatever the roll, tire the caster, and release it as a deed,
/// or tell the utility spell it is for. A fizzle leaves only its report, and an
/// auto cast tries again while stamina and points last; a backfire turns on the
/// caster's side; a cast that went off takes its victims, its missiles landing
/// as they arrive. In a fight it counts as an attack.
pub fn cast(
    cast: On<Cast>,
    mut bodies: Query<Body>,
    around: Around,
    mut fight: Option<
        Single<(&mut crate::game::combat::Round, Option<&crate::game::combat::Combatants>)>,
    >,
    roster: Single<&PartyRoster>,
    (content, definitions, difficulty): (Res<Catalogs>, Res<Definitions>, Res<Difficulty>),
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
    mut commands: Commands,
) {
    let roster = *roster;
    // An auto cast that fizzles goes again at once, provoking nobody anew.
    let mut pending = vec![(*cast, false)];
    while let Some((cast, retry)) = pending.pop() {
        let Cast { caster, spell: id, choice } = cast;
        let spell = &content.spells[id];
        let readonly = bodies.as_readonly();
        let combatants =
            fight.as_ref().and_then(|fight| fight.1).map_or(&[][..], |fight| fight.entities());
        let fighting = fight.is_some();
        let pace = roll::Pace {
            difficulty: *difficulty,
            moment: fight.as_ref().map_or(0, |fight| fight.0.moment()),
        };
        let records = (&content.spells, &*definitions);
        let Ok(body) = readonly.get(caster) else {
            commands.trigger(Refused { caster, refusal: Refusal::CannotAct });
            continue;
        };
        let moment = around.moment(spell, fighting);
        let asked = (means::affordable(&body, spell, &content.spells), around.keeps_alchemy(&body));
        let resolved = resolvable(spell, &definitions);
        if let Err(refusal) =
            may::may(spell, &body, choice.source, &around.held, asked, moment, resolved)
        {
            commands.trigger(Refused { caster, refusal });
            continue;
        }
        let Some(means) = Means::of(choice.source, &body, &around.held) else {
            commands.trigger(Refused { caster, refusal: Refusal::CannotAct });
            continue;
        };
        let item = means.item();
        // A potion or food used in a fight is taken by its user (retail
        // `UseItem` aims potions at their user).
        let forced = fighting && item.is_some_and(ItemUse::by_anyone);
        let targeting = if forced { Targeting::Caster } else { spell.targeting };
        let stage =
            Stage::of(caster, spell.reach.metres(), &readonly, &around, (roster, combatants));
        let aiming = stage.aiming(caster, targeting, choice.aim);
        let (mut victims, spot, marked) = match spell.facts.working {
            Some(working) => {
                let marking =
                    around.marking(working, &cast, spell, (&readonly, roster), &definitions);
                match reach::mark(&marking) {
                    Ok(marked) => (Vec::new(), None, Some(marked)),
                    Err(refusal) => {
                        commands.trigger(Refused { caster, refusal });
                        continue;
                    }
                }
            }
            None => match reach::victims(&aiming) {
                Ok(reached) => (reached.victims, reached.spot, None),
                Err(refusal) => {
                    commands.trigger(Refused { caster, refusal });
                    continue;
                }
            },
        };
        let (figure, boost) = means.figure(spell);
        // The power it goes off at. An instrument played too badly to sound
        // tires and trains its player all the same, and that is the end of it.
        let power = means::power_of(
            &mut rng,
            &means,
            &body,
            (&cast, spell),
            (boost, pace),
            &readonly,
            &around,
            records,
        );
        let Some(power) = power else {
            let tired = cost::stamina(&mut rng, &body, means.used(), spell, 0);
            let skill = means.used().and_then(|used| used.skill());
            bodies.get_mut(caster).unwrap().vitals.tire(tired);
            if let Some(skill) = skill {
                commands.trigger(Practise { member: caster, skill, usage: 1 });
            }
            commands.trigger(Refused { caster, refusal: Refusal::PlayedBadly });
            continue;
        };
        let struck = |origin: Vec3| {
            crate::game::hits::spells::struck(
                &readonly,
                &around.places,
                &around.obstacles,
                origin,
                (roster, combatants),
            )
        };
        // A blast takes whoever stands within its radius of where it lands,
        // but those on its caster's side.
        let side = body.stance();
        let radius = reach::blast_radius(spell, power);
        if let (Targeting::Radius, Some(centre)) = (targeting, spot) {
            victims = reach::blasted(centre, radius, Recipients::Opponents(side), &struck(centre));
        }
        // The roll to cast.
        let on_oneself = choice.aim.body() == Some(caster);
        let (chance, quiet) = means.failure(spell, power, pace, on_oneself);
        // The party under water casts no Fire.
        let quenched =
            spell.realm == Some(crate::content::characters::Realm::Fire) && around.underwater();
        let outcome = roll::outcome(&mut rng, chance, quiet, quenched, spell.can_backfire());
        let backfired = outcome == Outcome::Backfired;
        let own_side = reach::own_side(&aiming);
        let backfire = reach::backfire(&aiming, &own_side, spot);
        // What a backfire is as strong as, judged before anything changes: as
        // strong as the one it comes from, but a utility spell is as strong as
        // its caster.
        let strength = if backfired && spell.facts.working.is_none() {
            owners_strength(Some(backfire.owner), spell, (power, boost), &readonly)
        } else {
            power::strength(figure, power, boost, spell)
        };
        let stand = around.places.of(caster);
        let blast_back = (backfired && backfire.falls == Falls::Blast).then(|| {
            let against = Recipients::Opponents(side).reversed();
            reach::blasted(stand.feet, radius, against, &struck(stand.feet))
        });
        // A party member's spell aimed at enemies provokes those it is cast
        // at, whatever the roll (retail `MakeTargetGroupHostile` as the
        // action starts); a retry provokes nobody again.
        if body.member && spell.aimed_at_enemies() && !retry {
            // They are hostile before the cast lands.
            for &victim in &victims {
                commands.trigger(Provocation { monster: victim });
            }
        }
        let report = Report {
            caster,
            spell: id,
            power,
            item: item
                .and_then(|item| around.held.id(item.item))
                .map(|id| content.items[id].name.clone()),
            outcome,
            effects: Vec::new(),
        };
        let auto = choice.source == Source::Spell { spell: id, power: power::AUTO };
        let repeats = auto && spell.power == PowerChoice::ChosenOrRepeat;
        let tired = cost::stamina(&mut rng, &body, means.used(), spell, power);
        let practice = means
            .character()
            .map(|character| cost::practice(character, means.used(), spell, power));
        // Paid whatever the roll: points for a learned spell, a use for an
        // item, nothing for a monster's own; then the caster is tired.
        let payment = means.payment();
        let mut paying = bodies.get_mut(caster).unwrap();
        match payment {
            Payment::Points => {
                let realm = spell.realm.expect("a cast spell has a realm");
                paying.spells.as_mut().unwrap().1.spend(realm, spell.cost * power);
            }
            Payment::Use(item) => commands.trigger(Spend { item, amount: 1 }),
            Payment::Nothing => {}
        }
        paying.vitals.tire(tired);
        // A utility spell does its own thing, whatever the roll; it counts as an
        // act for practice even if it fizzles.
        if let (Some(working), Some(marked)) = (spell.facts.working, marked) {
            for (skill, usage) in practice.into_iter().flatten() {
                commands.trigger(Practise { member: caster, skill, usage });
            }
            commands.trigger(Worked { caster, working, report, strength, marked });
            continue;
        }
        if outcome == Outcome::Fizzled {
            news.write(MagicNews::Done(report));
            // An auto cast that fizzled goes again while stamina and points last.
            let again = bodies
                .as_readonly()
                .get(caster)
                .is_ok_and(|body| goes_again(&body, spell, &content));
            if auto && again {
                pending.push((cast, true));
            }
            continue;
        }
        let (owner, spell_of, victims) = if backfired {
            let victims = match backfire.falls {
                Falls::On(on) => on,
                Falls::Blast => blast_back.unwrap_or_default(),
            };
            let turned = Spell {
                owner: Some(backfire.owner),
                from: backfire.from,
                aim: backfire.aim,
                ..Spell::of(id, caster)
            };
            (backfire.owner, turned, victims)
        } else {
            let aim = match (spell.targeting, spot) {
                (Targeting::Radius | Targeting::Cone, Some(at)) => Aim::Point(at),
                _ => choice.aim,
            };
            let from = stand.eye;
            (caster, Spell { owner: Some(caster), from, aim, ..Spell::of(id, caster) }, victims)
        };
        if let Some(fight) = fight.as_mut() {
            fight.0.attacked(true);
        }
        let uses = if backfired { definitions.backfire(id) } else { definitions.spell(id) };
        let uses = uses.expect("a checked spell is defined");
        let flies = spell.missile.is_some() && !backfired && !victims.is_empty();
        let practice = practice.map(|skills| Practice { caster, skills, sure: flies });
        // A missile cast is reported as it leaves; its impacts when they land.
        let act = if flies {
            news.write(MagicNews::Done(report));
            Act::Missile { caster, spell: id }
        } else {
            Act::Cast(report)
        };
        let flight = |victim: Entity| {
            let to = match (spell.targeting, spot) {
                (Targeting::Radius, Some(at)) => at,
                _ => bodies
                    .as_readonly()
                    .get(victim)
                    .map_or(stand.eye, |body| around.places.of(body.entity).feet),
            };
            flies.then(|| reach::flight(stand.eye, to))
        };
        let row = definitions.get(uses);
        commands.trigger(Release {
            victims: victims.iter().map(|&victim| (victim, flight(victim))).collect(),
            deed: Deed {
                act,
                strike: Strike { definition: uses, strength, blow: None },
                leech: row.leeches.map(|lifesteal| Leech { caster: owner, lifesteal }),
                practice,
                recast: repeats.then(|| Recast {
                    caster,
                    choice,
                    victims: victims.clone(),
                    watches_stamina: definitions
                        .spell(id)
                        .is_some_and(|row| definitions.get(row).watches_stamina),
                }),
                spell: Some(Spell { backfired, victims, ..spell_of }),
            },
        });
    }
}
