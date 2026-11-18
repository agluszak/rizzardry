//! A spell that ends in a deed is cast in one moment, in the order of
//! `docs/rules/casting.md` sections 2 to 7: whether it may be cast, whom it
//! is aimed at, its power and the area that spreads it over, the provocation,
//! the roll, a blind caster's aim gone astray or a backfire turned on the
//! caster's side, the payment, and its release as a deed of magic that lands
//! ([`crate::game::hits`]). How its kind of target is aimed is a row of
//! `assets/tables/targeting.ron`; what it is cast with answers through
//! [`Means`]; whom an area takes in, through [`Reaching`].

use bevy::prelude::*;

use super::means::{Means, PlayedBadly};
use super::{Cast, MagicNews, Pays, Practice, Recast, Refused, Report};
use crate::content::characters::Realm;
use crate::content::items::UseType;
use crate::content::spells::{Anchor, ComesFrom, PowerChoice, SpellId, Strays, Targeting};
use crate::game::actors::CasterLevels;
use crate::game::characters::{Condition, Conditions, Practise, Vitals};
use crate::game::data::Catalogs;
use crate::game::hits::Deed;
use crate::game::hits::definitions::Rows;
use crate::game::hits::effects::Strike;
use crate::game::items::{Items, Spend};
use crate::game::magic::aim::{About, Against, Area, Extent, Reaching};
use crate::game::magic::castable::{CastableSpell, SeesBlind};
use crate::game::magic::landed::{Bursts, Leech, LetGoAt, TurnsBack};
use crate::game::magic::may::{Refusal, UseContext};
use crate::game::magic::reach::{Aim, blast_radius, flight};
use crate::game::magic::{Outcome, power, roll};
use crate::game::party::roster::PartyMember;
use crate::game::rng::GameRng;
use crate::game::world::hostility::{Provocation, Standing};
use crate::game::world::level::InPlay;
use crate::game::world::triggers::LevelState;

/// A spell is cast, and what comes of it happens: it is refused and nothing
/// is spent; or it is aimed, its power settled, the one it is against
/// provoked, rolled, paid for, and, unless it fizzled, let go as a deed.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn cast(
    cast: On<Cast>,
    spells: Query<&CastableSpell>,
    means: Means,
    reaching: Reaching,
    (standings, levels): (Query<&Standing>, Query<&CasterLevels>),
    casters: Query<(&Conditions, Has<PartyMember>, Option<&SeesBlind>)>,
    underwater: Option<Single<&LevelState, With<InPlay>>>,
    context: Single<&UseContext, With<crate::game::party::roster::PartyRoster>>,
    held: Items,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<MagicNews>,
    mut commands: Commands,
) {
    let Cast { caster, choice, again } = *cast;
    let Ok(CastableSpell(spell)) = spells.get(choice.castable) else {
        commands.trigger(Refused { caster, refusal: Refusal::NoItem });
        return;
    };
    if spell.facts.working.is_some() {
        return;
    }
    // An auto cast that fizzled goes again only while stamina and points last.
    let lasts = |(vitals, pool): (&Vitals, &crate::game::magic::SpellPoints)| {
        super::goes_again((vitals.stamina.current, pool), spell)
    };
    if again && !means.pools_of(caster).is_some_and(lasts) {
        return;
    }
    let refuse = |commands: &mut Commands, refusal| commands.trigger(Refused { caster, refusal });

    // Section 2: whether it may be cast.
    if let Some(refusal) = means.refusal(caster, choice, spell) {
        return refuse(&mut commands, refusal);
    }
    // In a fight a potion or food is taken by its user: it is at them.
    let by_user = means.item(choice.castable).is_some_and(|casts| {
        matches!(casts.by, UseType::Potion | UseType::Food) && **context == UseContext::Combat
    });
    let kind = if by_user { Targeting::Caster } else { spell.targeting };
    let aiming = kind.aiming();
    let side = standings.get(caster).map(|standing| standing.stance).ok();
    let extent = |power| Extent { reach: spell.reach.metres(), radius: blast_radius(spell, power) };

    // Section 5: what it is aimed about; none is no target.
    let reach = spell.reach.metres();
    let about = match aiming.anchor {
        Anchor::Caster => Some(reaching.about(caster)),
        Anchor::Friend => reaching.friend(caster, choice.aim, reach).map(|b| reaching.about(b)),
        Anchor::Foe => reaching.foe(caster, choice.aim, reach).map(|b| reaching.about(b)),
        Anchor::AnyFoe => {
            let reached = Area::Reached { reach };
            let any = !reaching.takes(reached, Some(caster), Against::FoesOf).is_empty();
            any.then(|| reaching.about(caster))
        }
        Anchor::Unaimed => return refuse(&mut commands, Refusal::Unsupported),
    };
    let Some(mut about) = about else { return refuse(&mut commands, Refusal::NoTarget) };

    // Section 3: its power.
    let powered = match means.power(caster, choice, spell, &mut rng) {
        Ok(powered) => powered,
        Err(PlayedBadly { stamina, trains }) => {
            commands.trigger(Pays { caster, castable: choice.castable, power: 0, stamina });
            if let Some(skill) = trains {
                commands.trigger(Practise { member: caster, skill, usage: 1 });
            }
            return refuse(&mut commands, Refusal::PlayedBadly);
        }
    };
    let power = powered.power;
    let extent = extent(power);

    // Section 5: whom its shape takes in about it, its power known.
    let against = Against::of(aiming.against, side);
    let mut victims = reaching.shaped(aiming.shape, about, caster, extent, against);

    // Section 7: a member's spell against enemies provokes whom it reaches,
    // whatever the roll (retail `MakeTargetGroupHostile`); one going again
    // provokes nobody anew.
    let (blind, member, sees) =
        casters.get(caster).map_or((false, false, 0), |(c, member, sees)| {
            (c.has(Condition::Blind), member, sees.map_or(0, |s| s.0))
        });
    if member && spell.aimed_at_enemies() && !again {
        for &monster in &victims {
            commands.trigger(Provocation { monster });
        }
    }

    // Section 4: the roll; Fire under water is quenched.
    let quenched =
        spell.realm == Some(Realm::Fire) && underwater.is_some_and(|state| state.camera_sway);
    let came =
        roll::outcome(&mut rng, powered.failure, powered.quiet, quenched, spell.can_backfire());

    // Section 5: a blind caster's spell that came off may go astray (retail
    // `CheckSpellBackfire`), unless it is aimed at the caster, of the Mental
    // realm, Rocket Blast or an automatic recast: one time in five, but one
    // who sees blind escapes it first. It takes in its shape about another
    // body, or a spot scattered within its reach.
    let guarded = again
        || choice.aim.body() == Some(caster)
        || spell.realm == Some(Realm::Mental)
        || spell.id == SpellId::ROCKET_BLAST;
    let strays =
        came == Outcome::Cast && !guarded && blind && rng.below(100) >= sees && rng.below(100) < 20;
    let astray = match aiming.strays {
        _ if !strays => None,
        Strays::Nowhere => None,
        Strays::Body => reaching.another(caster, &mut rng).map(|body| reaching.about(body)),
        Strays::Spot => {
            Some(About::spot(reaching.scatter(&mut rng, reaching.place(caster).feet, reach)))
        }
    };
    if let Some(astray) = astray {
        news.write(MagicNews::Astray { caster });
        about = astray;
        victims = reaching.shaped(aiming.shape, about, caster, extent, against);
    }

    // Section 6: a backfire falls on the caster's side, in its shape about
    // the caster (retail `PrepareSpellTarget`, `RedirectBackfiredSpellTarget`).
    let backfired = came == Outcome::Backfired;
    if backfired {
        let falls_on = Against::of(aiming.falls_on, side);
        victims = reaching.shaped(aiming.falls, reaching.about(caster), caster, extent, falls_on);
    }
    let here = reaching.place(caster);
    let spot = about.spot;

    // Section 7: it is paid for, whatever the roll.
    let stamina = means.toll(caster, choice, spell, power, &mut rng);
    commands.trigger(Pays { caster, castable: choice.castable, power, stamina });
    let learned = means.learned(choice.castable);
    let item = means.item(choice.castable).map(|_| choice.castable);
    let name = item.and_then(|item| held.id(item)).map(|id| content.items[id].name.clone());
    let report =
        Report { caster, spell: spell.id, power, item: name, outcome: came, effects: vec![] };
    news.write(MagicNews::Done(report));
    let spend = |commands: &mut Commands| {
        if let Some(item) = item {
            commands.trigger(Spend { item, amount: 1 });
        }
    };
    if came == Outcome::Fizzled {
        spend(&mut commands);
        // An auto cast that fizzled goes again at once.
        if learned && choice.power == power::AUTO {
            commands.trigger(Cast { caster, choice, again: true });
        }
        return;
    }

    // Section 5: it is let go. A backfire comes from whom its kind says, as
    // strong as they are; a cone or a blast lands where it went off.
    let owner = match (backfired, aiming.from, choice.aim) {
        (true, ComesFrom::Aimed, Aim::Body(body)) => body,
        (true, ComesFrom::Leader, Aim::Body(body)) => reaching.leader(body),
        _ => caster,
    };
    let at_spot = matches!(aiming.from, ComesFrom::Spot | ComesFrom::SpotToSpot);
    let from = if backfired && at_spot { spot } else { reaching.place(owner).eye };
    let aim = if at_spot { Aim::Point(spot) } else { choice.aim };
    let to = (aiming.from == ComesFrom::SpotToSpot).then_some(spot);
    let strength = match backfired {
        true => {
            let figure = levels.get(owner).map_or(0, |owned| owned.caster_level(spell));
            power::strength(figure, power, powered.strength.boost, spell)
        }
        false => powered.strength,
    };
    let Some(row) = crate::game::hits::definitions::row(spell) else {
        spend(&mut commands);
        return;
    };
    // Turned on its caster's side it gives nothing back when it follows rows
    // of its own.
    let (rows, leeches) = match backfired {
        true if row.backfires_apart(spell) => (Rows::Backfire(spell.id), None),
        true => (Rows::Backfire(spell.id), row.leeches),
        false => (Rows::Spell(spell.id), row.leeches),
    };
    let flies = spell.missile.is_some() && !backfired && !victims.is_empty();
    let arrivals: Vec<(Entity, Option<f32>)> = victims
        .iter()
        .map(|&victim| {
            let to = to.unwrap_or_else(|| reaching.place(victim).feet);
            (victim, flies.then(|| flight(here.eye, to)))
        })
        .collect();
    let strike = Strike::new(rows, strength, Some(caster), &content);
    let practice = means.practice(caster, choice, spell, power);
    let recasts =
        learned && choice.power == power::AUTO && spell.power == PowerChoice::ChosenOrRepeat;
    let owner_side = standings.get(owner).ok().map(|standing| standing.stance);
    let figure =
        |spell: &_| levels.get(owner).map_or(0, |owned: &CasterLevels| owned.caster_level(spell));
    let against = Against::opposing(owner_side, backfired);
    let bursts = Bursts::of(spell, strength, Some(owner), figure, against, &content);
    let turns = row_turnable(spell, backfired).then(|| TurnsBack {
        spell: spell.clone(),
        caster,
        from,
        aim,
    });
    let mut deed =
        commands.spawn((LetGoAt(victims), Practice { caster, skills: practice, sure: flies }));
    if let Some(bursts) = bursts {
        deed.insert(bursts);
    }
    if let Some(turns) = turns {
        deed.insert(turns);
    }
    if let Some(gives) = leeches {
        deed.insert(Leech { caster: owner, gives });
    }
    if recasts {
        deed.insert(Recast { caster, choice, watches_stamina: row.watches_stamina });
    }
    // It lands as it is made: what follows it is beside it first. The item is
    // spent after it is let go, so that the last use is told by name.
    deed.insert(Deed::flying(strike, arrivals));
    spend(&mut commands);
}

/// Whether Eye for an Eye may turn a spell back: one aimed at enemies, with
/// a row, that did not backfire.
fn row_turnable(spell: &crate::content::spells::SpellDefinition, backfired: bool) -> bool {
    crate::game::hits::definitions::row(spell).is_some() && spell.aimed_at_enemies() && !backfired
}
