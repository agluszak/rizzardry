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
use crate::content::spells::{
    ComesFrom, Falls, PowerChoice, SpellId, Spreads, Strays, Takes, Targeting,
};
use crate::game::actors::CasterLevels;
use crate::game::characters::{Condition, Conditions, Practise, Vitals};
use crate::game::combat::{Battle, Combatants};
use crate::game::data::Catalogs;
use crate::game::hits::Deed;
use crate::game::hits::definitions::Rows;
use crate::game::hits::effects::Strike;
use crate::game::items::{Items, Spend};
use crate::game::magic::aim::{Against, Area, Reaching};
use crate::game::magic::castable::CastableSpell;
use crate::game::magic::landed::{Leech, Spell};
use crate::game::magic::may::Refusal;
use crate::game::magic::reach::{Aim, blast_radius, flight};
use crate::game::magic::{Outcome, power, roll};
use crate::game::rng::GameRng;
use crate::game::world::hostility::{Provocation, Standing};
use crate::game::world::level::InPlay;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::Monster;
use crate::game::world::triggers::LevelState;

/// A spell is cast, and what comes of it happens: it is refused and nothing
/// is spent; or it is aimed, its power settled, the one it is against
/// provoked, rolled, paid for, and, unless it fizzled, let go as a deed.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn cast(
    cast: On<Cast>,
    spells: Query<&CastableSpell>,
    means: Means,
    (reaching, obstacles): (Reaching, WorldObstacles),
    (standings, conditions, levels): (Query<&Standing>, Query<&Conditions>, Query<&CasterLevels>),
    (fighting, living): (
        Option<Single<&Combatants, With<Battle>>>,
        Query<(&Vitals, Option<&Monster>)>,
    ),
    underwater: Option<Single<&LevelState, With<InPlay>>>,
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
        matches!(casts.by, UseType::Potion | UseType::Food) && fighting.is_some()
    });
    let kind = if by_user { Targeting::Caster } else { spell.targeting };
    let aiming = kind.aiming();
    let side = standings.get(caster).map(|standing| standing.stance).ok();
    let here = reaching.place(caster);
    let reach = spell.reach.metres();

    // Section 5: its first aim.
    let mut spot = None;
    let mut victims = match aiming.takes {
        Takes::Caster => vec![caster],
        Takes::Party => reaching.takes(Area::Party, Some(caster), Against::FoesOf),
        Takes::Friend => {
            reaching.takes(Area::Friend { at: choice.aim, reach }, Some(caster), Against::FoesOf)
        }
        Takes::Reached => reaching.takes(Area::Reached { reach }, Some(caster), Against::FoesOf),
        Takes::Foe => {
            let foe = reaching.foe(caster, choice.aim, reach);
            spot = foe.map(|foe| reaching.place(foe).feet);
            foe.into_iter().collect()
        }
        Takes::Unaimed => return refuse(&mut commands, Refusal::Unsupported),
    };
    if victims.is_empty() && !matches!(aiming.takes, Takes::Caster | Takes::Party) {
        return refuse(&mut commands, Refusal::NoTarget);
    }
    let aimed = victims.first().copied();

    // Section 3: its power.
    let powered = match means.power(caster, choice, spell, &mut rng) {
        Ok(powered) => powered,
        Err(PlayedBadly { stamina, trains }) => {
            commands.trigger(Pays { caster, points: None, stamina });
            if let Some(skill) = trains {
                commands.trigger(Practise { member: caster, skill, usage: 1 });
            }
            return refuse(&mut commands, Refusal::PlayedBadly);
        }
    };
    let power = powered.power;
    let radius = blast_radius(spell, power);
    let foe_feet = spot.unwrap_or(here.feet);

    // Section 5: the area its power spreads it over.
    match (aiming.spreads, aimed) {
        (Spreads::Group, Some(first)) => {
            victims = reaching.takes(Area::Group(first), Some(caster), Against::Everyone);
        }
        (Spreads::Cone, _) => {
            let cone = Area::Cone { origin: here.eye, spot: foe_feet, reach };
            victims = reaching.takes(cone, Some(caster), Against::Everyone);
        }
        (Spreads::Blast, _) => {
            let blast = Area::Blast { spot: foe_feet, origin: foe_feet, radius };
            victims = reaching.takes(blast, Some(caster), Against::opposing(side, false));
        }
        // A member's spell at the party reaches the monsters on the party's
        // side about it.
        (Spreads::Widen, _) if means.by_member(choice.castable) && radius > 0.0 => {
            let about = radius + crate::game::party::body::RADIUS;
            let area = Area::Blast { spot: here.centre, origin: here.eye, radius: about };
            let allies = reaching.takes(area, Some(caster), Against::opposing(side, true));
            let party: Vec<Entity> = reaching.party().collect();
            victims.extend(allies.into_iter().filter(|ally| !party.contains(ally)));
        }
        _ => {}
    }

    // Section 7: a member's spell against enemies provokes whom it reaches,
    // whatever the roll (retail `MakeTargetGroupHostile`); one going again
    // provokes nobody anew.
    if means.by_member(choice.castable) && spell.aimed_at_enemies() && !again {
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
    // who sees blind escapes it first.
    let guarded = again
        || choice.aim.body() == Some(caster)
        || spell.realm == Some(Realm::Mental)
        || spell.id == SpellId::ROCKET_BLAST;
    let blind = conditions.get(caster).is_ok_and(|c| c.has(Condition::Blind));
    let strays = came == Outcome::Cast
        && !guarded
        && blind
        && rng.below(100) >= powered.sees_blind
        && rng.below(100) < 20;
    if strays {
        let astray = match aiming.strays {
            Strays::Nowhere => None,
            Strays::AtOne | Strays::AtGroup => {
                // At another body in the fight at random, members and living
                // monsters alike, never the caster (`PickBackfireTarget`).
                let others = others(caster, &reaching, fighting.as_deref().copied(), &living);
                (!others.is_empty()).then(|| {
                    let body = others[rng.below(others.len() as u32) as usize];
                    spot = Some(reaching.place(body).feet);
                    match aiming.strays {
                        Strays::AtGroup => {
                            reaching.takes(Area::Group(body), Some(caster), Against::Everyone)
                        }
                        _ => vec![body],
                    }
                })
            }
            Strays::Cone => {
                let to = scatter(&mut rng, here.feet, reach, &obstacles);
                spot = Some(to);
                let cone = Area::Cone { origin: here.eye, spot: to, reach };
                Some(reaching.takes(cone, Some(caster), Against::Everyone))
            }
            Strays::Blast => {
                let to = scatter(&mut rng, here.feet, reach, &obstacles);
                spot = Some(to);
                let blast = Area::Blast { spot: to, origin: to, radius };
                Some(reaching.takes(blast, Some(caster), Against::opposing(side, false)))
            }
        };
        if let Some(astray) = astray {
            news.write(MagicNews::Astray { caster });
            victims = astray;
        }
    }

    // Section 6: a backfire falls on the caster's side (retail
    // `PrepareSpellTarget`, `RedirectBackfiredSpellTarget`).
    let backfired = came == Outcome::Backfired;
    if backfired {
        victims = match aiming.falls {
            Falls::Caster => vec![caster],
            Falls::Group => reaching.takes(Area::Group(caster), Some(caster), Against::Everyone),
            Falls::Blast => {
                let blast = Area::Blast { spot: here.feet, origin: here.feet, radius };
                reaching.takes(blast, Some(caster), Against::opposing(side, true))
            }
            Falls::Fellows => {
                reaching.takes(Area::Reached { reach }, Some(caster), Against::FellowsOf)
            }
        };
    }

    // Section 7: it is paid for, whatever the roll.
    let stamina = means.toll(caster, choice, spell, power, &mut rng);
    let learned = means.learned(choice.castable);
    let points = spell.realm.filter(|_| learned).map(|realm| (realm, spell.cost * power));
    commands.trigger(Pays { caster, points, stamina });
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
    let from = match (backfired && at_spot, spot) {
        (true, Some(spot)) => spot,
        _ => reaching.place(owner).eye,
    };
    let aim = match (at_spot, spot) {
        (true, Some(spot)) => Aim::Point(spot),
        _ => choice.aim,
    };
    let to = spot.filter(|_| aiming.from == ComesFrom::SpotToSpot);
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
    let mut deed = commands.spawn((
        Spell {
            id: spell.id,
            caster,
            backfired,
            reflected: false,
            owner: Some(owner),
            from,
            aim,
            victims,
        },
        Practice { caster, skills: practice, sure: flies },
    ));
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

/// The bodies a spell gone astray at a body may go at, never the caster:
/// the members by seat, then the living monsters in the fight as placed.
fn others(
    caster: Entity,
    reaching: &Reaching,
    fighting: Option<&Combatants>,
    living: &Query<(&Vitals, Option<&Monster>)>,
) -> Vec<Entity> {
    let party: Vec<Entity> = reaching.party().collect();
    let mut monsters: Vec<(i64, Entity)> = fighting
        .iter()
        .flat_map(|fight| fight.entities().iter().copied())
        .filter(|&body| body != caster && !party.contains(&body))
        .filter_map(|body| {
            let (vitals, monster) = living.get(body).ok()?;
            (!vitals.is_dead()).then(|| (monster.map_or(i64::MAX, |m| i64::from(m.location)), body))
        })
        .collect();
    monsters.sort_by_key(|&(location, _)| location);
    let members = party.into_iter().filter(|&member| member != caster);
    members.chain(monsters.into_iter().map(|(_, body)| body)).collect()
}

/// A spot at random within `reach` of `origin` (retail
/// `ScatterSpellPointTarget`): up to five tries for one that stands on
/// ground, each stopped short by what is solid in the way; one too far is
/// drawn back to the reach.
fn scatter(rng: &mut GameRng, origin: Vec3, reach: f32, obstacles: &WorldObstacles) -> Vec3 {
    let mut spot = origin;
    for _ in 0..5 {
        let across = |rng: &mut GameRng| (rng.below(2001) as f32 - 1000.0) / 1000.0 * reach;
        let (x, z) = (across(rng), across(rng));
        let up = rng.below(1001) as f32 / 1000.0 * reach;
        let mut point = origin + Vec3::new(x, up, z);
        if let Ok(way) = Dir3::new(point - origin)
            && let Some(hit) = obstacles.first_hit(origin, way, origin.distance(point))
        {
            point = origin + *way * hit.distance;
        }
        spot = point;
        if let Some(ground) = obstacles.ground_below(point, up + reach) {
            spot = ground;
            break;
        }
    }
    if spot.distance(origin) > reach {
        spot = origin + (spot - origin).normalize_or_zero() * reach;
    }
    spot
}
