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

use self::means::{Circumstances, Marks};
use self::means::{ItemUse, Means, Payment};
pub use self::means::{goes_again, reaches, resolvable, usable_item, usable_items};
use super::may::{self, Refusal};
use super::reach::{self, Aim, Falls, Marked};
pub use super::roll::learned as learned_chance;
use super::{Outcome, Strength, cost, power, roll};
use crate::content::spells::{PowerChoice, SpellId, Targeting, Working};
use crate::game::actors::Places;
use crate::game::characters::Practise;
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::hits::definitions::Rows;
use crate::game::hits::effects::{Body, BodyReadOnlyItem, Strike};
use crate::game::hits::reach::{Field, Recipients};
use crate::game::hits::{Deed, Effect};
use crate::game::items::{Items, Spend};
use crate::game::magic::landed::{Leech, Spell, owners_strength};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::hostility::Provocation;
use crate::game::world::identify::{GroupMember, GroupMembers};
use crate::game::world::obstacles::WorldObstacles;

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
    /// A blind caster's spell went astray.
    Astray {
        caster: Entity,
    },
    /// What a deed of magic did, once all of it has landed.
    Landed {
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

/// Whether a blind caster's aim strays: one time in five. A member with
/// the Effective While Blind trait first escapes it at 50 scaled by
/// profession level in a hundred.
fn blind_aim_strays(body: &BodyReadOnlyItem, rng: &mut GameRng) -> bool {
    use crate::content::characters::Trait;
    if !body.conditions.has(crate::game::characters::Condition::Blind) {
        return false;
    }
    let sees_anyway = body.trained().is_some_and(|c| {
        c.traits.contains(Trait::EffectiveWhileBlind)
            && rng.below(100) < c.calling.scale(50.0) as u32
    });
    !sees_anyway && rng.below(100) < 20
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
    /// What a utility spell did; what a deed does is told as it lands
    /// ([`MagicNews::Landed`]).
    pub effects: Vec<Effect>,
}

/// A cast's practice, owed once its deed has landed: its caster practises
/// these skills if the cast affected anyone (retail `SpellAffectedTarget`),
/// or whatever came of it once a missile is on its way.
#[derive(Component, Clone, Debug)]
pub struct Practice {
    pub caster: Entity,
    pub skills: Vec<(crate::content::characters::Skill, u32)>,
    pub sure: bool,
}

/// An auto recast (retail's "repeat" power): once it has landed, its caster
/// casts again while its victims are not whole and stamina and points last.
#[derive(Component, Clone, Debug)]
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
/// as they arrive.
pub fn cast(
    cast: On<Cast>,
    mut bodies: Query<Body>,
    (places, obstacles, held): (Places, WorldObstacles, Items),
    (circumstances, marks): (Circumstances, Marks),
    (memberships, groups, kinds): (
        Query<&'static GroupMember>,
        Query<&'static GroupMembers>,
        Query<&crate::game::world::population::Spellcasting>,
    ),
    fight: Option<Single<(&crate::game::combat::Round, Option<&crate::game::combat::Combatants>)>>,
    roster: Single<&PartyRoster>,
    (content, difficulty): (Res<Catalogs>, Res<Difficulty>),
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
        let Ok(body) = readonly.get(caster) else {
            commands.trigger(Refused { caster, refusal: Refusal::CannotAct });
            continue;
        };
        let moment = circumstances.moment(spell, fighting);
        let asked =
            (means::affordable(&body, spell, &content.spells), means::keeps_alchemy(&body, &kinds));
        let resolved = resolvable(spell, &content);
        if let Err(refusal) = may::may(spell, &body, choice.source, &held, asked, moment, resolved)
        {
            commands.trigger(Refused { caster, refusal });
            continue;
        }
        let Some(means) = Means::of(choice.source, &body, &held) else {
            commands.trigger(Refused { caster, refusal: Refusal::CannotAct });
            continue;
        };
        let item = means.item();
        // A potion or food used in a fight is taken by its user (retail
        // `UseItem` aims potions at their user).
        let forced = fighting && item.is_some_and(ItemUse::by_anyone);
        let targeting = if forced { Targeting::Caster } else { spell.targeting };
        let field = Field {
            bodies: &readonly,
            places: &places,
            obstacles: &obstacles,
            roster,
            combatants,
            memberships: &memberships,
            groups: &groups,
        };
        let aiming = reach::Aiming {
            field: &field,
            caster,
            targeting,
            aim: choice.aim,
            reach: spell.reach.metres(),
        };
        let aimed = match spell.facts.working {
            Some(working) => marks
                .mark(working, &cast, spell, (&readonly, roster), &content)
                .map(|marked| (Vec::new(), None, Some(marked))),
            None => aiming.victims().map(|reached| (reached.victims, reached.spot, None)),
        };
        let (mut victims, mut spot, marked) = match aimed {
            Ok(aimed) => aimed,
            Err(refusal) => {
                commands.trigger(Refused { caster, refusal });
                continue;
            }
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
            &held,
            &content,
        );
        let Some(power) = power else {
            let tired = cost::stamina(&mut rng, &body, means.used(), spell, 0);
            let skill = means.used().and_then(|used| used.skill());
            let mut paying = bodies.get_mut(caster).unwrap();
            paying.vitals.tire(tired);
            if let Some(skill) = skill {
                commands.trigger(Practise { member: caster, skill, usage: 1 });
            }
            commands.trigger(Refused { caster, refusal: Refusal::PlayedBadly });
            continue;
        };
        // A blast takes whoever stands within its radius of where it lands,
        // but those on its caster's side.
        let side = body.stance();
        let radius = reach::blast_radius(spell, power);
        if let (Targeting::Radius, Some(centre)) = (targeting, spot) {
            victims = field.within(centre, centre, radius, Recipients::Opponents(side));
        }
        // A member's spell aimed at the party also reaches the monsters on
        // its side within the spell's radius of the party, widened by the
        // party's own (retail `PopulateSpellTargetMarkers`).
        if targeting == Targeting::Party && body.member && radius > 0.0 {
            let at = places.of(caster);
            let reach = radius + crate::game::party::body::RADIUS;
            let own_side = Recipients::Opponents(side).reversed();
            let allies = field.within(at.centre, at.eye, reach, own_side);
            victims.extend(allies.into_iter().filter(|ally| !roster.contains(ally)));
        }
        // The roll to cast.
        let on_oneself = choice.aim.body() == Some(caster);
        let (chance, quiet) = means.failure(spell, power, pace, on_oneself);
        // The party under water casts no Fire.
        let quenched = spell.realm == Some(crate::content::characters::Realm::Fire)
            && circumstances.underwater();
        let outcome = roll::outcome(&mut rng, chance, quiet, quenched, spell.can_backfire());
        let backfired = outcome == Outcome::Backfired;
        // A blind caster's spell that came off may go astray, unless it is
        // aimed at the caster, of the Mental realm, Rocket Blast or an
        // automatic recast (retail `CheckSpellBackfire` and its caller).
        let guarded = retry
            || on_oneself
            || spell.realm == Some(crate::content::characters::Realm::Mental)
            || id == SpellId::ROCKET_BLAST;
        if outcome == Outcome::Cast
            && !guarded
            && blind_aim_strays(&body, &mut rng)
            && let Some(aim) = aiming.astray(&mut rng)
            && let Ok(reached) = (reach::Aiming { aim, ..aiming }).victims()
        {
            victims = reached.victims;
            spot = reached.spot;
            if let (Targeting::Radius, Some(centre)) = (targeting, spot) {
                victims = field.within(centre, centre, radius, Recipients::Opponents(side));
            }
            news.write(MagicNews::Astray { caster });
        }
        let backfire = aiming.backfire(spot);
        // What a backfire is as strong as, judged before anything changes: as
        // strong as the one it comes from, but a utility spell is as strong as
        // its caster.
        let strength = if backfired && spell.facts.working.is_none() {
            owners_strength(Some(backfire.owner), spell, (power, boost), &readonly)
        } else {
            power::strength(figure, power, boost, spell)
        };
        let stand = places.of(caster);
        let blast_back = (backfired && backfire.falls == Falls::Blast).then(|| {
            let against = Recipients::Opponents(side).reversed();
            field.within(stand.feet, stand.feet, radius, against)
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
            item: item.and_then(|item| held.id(item.item)).map(|id| content.items[id].name.clone()),
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
        let row = content.spell_row(id).expect("a checked spell has a row");
        // Turned on its caster's side it gives nothing back when it follows
        // rows of its own.
        let (rows, leeches) = match backfired {
            true if row.backfires_apart(spell) => (Rows::Backfire(id), None),
            true => (Rows::Backfire(id), row.leeches),
            false => (Rows::Spell(id), row.leeches),
        };
        let flies = spell.missile.is_some() && !backfired && !victims.is_empty();
        news.write(MagicNews::Done(report));
        let flight = |victim: Entity| {
            let to = match (spell.targeting, spot) {
                (Targeting::Radius, Some(at)) => at,
                _ => bodies
                    .as_readonly()
                    .get(victim)
                    .map_or(stand.eye, |body| places.of(body.entity).feet),
            };
            flies.then(|| reach::flight(stand.eye, to))
        };
        let strike = Strike { rows, strength, blow: None, by: Some(caster) };
        // What follows the deed is put beside it before it is let go.
        let arrivals: Vec<_> = victims.iter().map(|&victim| (victim, flight(victim))).collect();
        let mut deed = commands.spawn(Spell { backfired, victims: victims.clone(), ..spell_of });
        if let Some(gives) = leeches {
            deed.insert(Leech { caster: owner, gives });
        }
        if let Some(skills) = practice {
            deed.insert(Practice { caster, skills, sure: flies });
        }
        if repeats {
            deed.insert(Recast { caster, choice, victims, watches_stamina: row.watches_stamina });
        }
        deed.insert(Deed::flying(strike, arrivals));
    }
}
