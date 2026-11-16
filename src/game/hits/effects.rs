//! What a deed does to each victim it reaches: its rows in turn, each
//! validated, saved against and resisted ([`lands`]), then made to happen by
//! whichever observer the row's kind is about, each reading only what it
//! changes: health ([`health`]), what ails the victim ([`ailments`]), its
//! spell points ([`points`]), the spells laid on it ([`laid`]) and what heat
//! takes of what it carries ([`burn`]). Every row sees how the one before it
//! came out. Bevy carries each event out before the next is begun, so a row
//! has fully landed before the next is weighed.

use bevy::prelude::*;
use std::sync::Arc;

use super::applied::{self, Outcome};
use super::defences;
use super::definitions::{self, Blow, EffectKind, EffectSpec, Rows};
use super::{Damaged, Deed, Effect};
use crate::content::characters::{Attribute, Skill};
use crate::content::items::EquipClass;
use crate::content::spells::{SpellDefinition, SpellId};
use crate::game::actors::spells::{Enchantment, Laying};
use crate::game::actors::{Attributes, Drained, Level, Resistances, Swallowed};
use crate::game::characters::{Calling, Condition, Conditions, Lasting, Practise, Vitals};
use crate::game::data::Catalogs;
use crate::game::difficulty::Difficulty;
use crate::game::inventory::PartyInventory;
use crate::game::items::{Items, Lose};
use crate::game::magic::{SpellPoints, Strength};
use crate::game::party::roster::{PartyMember, PartyRoster};
use crate::game::rng::GameRng;
use crate::game::world::hostility::Standing;
use crate::game::world::population::SpellBudget;

/// One thing landing: the rows it follows at a strength, a weapon's blow
/// behind them when it is one.
#[derive(Clone, Debug)]
pub struct Strike {
    pub effects: Arc<[EffectSpec]>,
    pub spell: Option<SpellDefinition>,
    pub strength: Strength,
    pub blow: Option<Blow>,
    /// Who does it, and answers for a death it brings
    /// ([`Vitals::hurt`](crate::game::characters::Vitals::hurt)); nobody for
    /// a cloud's pulse, a trap or a device.
    pub by: Option<Entity>,
}

impl Strike {
    /// Resolve the authored effects once. Landing reads the deed, never its table again.
    pub fn new(
        rows: Rows,
        strength: Strength,
        blow: Option<Blow>,
        by: Option<Entity>,
        content: &Catalogs,
    ) -> Self {
        let spell = rows.spell().map(|id| content.spells[id].clone());
        Self { effects: rows.effects(content).into(), spell, strength, blow, by }
    }
}

/// A deed lands its row `row` on a victim, and then the next.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub(super) struct Lands {
    #[event_target]
    pub deed: Entity,
    pub victim: Entity,
    pub row: usize,
}

/// A row got through to a victim at a magnitude, for the observer of what
/// it is about to make happen.
#[derive(EntityEvent, Clone, Debug)]
struct Applies {
    #[event_target]
    deed: Entity,
    victim: Entity,
    row: EffectSpec,
    magnitude: u32,
}

/// A row has been applied to one who was alive before it, which may have
/// run their health out.
#[derive(EntityEvent, Clone, Copy, Debug)]
struct Settles {
    #[event_target]
    victim: Entity,
}

/// A deed's rows for the party as a whole land, once.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub(super) struct LandsOnParty {
    #[event_target]
    pub deed: Entity,
}

/// Weigh one row on a victim (retail `ApplySpellEffectToTarget`,
/// `ApplyEffectConditions`): whether it is for them, whether it waits on the
/// row before, its roll, and the defences of [`super::defences`]. What gets
/// through is applied; then the next row.
pub(super) fn lands(
    lands: On<Lands>,
    mut deeds: Query<&mut Deed>,
    victims: Query<(&Vitals, &Level, &Standing, &Resistances, Has<PartyMember>)>,
    trained: Query<(), With<Calling>>,
    difficulty: Res<Difficulty>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    let Lands { deed: id, victim, row } = *lands;
    let Ok(mut deed) = deeds.get_mut(id) else { return };
    if row == 0 {
        deed.outcome = None;
    }
    let Some(spec) = deed.rows.get(row).cloned() else { return };
    let Ok((vitals, level, standing, resistances, member)) = victims.get(victim) else {
        return;
    };
    let next = Lands { row: row + 1, ..*lands };
    if !applied::is_for(&spec, member, resistances.kind) {
        commands.trigger(next);
        return;
    }
    if applied::waits(&spec, deed.outcome) {
        deed.outcome = Some(Outcome::Missed);
        commands.trigger(next);
        return;
    }
    let spec = spec.pick(&mut rng);
    let strike = &deed.strike;
    let spell = strike.spell.as_ref();
    let rolled = spec.roll.map_or(0, |roll| {
        definitions::roll(roll, strike.strength, strike.blow.as_ref(), spell, &mut rng)
    });
    let alive = !vitals.is_dead();
    let defended = defences::defend(
        &mut rng,
        (&spec, spec.realm(spell)),
        rolled,
        strike.strength,
        *difficulty,
        (!alive, level.0, standing.stance, resistances),
    );
    match defended {
        defences::Defended::Through(magnitude) => {
            deed.outcome = Some(Outcome::Missed);
            commands.trigger(Applies { deed: id, victim, row: spec, magnitude });
            if alive {
                commands.trigger(Settles { victim });
            }
        }
        defences::Defended::Missed => deed.outcome = Some(Outcome::Missed),
        defences::Defended::Resisted { saved } => {
            if saved && trained.contains(victim) {
                commands.trigger(Practise { member: victim, skill: Skill::IronWill, usage: 2 });
            }
            deed.effects.push(Effect::Resisted { victim });
            deed.outcome = Some(Outcome::Resisted);
        }
    }
    commands.trigger(next);
}

/// What a row does to health and stamina: harm, a drain of the most there
/// can be, death outright, tiring, healing, rest and raising the dead. Harm
/// is told of, and plain harm may wake a sleeper.
#[allow(clippy::type_complexity)]
fn health(
    applies: On<Applies>,
    mut deeds: Query<&mut Deed>,
    mut victims: Query<(&mut Vitals, &mut Conditions, &mut Drained, &Attributes, Has<Calling>)>,
    mut laying: Laying,
    mut rng: ResMut<GameRng>,
    mut damaged: MessageWriter<Damaged>,
) {
    use EffectKind as K;
    let Applies { deed, victim, ref row, magnitude } = *applies;
    let Ok(mut deed) = deeds.get_mut(deed) else { return };
    let Ok((mut vitals, mut conditions, mut drained, a, trained)) = victims.get_mut(victim) else {
        return;
    };
    let by = deed.strike.by;
    let nothing_or =
        |amount: u32, effect: Effect| if amount == 0 { Effect::Nothing { victim } } else { effect };
    let (lost, harm) = match row.kind {
        K::Harm => {
            // A leech gives back the magnitude the victim's health could
            // bear, before a Guardian Angel takes any (retail
            // `DrainTargetsLife`, `Magic Effects.cpp:1626-1650`).
            deed.harm += magnitude.min(vitals.health.current);
            (vitals.hurt(laying.absorb(victim, magnitude), by), true)
        }
        K::Sap => (vitals.drain_health(&mut conditions, &mut drained, magnitude, by), false),
        K::Slay => (vitals.slay(by), false),
        K::Tire => {
            let amount = vitals.tire(magnitude);
            return deed.landed(Effect::Tired { victim, amount });
        }
        K::Heal => {
            let amount = vitals.heal(magnitude);
            return deed.landed(nothing_or(amount, Effect::Healed { victim, amount }));
        }
        K::Refresh => {
            let amount = vitals.rest(magnitude);
            return deed.landed(nothing_or(amount, Effect::Refreshed { victim, amount }));
        }
        // Only one who trained is raised.
        K::Raise if trained && !vitals.is_dead() => {
            deed.effects.push(Effect::Nothing { victim });
            return;
        }
        K::Raise if trained => {
            let (health, stamina) = applied::raised(
                vitals.health.max,
                vitals.stamina.max,
                a[Attribute::Piety],
                a[Attribute::Vitality],
            );
            vitals.raise(health, stamina);
            return deed.landed(Effect::Raised { victim });
        }
        _ => return,
    };
    deed.landed(Effect::Damaged { victim, amount: lost });
    if lost > 0 {
        damaged.write(Damaged { body: victim, amount: lost, died: vitals.is_dead() });
    }
    if harm && lost > 0 && !vitals.is_dead() {
        conditions.wake_on_hit(a[Attribute::Senses], &mut rng);
    }
}

/// What a row does to what ails a victim: afflicting it, swallowing it,
/// curing, cleansing and lifting a drain.
fn ailments(
    applies: On<Applies>,
    mut deeds: Query<&mut Deed>,
    mut victims: Query<(&mut Conditions, &mut Drained, &Standing)>,
    mut laying: Laying,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    use EffectKind as K;
    let Applies { deed, victim, ref row, magnitude } = *applies;
    let Ok(mut deed) = deeds.get_mut(deed) else { return };
    let Ok((mut conditions, mut drained, standing)) = victims.get_mut(victim) else { return };
    let effect = match row.kind {
        K::Afflict { condition, poison, indefinite } => {
            let stance = standing.stance;
            let Some(lasting) = applied::afflicts(condition, stance, indefinite, magnitude) else {
                return;
            };
            if condition == Condition::Poisoned {
                conditions.poison(lasting, poison);
            } else if condition != Condition::Slowed {
                conditions.afflict(condition, lasting);
            } else if let Some(left) = laying.slow(victim, lasting) {
                conditions.afflict(condition, left);
            }
            Effect::Inflicted { victim, condition }
        }
        K::Swallow => {
            let Some(by) = deed.strike.by else { return };
            conditions.afflict(Condition::Missing, Lasting::Indefinite);
            commands.entity(victim).insert(Swallowed(by));
            Effect::Inflicted { victim, condition: Condition::Missing }
        }
        K::Cure { conditions: ref cures, outright } => {
            let strength = deed.strike.strength;
            if cures.contains(&Condition::Poisoned) {
                conditions.weaken_poison(defences::cure_rounds(strength), &mut rng);
            }
            let cured = defences::cure(&mut rng, &conditions, cures, strength, outright);
            for &(condition, left) in &cured.shortened {
                conditions.shorten(condition, left);
            }
            for &condition in &cured.lifted {
                conditions.lift(condition);
            }
            match cured.lifted.first() {
                Some(&condition) => Effect::Cured { victim, condition },
                None => Effect::Nothing { victim },
            }
        }
        K::Cleanse => {
            let condition = conditions
                .iter()
                .map(|&(condition, _)| condition)
                .find(|&condition| condition != Condition::Infatuated);
            conditions.lift_all();
            condition
                .map_or(Effect::Nothing { victim }, |condition| Effect::Cured { victim, condition })
        }
        K::Renew if drained.lift(&mut conditions) => {
            Effect::Cured { victim, condition: Condition::Drained }
        }
        K::Renew => Effect::Nothing { victim },
        _ => return,
    };
    deed.landed(effect);
}

/// What a row does to a victim's spell points, or a monster's budget of
/// them: draining and replenishing.
fn points(
    applies: On<Applies>,
    mut deeds: Query<&mut Deed>,
    mut casters: Query<&mut SpellPoints>,
    mut budgets: Query<&mut SpellBudget>,
    mut rng: ResMut<GameRng>,
) {
    use EffectKind as K;
    let Applies { deed, victim, ref row, magnitude } = *applies;
    let Ok(mut deed) = deeds.get_mut(deed) else { return };
    if row.kind == K::DrainBudget {
        let Ok(mut budget) = budgets.get_mut(victim) else { return };
        let amount = budget.0.min(magnitude);
        budget.0 -= amount;
        return deed.landed(Effect::Drained { victim, amount });
    }
    if !matches!(row.kind, K::DrainPool | K::DrainRealm(_) | K::Replenish) {
        return;
    }
    let Ok(mut pool) = casters.get_mut(victim) else { return };
    let effect = match row.kind {
        K::Replenish => {
            let amount = pool.restore_evenly(magnitude);
            if amount == 0 {
                Effect::Nothing { victim }
            } else {
                Effect::Replenished { victim, amount }
            }
        }
        K::DrainRealm(realm) => {
            let realms = realm.map_or(enumset::EnumSet::all(), enumset::EnumSet::only);
            let amount = realms.iter().map(|realm| pool.drain_realm(realm, magnitude)).sum();
            Effect::Drained { victim, amount }
        }
        _ => Effect::Drained { victim, amount: pool.drain(magnitude, &mut rng) },
    };
    deed.landed(effect);
}

/// What a row does to the spells laid on a victim: purifying them, and
/// laying one. Haste first spends itself on a slowing; wholly spent, it
/// changed something all the same.
fn laid(
    applies: On<Applies>,
    mut deeds: Query<&mut Deed>,
    mut victims: Query<(&Vitals, &mut Conditions)>,
    mut laying: Laying,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
) {
    use EffectKind as K;
    let Applies { deed, victim, ref row, magnitude } = *applies;
    let Ok(mut deed) = deeds.get_mut(deed) else { return };
    let strength = deed.strike.strength;
    let own = || deed.strike.spell.as_ref().expect("only a spell's row is the spell's own");
    let (spell, answers_slowing) = match &row.kind {
        K::SpellPurify => {
            laying.purify(victim, magnitude + own().level);
            return deed.landed(Effect::Purified { victim });
        }
        K::Enchant(spell) => (*spell, answers_slowing(&content.spells[*spell])),
        K::SpellEnchant => (own().id, answers_slowing(own())),
        _ => return,
    };
    let Ok((vitals, mut conditions)) = victims.get_mut(victim) else { return };
    let turns = if answers_slowing { conditions.hasted(magnitude) } else { magnitude };
    let dead = vitals.is_dead();
    let laid = !dead && lay(&mut laying, victim, spell, turns, strength, &content, &mut rng);
    let spent = answers_slowing && turns == 0 && magnitude > 0;
    deed.landed(if !dead && (laid || spent) {
        Effect::Enchanted { victim, spell }
    } else {
        Effect::Nothing { victim }
    });
}

/// Heat takes what one who trained carries.
fn burn(
    applies: On<Applies>,
    mut deeds: Query<&mut Deed>,
    trained: Query<(), With<Calling>>,
    held: Items,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    let Applies { deed, victim, ref row, magnitude } = *applies;
    if row.kind != EffectKind::Burn || !trained.contains(victim) {
        return;
    }
    let Ok(mut deed) = deeds.get_mut(deed) else { return };
    deed.outcome = Some(Outcome::Landed);
    let burnt = scorch(held.carried(victim), &held, magnitude, &mut rng, &mut commands);
    deed.effects.extend(burnt);
}

/// One whose health a row ran out has fallen, and it is settled before
/// anything else lands on them ([`crate::game::characters::fall`]).
fn settles(settles: On<Settles>, victims: Query<&Vitals>, mut commands: Commands) {
    if victims.get(settles.victim).is_ok_and(Vitals::is_dead) {
        commands.trigger(crate::game::characters::Fell { body: settles.victim });
    }
}

/// A deed's rows for the party itself land once (retail visits the party's
/// shared slots, pool and purse apart from its members): what the living
/// members have of spell points, the spells laid on the party, the pool
/// that heat takes from, and the purse.
#[allow(clippy::type_complexity)]
fn lands_on_party(
    lands: On<LandsOnParty>,
    mut deeds: Query<&mut Deed>,
    party: Single<(Entity, &mut PartyInventory, &PartyRoster)>,
    mut members: Query<(&Vitals, &mut SpellPoints)>,
    mut laying: Laying,
    held: Items,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    use EffectKind as K;
    let Ok(mut deed) = deeds.get_mut(lands.deed) else { return };
    let (party, mut inventory, roster) = party.into_inner();
    let victim = party;
    let strike = deed.strike.clone();
    let own = || strike.spell.as_ref().expect("only a spell's row is the spell's own");
    let mut previous = None;
    for spec in deed.party_rows.clone() {
        if applied::waits(&spec, previous) {
            previous = Some(Outcome::Missed);
            continue;
        }
        let magnitude = spec.roll.map_or(0, |roll| {
            definitions::roll(
                roll,
                strike.strength,
                strike.blow.as_ref(),
                strike.spell.as_ref(),
                &mut rng,
            )
        });
        if spec.chance.is_some_and(|chance| rng.below(100) >= chance) {
            previous = Some(Outcome::Missed);
            continue;
        }
        match spec.kind {
            K::DrainPool => {
                for member in roster.iter() {
                    let Ok((vitals, mut pool)) = members.get_mut(member) else { continue };
                    if vitals.is_dead() {
                        continue;
                    }
                    let amount = pool.drain(magnitude, &mut rng);
                    deed.effects.push(Effect::Drained { victim: member, amount });
                }
            }
            K::SpellPurify => {
                laying.purify(party, magnitude + own().level);
                deed.effects.push(Effect::Purified { victim });
            }
            K::SpellEnchant => {
                let spell = own().id;
                let laid =
                    lay(&mut laying, party, spell, magnitude, strike.strength, &content, &mut rng);
                deed.effects.push(if laid {
                    Effect::Enchanted { victim, spell }
                } else {
                    Effect::Nothing { victim }
                });
            }
            K::Burn => {
                let pool = held.pool(party).iter().copied();
                let burnt = scorch(pool, &held, magnitude, &mut rng, &mut commands);
                deed.effects.extend(burnt);
            }
            K::LoseGold => {
                // A percent of the purse, at most all of it.
                let amount = inventory.gold * magnitude.min(100) / 100;
                inventory.gold -= amount;
                deed.effects.push(Effect::GoldLost { amount });
            }
            K::Dispel => laying.dispel(party),
            _ => unreachable!("not an effect on the party itself"),
        }
        previous = Some(Outcome::Landed);
    }
}

/// Lay a spell on a bearer for `turns` at a cast's strength; the absorption of
/// a spell that absorbs damage is rolled as it is laid. Whether it took.
fn lay(
    laying: &mut Laying,
    bearer: Entity,
    spell: SpellId,
    turns: u32,
    strength: Strength,
    content: &Catalogs,
    rng: &mut GameRng,
) -> bool {
    let definition = &content.spells[spell];
    let absorbing = definitions::row(definition).is_some_and(|row| row.absorbs);
    let absorbs = applied::absorbs(rng, absorbing, definition.dice, strength.power, strength.boost);
    let laid = Enchantment { spell, power: strength.power, boost: strength.boost };
    laying.lay(bearer, laid, turns, absorbs)
}

/// Heat takes each potion, scroll or book among `items` at `chance` percent,
/// each of a stack on its own; what it took is lost, and told of by class.
fn scorch(
    items: impl IntoIterator<Item = Entity>,
    held: &Items,
    chance: u32,
    rng: &mut GameRng,
    commands: &mut Commands,
) -> Vec<Effect> {
    let classes = [EquipClass::Potion, EquipClass::Scroll, EquipClass::Spellbook];
    let mut burnt = [0; 3];
    for item in items {
        let Some(class) = held.flammable(item) else { continue };
        let lost = applied::burnt(rng, held.units(item), chance);
        if lost > 0 {
            burnt[classes.iter().position(|&burns| burns == class).unwrap()] += u32::from(lost);
            commands.trigger(Lose { item, amount: lost });
        }
    }
    classes
        .into_iter()
        .zip(burnt)
        .filter(|&(_, amount)| amount != 0)
        .map(|(class, amount)| Effect::Destroyed { class, amount })
        .collect()
}

/// Haste first spends itself on a slowing.
fn answers_slowing(spell: &SpellDefinition) -> bool {
    definitions::row(spell).is_some_and(|row| row.answers_slowing)
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(lands)
        .add_observer(health)
        .add_observer(ailments)
        .add_observer(points)
        .add_observer(laid)
        .add_observer(burn)
        .add_observer(settles)
        .add_observer(lands_on_party);
}
