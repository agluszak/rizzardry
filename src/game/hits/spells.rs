//! Spells landing on bodies (retail `ProcessSpellEffectTargets`,
//! `ResolveSpellMissileHit` and the effect functions of `Magic Effects.cpp`),
//! table-driven by spell: a [`SpellHit`] on each victim, landed by
//! [`land_spells`]; what a drain or a burst does once all of a deed's hits
//! have landed follows in [`drain_life`] and [`boil_blood`].

use bevy::prelude::*;

use super::Effect;
use super::body::{self, Body, BodyItem, Defence, INDEFINITE, wound};
use super::{Deed, DeedHits, HitOf, Strikes};
use crate::content::characters::Attribute;
use crate::content::items::{Dice, EquipClass, ItemCatalog};
use crate::content::species::{MonsterCatalog, MonsterKind};
use crate::content::spells::{SpellDefinition, SpellId};
use crate::game::characters::{Condition, Lasting, Vitals};
use crate::game::difficulty::Difficulty;
use crate::game::factions::Factions;
use crate::game::inventory::PartyInventory;
use crate::game::magic::Strength;
use crate::game::magic::enchantments::{Enchantment, Enchantments, PartyEffects};
use crate::game::party::DamageApplied;
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;
use crate::game::world::identify::MonsterGroup;

/// What a spell does, by retail's effect functions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    /// `ApplyDamageToTargets` or a missile hit: the magnitude less resistance.
    Damage,
    /// Damage to undead (and Banish's other kinds) only; others are untouched.
    DamageUndead {
        banish: bool,
    },
    Enchant,
    Heal,
    Stamina,
    /// Healing followed by stamina with twice the dice count.
    RestoreHealth,
    RestoreMagic,
    GoldLoss,
    DestroyConsumables,
    DrainMagic,
    /// Heal, restore stamina and lift conditions 2..17 but infatuation.
    Restoration,
    /// `InflictConditionAttack` at a percent chance.
    Inflict(Condition, u32),
    /// Damage, then a condition attack on every victim.
    DamageThen(Condition, u32),
    /// Damage, then nausea at 90%, then unconsciousness at 25%.
    NoxiousFumes,
    /// Fear; a party member it lands on is attacked again for unconsciousness
    /// (Terror), a monster for more fear (Sonic Boom), at half strength.
    Fear {
        second: Condition,
    },
    /// Insanity; one who saves is afraid anyway.
    Pandemonium,
    /// Death on a failed save, unless resistance cuts the attempt to nothing.
    Slay,
    /// Half the time a weighted condition, otherwise damage.
    Affliction,
    Cure(&'static [Condition]),
    /// Lift Drained, and with it what draining and disease took.
    Renewal,
    /// `DamageTargetsAndReport`: damage that drains party members' health
    /// for good.
    Vortex,
    Resurrect,
    /// Drain health; 70% of it heals the most wounded party member.
    Lifesteal,
    MightToMagic,
    Purify,
}

/// The spells implemented, by retail id (`spell_ids.h`), and what they do.
pub(crate) fn kind(spell: SpellId) -> Option<Kind> {
    use Condition::*;
    Some(match spell {
        SpellId::BOILING_BLOOD
        | SpellId::BOILING_BLOOD_EXPLOSION
        | SpellId::ACID_SPLASH
        | SpellId::FROST
        | SpellId::ENERGY_BLAST
        | SpellId::MAKE_WOUNDS
        | SpellId::MAGIC_MISSILES
        | SpellId::SHRILL_SOUND
        | SpellId::FIREBALL
        | SpellId::PSIONIC_FIRE
        | SpellId::WHIPPING_ROCKS
        | SpellId::CRUSH
        | SpellId::EGO_WHIP
        | SpellId::FIRE_BOMB
        | SpellId::ICEBALL
        | SpellId::WHIRLWIND
        | SpellId::DEHYDRATE
        | SpellId::LIGHTNING
        | SpellId::EARTHQUAKE
        | SpellId::FALLING_STARS
        | SpellId::MIND_FLAY
        | SpellId::NUCLEAR_BLAST
        | SpellId::TSUNAMI => Kind::Damage,
        SpellId::EYE_FOR_AN_EYE
        | SpellId::DRACON_BREATH
        | SpellId::SHADOW_HOUND
        | SpellId::CHAMELEON
        | SpellId::ACID_BOMB
        | SpellId::DEATH_CLOUD
        | SpellId::TOXIC_CLOUD
        | SpellId::DRAINING_CLOUD
        | SpellId::FIRESTORM
        | SpellId::RAZOR_CLOAK
        | SpellId::ARMORMELT
        | SpellId::RING_OF_FIRE
        | SpellId::LIGHT
        | SpellId::X_RAY
        | SpellId::BLESS
        | SpellId::DETECT_SECRETS
        | SpellId::ENCHANTED_BLADE
        | SpellId::GUARDIAN_ANGEL
        | SpellId::MISSILE_SHIELD
        | SpellId::ARMORPLATE
        | SpellId::MAGIC_SCREEN
        | SpellId::ELEMENT_SHIELD
        | SpellId::HASTE
        | SpellId::SOUL_SHIELD
        | SpellId::SUPERMAN
        | SpellId::BODY_OF_STONE => Kind::Enchant,
        SpellId::HOLY_WATER => Kind::DamageUndead { banish: false },
        SpellId::BANISH => Kind::DamageUndead { banish: true },
        SpellId::HEAL_WOUNDS | SpellId::HEAL_ALL => Kind::Heal,
        SpellId::STAMINA | SpellId::REST_ALL => Kind::Stamina,
        SpellId::RESTORATION => Kind::Restoration,
        SpellId::ITCHING_SKIN => Kind::Inflict(Irritated, 100),
        SpellId::PARALYZE | SpellId::FREEZE_FLESH | SpellId::FREEZE_ALL => {
            Kind::Inflict(Paralyzed, 100)
        }
        SpellId::SLEEP => Kind::Inflict(Asleep, 100),
        SpellId::BLINDING_FLASH => Kind::Inflict(Blind, 100),
        SpellId::INSANITY => Kind::Inflict(Insane, 100),
        SpellId::SLOW => Kind::Inflict(Slowed, 100),
        SpellId::WEB => Kind::Inflict(Webbed, 100),
        SpellId::SILENCE => Kind::Inflict(Silenced, 100),
        SpellId::HEX => Kind::Inflict(Hexed, 100),
        SpellId::MIND_STAB | SpellId::PSIONIC_BLAST => Kind::DamageThen(Insane, 25),
        SpellId::BLIZZARD => Kind::DamageThen(Blind, 25),
        SpellId::CEREBRAL_HEMORRHAGE => Kind::DamageThen(Insane, 75),
        SpellId::CONCUSSION => Kind::DamageThen(Unconscious, 50),
        SpellId::NOXIOUS_FUMES => Kind::NoxiousFumes,
        SpellId::TERROR => Kind::Fear { second: Unconscious },
        SpellId::SONIC_BOOM => Kind::Fear { second: Afraid },
        SpellId::PANDEMONIUM => Kind::Pandemonium,
        SpellId::INSTANT_DEATH
        | SpellId::QUICKSAND
        | SpellId::ASPHYXIATION
        | SpellId::DEATH_WISH => Kind::Slay,
        SpellId::PRISMIC_RAY | SpellId::PRISMIC_CHAOS => Kind::Affliction,
        SpellId::CURE_LESSER_COND => Kind::Cure(&[Irritated, Nauseated, Afraid, Asleep, Blind]),
        SpellId::CURE_PARALYSIS => Kind::Cure(&[Paralyzed, Webbed]),
        SpellId::CURE_POISON => Kind::Cure(&[Poisoned]),
        SpellId::CURE_DISEASE => Kind::Cure(&[Diseased]),
        SpellId::REMOVE_CURSE => Kind::Cure(&[Hexed]),
        SpellId::TURNCOAT => Kind::Inflict(Turncoat, 100),
        SpellId::SANE_MIND => Kind::Cure(&[Insane, Turncoat]),
        SpellId::RESURRECTION => Kind::Resurrect,
        SpellId::LIFESTEAL => Kind::Lifesteal,
        SpellId::MIGHT_TO_MAGIC => Kind::MightToMagic,
        SpellId::PURIFY_AIR => Kind::Purify,
        SpellId::PHILOSOPHERS_BANE => Kind::GoldLoss,
        SpellId::VAMPIRIC_VORTEX => Kind::Vortex,
        SpellId::RENEWAL => Kind::Renewal,
        SpellId::HEATWAVE => Kind::DestroyConsumables,
        SpellId::ANTI_MAGIC => Kind::DrainMagic,
        SpellId::RESTORE_HEALTH => Kind::RestoreHealth,
        SpellId::RESTORE_MAGIC => Kind::RestoreMagic,
        SpellId::SMELLING_SALTS => Kind::Cure(&[Unconscious, Asleep]),
        _ => return None,
    })
}

/// Whether a spell's effect is implemented; others are refused before
/// anything is spent.
pub fn modelled(spell: SpellId) -> bool {
    kind(spell).is_some()
}

/// Whether a spell would do anything to a victim (retail
/// `MonsterSpellTargetOK`): healing only the hurt, rest only the tired,
/// cures only what they cure, and no condition or enchantment it already
/// has. Pandemonium still works on the frightened while they are sane.
pub fn worthwhile(spell: SpellId, victim: &Vitals) -> bool {
    use Condition::*;
    let has = |condition| victim.conditions.has(condition);
    match kind(spell) {
        Some(Kind::Heal) => victim.health.current < victim.health.max,
        Some(Kind::Stamina) => victim.stamina.current < victim.stamina.max,
        Some(Kind::Inflict(condition, _)) => !has(condition),
        Some(Kind::Fear { .. }) => !has(Afraid),
        Some(Kind::Pandemonium) => !has(Afraid) || !has(Insane),
        Some(Kind::Enchant) => !victim.enchantments.has(spell),
        Some(Kind::Cure(conditions)) => conditions.iter().any(|&c| has(c)),
        _ => true,
    }
}

/// Whether a spell restores what is missing, so that "auto" power and
/// repeating aim at what the victim lacks.
pub(crate) fn restores(spell: SpellId) -> bool {
    matches!(kind(spell), Some(Kind::Heal | Kind::Stamina | Kind::Restoration))
}

/// The monster kinds Dispel Undead harms.
const UNDEAD_KINDS: [MonsterKind; 2] = [MonsterKind::Undead, MonsterKind::Demon];

/// The kind only Banish harms besides them.
const BANISHED_KIND: MonsterKind = MonsterKind::Spirit;

/// What Restoration restores health and stamina by, whatever its record says.
const RESTORATION_DICE: Dice = Dice { base: 0, count: 8, sides: 8 };

/// The share of Lifesteal's damage that heals the most wounded party member,
/// in percent.
const LIFESTEAL_SHARE: u32 = 70;

/// Prismic spells' conditions and their weights (retail `ME:251-253`).
const AFFLICTIONS: [(Option<Condition>, u32); 9] = [
    (Some(Condition::Nauseated), 10),
    (Some(Condition::Afraid), 10),
    (Some(Condition::Hexed), 5),
    (Some(Condition::Insane), 5),
    (Some(Condition::Blind), 5),
    (Some(Condition::Turncoat), 1),
    (Some(Condition::Asleep), 10),
    (Some(Condition::Paralyzed), 3),
    (None, 1),
];

/// The magnitude a spell rolls at `power` (retail `RollEffectMagnitude`): its
/// dice count times power, then raised by the boost percent.
pub(crate) fn magnitude(rng: &mut GameRng, dice: Option<Dice>, strength: Strength) -> u32 {
    let Some(dice) = dice else { return 0 };
    let count = dice.count * strength.power as i32;
    let rolled = rng.roll(Dice { count, ..dice });
    rolled + rolled * strength.boost / 100
}

/// Rounds a condition spell inflicts (retail `RollEffectDuration`).
pub(crate) fn duration(rng: &mut GameRng, spell: &SpellDefinition, strength: Strength) -> u32 {
    let base = spell.duration_per_power * strength.power as i32 + spell.duration_base;
    if base == 9999 {
        return 9999;
    }
    let mut rounds = (base + 1).max(0) as u32;
    match rng.below(4) {
        0 if rounds > 1 => rounds += 1,
        1 if rounds <= 2 => rounds += 1,
        _ => {}
    }
    rounds + rounds * strength.boost / 100
}

/// A spell landing on one body.
#[derive(Component, Clone, Copy, Debug)]
pub struct SpellHit {
    pub spell: SpellId,
    pub strength: Strength,
    /// It turned on its caster's side; a personal spell then does harm.
    pub backfired: bool,
    /// The first party member the deed lands on, who answers for the party:
    /// its shared spells, purse and pool.
    pub party_share: bool,
}

/// A deed of Lifesteal or Might to Magic: what its hits drained, to give to
/// its caster's side once they have all landed.
#[derive(Component, Debug)]
pub struct Drains {
    /// A monster caster heals itself; a member's drain goes to the party.
    pub caster: Entity,
    pub lifesteal: bool,
    pub drained: u32,
}

/// Land a spell on `victims`, a hit on each, as part of `deed`.
pub fn land(
    commands: &mut Commands,
    deed: Entity,
    (spell, strength, backfired): (SpellId, Strength, bool),
    victims: &[Entity],
    party: &[Entity],
) {
    let mut shared = false;
    for &victim in victims {
        let party_share = !shared && party.contains(&victim);
        shared |= party_share;
        commands.spawn((
            SpellHit { spell, strength, backfired, party_share },
            Strikes(victim),
            HitOf(deed),
        ));
    }
}

/// What a spell turned on its caster's side does instead: Dracon Breath
/// nauseates, Haste slows, Razor Cloak harms; other personal spells nothing.
fn backfire(spell: SpellId, kind: Kind) -> Option<Kind> {
    if !crate::game::magic::enchantments::personal(spell) {
        return Some(kind);
    }
    match spell {
        SpellId::DRACON_BREATH => Some(Kind::Inflict(Condition::Nauseated, 100)),
        SpellId::HASTE => Some(Kind::Inflict(Condition::Slowed, 100)),
        SpellId::RAZOR_CLOAK => Some(Kind::Damage),
        _ => None,
    }
}

/// Damage a living body by an already rolled magnitude, less resistance.
fn harm(
    rng: &mut GameRng,
    body: &mut BodyItem,
    defence: &Defence,
    (rolled, against): (u32, u32),
    hurting: (&MonsterCatalog, Option<Entity>),
    out: (&mut Commands, &mut MessageWriter<DamageApplied>),
) -> Option<Effect> {
    if body.is_dead() {
        return None;
    }
    match body::resist(rng, rolled, defence.resistance, defence.level, against) {
        0 => Some(Effect::Resisted { victim: body.entity }),
        amount => Some(wound(body, amount, rng, hurting, out)),
    }
}

/// Retail `ResolveAttackOnTarget`: a condition, or death when `condition` is
/// `None`, unless the body's kind is immune, it saves, or shrugs it off by
/// its nature; then resistance cuts its duration, and nothing lands from
/// none (retail `InflictConditionOnTarget`). `rounds` of `None` lasts until
/// lifted.
#[allow(clippy::too_many_arguments)]
pub(crate) fn afflict(
    rng: &mut GameRng,
    body: &mut BodyItem,
    defence: &Defence,
    (condition, poison): (Option<Condition>, u32),
    (rounds, against): (Option<u32>, u32),
    (species, difficulty, fight): (&MonsterCatalog, Difficulty, Option<Entity>),
    improved: &mut Vec<(Entity, crate::content::characters::Skill)>,
    out: (&mut Commands, &mut MessageWriter<DamageApplied>),
) -> Option<Effect> {
    if body.is_dead() {
        return None;
    }
    let victim = body.entity;
    let immune = match condition {
        Some(condition) => body.immune(condition, species),
        None => body.death_immune(species),
    };
    if immune {
        return Some(Effect::Resisted { victim });
    }
    let base = condition.map_or(body::DEATH_SAVE, body::condition_save);
    let (saved, practised) = body.saves(rng, defence, (base, against), difficulty);
    improved.extend(practised.map(|skill| (victim, skill)));
    if saved || condition.is_some_and(|c| body.shrugs(c, rounds.is_none())) {
        return Some(Effect::Resisted { victim });
    }
    let lasting = rounds.unwrap_or(INDEFINITE);
    let lasting = match body::resist(rng, lasting, defence.resistance, defence.level, against) {
        0 => return Some(Effect::Resisted { victim }),
        _ if rounds.is_none() => Lasting::Indefinite,
        1 => Lasting::Rounds(2),
        rounds => Lasting::Rounds(rounds),
    };
    let Some(condition) = condition else {
        let lost = body.slay(rng, (species, fight));
        return Some(slain(body, lost, species, out));
    };
    body.inflict((condition, poison), lasting, defence.side)
        .then_some(Effect::Inflicted { victim, condition })
}

/// What slaying a body did: a member's notice, a monster's kill.
fn slain(
    body: &mut BodyItem,
    lost: u32,
    species: &MonsterCatalog,
    (_, notices): (&mut Commands, &mut MessageWriter<DamageApplied>),
) -> Effect {
    let died = body.is_dead();
    if body.character.is_some() && lost > 0 {
        notices.write(DamageApplied { member: body.entity, amount: lost, died });
    }
    let experience = match &body.monster {
        Some(monster) if died && !body.summoned => species[monster.species].experience,
        _ => 0,
    };
    Effect::Damaged {
        victim: body.entity,
        outcome: crate::game::world::damage::DamageOutcome { damage: lost, died, experience },
    }
}

/// Restore a living body's health or stamina by rolled `dice`, up to the
/// maximum: the amount, or `None` (nothing rolled) when it lacks none.
fn restore(
    rng: &mut GameRng,
    body: &mut BodyItem,
    health: bool,
    dice: Option<Dice>,
    strength: Strength,
) -> Option<u32> {
    if body.is_dead() {
        return None;
    }
    let pool = if health { body.vitals.health } else { body.vitals.stamina };
    let missing = pool.max - pool.current;
    if missing == 0 {
        return None;
    }
    let amount = magnitude(rng, dice, strength).min(missing);
    if health {
        body.vitals.heal(amount);
    } else {
        body.vitals.rest(amount);
    }
    Some(amount)
}

/// Consume physical units, not charges, in place; remaining identity and
/// equipment binding metadata belong to the existing owner.
fn burn_stack(
    stack: crate::game::inventory::ItemStack,
    items: &ItemCatalog,
    chance: u32,
    rng: &mut GameRng,
    totals: &mut [u32; 3],
) -> Option<crate::game::inventory::ItemStack> {
    let category = match items[stack.item].class {
        EquipClass::Potion => 0,
        EquipClass::Scroll => 1,
        EquipClass::Spellbook => 2,
        _ => return Some(stack),
    };
    let lost = (0..stack.count()).filter(|_| rng.below(100) < chance).count() as u16;
    totals[category] += u32::from(lost);
    stack.remove_units(lost)
}

/// The party's own spells that a Purify shortens: the offensive slots, by
/// more the stronger Purify is than each cloud.
fn purify(
    slots: &mut Enchantments,
    amount: u32,
    spell: &SpellDefinition,
    catalog: &crate::content::spells::SpellCatalog,
) {
    slots.0.retain_mut(|slot| {
        if crate::game::magic::enchantments::OFFENSIVE.contains(&slot.spell) {
            let cloud = &catalog[slot.spell];
            let removed = (amount + spell.level).max(cloud.level) - cloud.level;
            slot.turns -= removed.min(slot.turns);
        }
        slot.turns > 0
    });
}

/// Land each spell hit on its body: what the spell does there, gathered on
/// its deed. A body that died or left since is missed.
#[allow(clippy::too_many_arguments)]
pub fn land_spells(
    mut commands: Commands,
    hits: Query<(Entity, &SpellHit, &Strikes, &HitOf)>,
    mut bodies: Query<Body>,
    mut deeds: Query<(&mut Deed, Option<&mut Drains>)>,
    (mut party, mut inventory, roster): (
        Single<&mut PartyEffects>,
        Single<&mut PartyInventory>,
        Single<&PartyRoster>,
    ),
    groups: Query<&MonsterGroup>,
    content: If<Res<crate::game::data::Catalogs>>,
    (difficulty, factions, clock): (
        Res<Difficulty>,
        Res<Factions>,
        Res<crate::game::time::GameClock>,
    ),
    fight: Query<Entity, With<crate::game::combat::Battle>>,
    mut rng: ResMut<GameRng>,
    mut notices: MessageWriter<DamageApplied>,
) {
    let (items, species, spells) = (&content.items, &content.species, &content.spells);
    let (difficulty, fight, now) = (*difficulty, fight.single().ok(), clock.ms);
    let rng = &mut *rng;
    for (
        hit,
        &SpellHit { spell, strength, backfired, party_share },
        &Strikes(victim),
        &HitOf(of),
    ) in &hits
    {
        commands.entity(hit).despawn();
        let definition = &spells[spell];
        let (Some(kind), Some(realm)) = (kind(spell), definition.realm) else { continue };
        let Some(kind) = (if backfired { backfire(spell, kind) } else { Some(kind) }) else {
            continue;
        };
        let (mut deed, mut drains) = deeds.get_mut(of).unwrap();
        let deed = &mut *deed;
        // What touches the whole party lands once, through its first member.
        if kind == Kind::DrainMagic {
            if party_share {
                let base = strength.power * 50;
                let amount = base + base * strength.boost / 100;
                // Anti-Magic drains the whole living party, not only its victims.
                for member in roster.iter() {
                    let mut member = bodies.get_mut(member).unwrap();
                    if !member.is_dead() {
                        let amount = member.drain_magic(amount, rng, (spells, species));
                        deed.effects.push(Effect::Drained { victim: member.entity, amount });
                    }
                }
                // Anti-Magic clears the party's lasting slots, not combat or actor spells.
                party.0.0.retain(|e| crate::game::magic::enchantments::combat_only(e.spell));
            }
            continue;
        }
        if kind == Kind::GoldLoss {
            if party_share {
                // Retail's second roll when the first is below 100 is intentional.
                let first = magnitude(rng, definition.dice, strength);
                let percent =
                    if first < 100 { magnitude(rng, definition.dice, strength) } else { 100 };
                let lost = inventory.gold * percent.min(100) / 100;
                inventory.gold -= lost;
                deed.effects.push(Effect::GoldLost { amount: lost });
            }
            continue;
        }
        let Ok(mut body) = bodies.get_mut(victim) else { continue };
        let body = &mut body;
        let member = body.character.is_some();
        let sides = (&groups, &*factions, now);
        let defence = body.defence(realm, (items, species, &party.0), sides);
        let ruling = (species, difficulty, fight);
        let improved = &mut deed.improved;
        let effects = &mut deed.effects;
        let attack =
            |rng: &mut GameRng,
             body: &mut BodyItem,
             condition: Condition,
             chance: u32,
             strength: Strength,
             improved: &mut Vec<_>,
             out: (&mut Commands, &mut MessageWriter<DamageApplied>)| {
                if rng.below(100) >= chance || body.is_dead() {
                    return None;
                }
                let rounds = duration(rng, definition, strength);
                afflict(
                    rng,
                    body,
                    &defence,
                    (Some(condition), 0),
                    (Some(rounds), strength.against),
                    ruling,
                    improved,
                    out,
                )
            };
        let hurting = (species, fight);
        match kind {
            Kind::Damage => {
                let rolled = magnitude(rng, definition.dice, strength);
                effects.extend(harm(
                    rng,
                    body,
                    &defence,
                    (rolled, strength.against),
                    hurting,
                    (&mut commands, &mut notices),
                ));
            }
            Kind::DamageUndead { banish } => {
                let Some(monster) = &body.monster else { continue };
                let kind = species[monster.species].kind;
                if !(UNDEAD_KINDS.contains(&kind) || (banish && kind == BANISHED_KIND)) {
                    effects.push(Effect::Nothing { victim });
                    continue;
                }
                let rolled = magnitude(rng, definition.dice, strength);
                effects.extend(harm(
                    rng,
                    body,
                    &defence,
                    (rolled, strength.against),
                    hurting,
                    (&mut commands, &mut notices),
                ));
            }
            Kind::Heal | Kind::Stamina | Kind::Restoration => {
                if body.is_dead() {
                    continue;
                }
                let dice = if kind == Kind::Restoration {
                    Some(RESTORATION_DICE)
                } else {
                    definition.dice
                };
                let mut effect = Effect::Nothing { victim };
                if kind != Kind::Stamina
                    && let Some(amount) = restore(rng, body, true, dice, strength)
                {
                    effect = Effect::Healed { victim, amount };
                }
                if kind != Kind::Heal
                    && let Some(amount) = restore(rng, body, false, dice, strength)
                    && kind == Kind::Stamina
                {
                    effect = Effect::Refreshed { victim, amount };
                }
                if kind == Kind::Restoration {
                    use Condition::*;
                    let all = [
                        Diseased,
                        Irritated,
                        Nauseated,
                        Slowed,
                        Afraid,
                        Poisoned,
                        Silenced,
                        Hexed,
                        Insane,
                        Blind,
                        Turncoat,
                        Webbed,
                        Asleep,
                        Paralyzed,
                        Unconscious,
                    ];
                    let lifted = body::cure(rng, &mut body.vitals.conditions, &all, strength, true);
                    if matches!(effect, Effect::Nothing { .. })
                        && let Some(&condition) = lifted.first()
                    {
                        effect = Effect::Cured { victim, condition };
                    }
                }
                effects.push(effect);
            }
            Kind::RestoreHealth => {
                if let Some(amount) = restore(rng, body, true, definition.dice, strength) {
                    effects.push(Effect::Healed { victim, amount });
                }
                let dice = definition.dice.map(|dice| Dice { count: dice.count * 2, ..dice });
                if let Some(amount) = restore(rng, body, false, dice, strength) {
                    effects.push(Effect::Refreshed { victim, amount });
                }
            }
            Kind::RestoreMagic => {
                if body.is_dead() || body.magic.is_none() {
                    continue;
                }
                // Retail rolls even when every realm is already full.
                let amount = magnitude(rng, definition.dice, strength);
                let (_, pool) = body.magic.as_mut().unwrap();
                let amount = pool.restore_evenly(amount);
                effects.push(if amount == 0 {
                    Effect::Nothing { victim }
                } else {
                    Effect::Replenished { victim, amount }
                });
            }
            Kind::Inflict(condition, chance) => {
                effects.extend(attack(
                    rng,
                    body,
                    condition,
                    chance,
                    strength,
                    improved,
                    (&mut commands, &mut notices),
                ));
            }
            Kind::DamageThen(..) | Kind::NoxiousFumes => {
                let rolled = magnitude(rng, definition.dice, strength);
                effects.extend(harm(
                    rng,
                    body,
                    &defence,
                    (rolled, strength.against),
                    hurting,
                    (&mut commands, &mut notices),
                ));
                let attacks = match kind {
                    Kind::DamageThen(condition, chance) => vec![(condition, chance)],
                    _ => vec![(Condition::Nauseated, 90), (Condition::Unconscious, 25)],
                };
                for (condition, chance) in attacks {
                    effects.extend(attack(
                        rng,
                        body,
                        condition,
                        chance,
                        strength,
                        improved,
                        (&mut commands, &mut notices),
                    ));
                }
            }
            Kind::Fear { second } => {
                let landed = attack(
                    rng,
                    body,
                    Condition::Afraid,
                    100,
                    strength,
                    improved,
                    (&mut commands, &mut notices),
                );
                // A member may be knocked out too, a monster frightened again.
                let again = match second {
                    Condition::Unconscious => member,
                    Condition::Afraid => !member,
                    _ => false,
                };
                if matches!(landed, Some(Effect::Inflicted { .. })) && again {
                    let half = Strength { against: strength.against / 2, ..strength };
                    attack(rng, body, second, 100, half, improved, (&mut commands, &mut notices));
                }
                effects.extend(landed);
            }
            Kind::Pandemonium => {
                let landed = attack(
                    rng,
                    body,
                    Condition::Insane,
                    100,
                    strength,
                    improved,
                    (&mut commands, &mut notices),
                );
                // One who keeps their mind is afraid anyway.
                if !matches!(landed, Some(Effect::Resisted { .. })) {
                    effects.extend(landed);
                    continue;
                }
                let rounds = duration(rng, definition, strength);
                let rounds =
                    body::resist(rng, rounds, defence.resistance, defence.level, strength.against);
                if rounds == 0 || body.immune(Condition::Afraid, species) {
                    effects.push(Effect::Resisted { victim });
                    continue;
                }
                let lasting = Lasting::Rounds(rounds.max(2));
                if body.inflict((Condition::Afraid, 0), lasting, defence.side) {
                    effects.push(Effect::Inflicted { victim, condition: Condition::Afraid });
                }
            }
            Kind::Slay => {
                effects.extend(slay(
                    rng,
                    body,
                    &defence,
                    (definition, strength),
                    ruling,
                    improved,
                    (&mut commands, &mut notices),
                ));
            }
            Kind::Affliction => {
                let roll = rng.below(100);
                if roll >= 50 {
                    let rolled = magnitude(rng, definition.dice, strength);
                    effects.extend(harm(
                        rng,
                        body,
                        &defence,
                        (rolled, strength.against),
                        hurting,
                        (&mut commands, &mut notices),
                    ));
                    continue;
                }
                let pick = crate::game::rng::weighted(roll, AFFLICTIONS.iter().map(|&(_, w)| w))
                    .and_then(|index| AFFLICTIONS[index].0);
                effects.extend(match pick {
                    Some(condition) => attack(
                        rng,
                        body,
                        condition,
                        100,
                        strength,
                        improved,
                        (&mut commands, &mut notices),
                    ),
                    None => slay(
                        rng,
                        body,
                        &defence,
                        (definition, strength),
                        ruling,
                        improved,
                        (&mut commands, &mut notices),
                    ),
                });
            }
            Kind::Cure(conditions) => {
                // Taking rounds off a member's poison weakens it as aging does.
                if member && conditions.contains(&Condition::Poisoned) {
                    body.vitals.weaken_poison(body::cure_rounds(strength), rng);
                }
                let lifted =
                    body::cure(rng, &mut body.vitals.conditions, conditions, strength, false);
                effects.push(match lifted.first() {
                    Some(&condition) => Effect::Cured { victim, condition },
                    None => Effect::Nothing { victim },
                });
            }
            Kind::Renewal => {
                let lifted = match body.character.as_mut() {
                    Some(character) => body.vitals.lift_drain(character),
                    None => !body::cure(
                        rng,
                        &mut body.vitals.conditions,
                        &[Condition::Drained],
                        strength,
                        true,
                    )
                    .is_empty(),
                };
                effects.push(if lifted {
                    Effect::Cured { victim, condition: Condition::Drained }
                } else {
                    Effect::Nothing { victim }
                });
            }
            Kind::Vortex => {
                if !member {
                    let rolled = magnitude(rng, definition.dice, strength);
                    effects.extend(harm(
                        rng,
                        body,
                        &defence,
                        (rolled, strength.against),
                        hurting,
                        (&mut commands, &mut notices),
                    ));
                    continue;
                }
                // Members take the plain roll, without the cast's boost, for good.
                let rolled = magnitude(rng, definition.dice, Strength { boost: 0, ..strength });
                if body.is_dead() {
                    continue;
                }
                let amount =
                    body::resist(rng, rolled, defence.resistance, defence.level, strength.against);
                if amount == 0 {
                    effects.push(Effect::Resisted { victim });
                    continue;
                }
                let lost = body.sap(amount, rng, fight).unwrap();
                effects.push(slain(body, lost, species, (&mut commands, &mut notices)));
            }
            Kind::Resurrect => {
                let Some(character) = &body.character else { continue };
                if !body.is_dead() {
                    effects.push(Effect::Nothing { victim });
                    continue;
                }
                // Retail lifts Dead with health and stamina at
                // `max·((PIE + VIT) >> 2)/100`, at least 1.
                let stats = character.stats(&body.vitals.enchantments);
                let share =
                    (stats.attribute(Attribute::Piety) + stats.attribute(Attribute::Vitality)) >> 2;
                let vitals = &mut body.vitals;
                vitals.health.current = (vitals.health.max * share / 100).max(1);
                vitals.stamina.current = (vitals.stamina.max * share / 100).max(1);
                effects.push(Effect::Raised { victim });
            }
            Kind::Lifesteal | Kind::MightToMagic => {
                if body.is_dead() {
                    continue;
                }
                let rolled = magnitude(rng, definition.dice, strength);
                let effect = harm(
                    rng,
                    body,
                    &defence,
                    (rolled, strength.against),
                    hurting,
                    (&mut commands, &mut notices),
                );
                if let (Some(Effect::Damaged { outcome, .. }), Some(drains)) =
                    (effect, drains.as_mut())
                {
                    drains.drained += outcome.damage;
                }
                effects.extend(effect);
            }
            Kind::Purify => {
                // Purify rolls from power alone, rather than its authored duration.
                let base = strength.power + 1;
                let amount = match rng.below(4) {
                    0 if base > 1 => base + 1,
                    1 if base < 3 => base + 1,
                    _ => base,
                };
                let amount = amount + amount * strength.boost / 100;
                // The party's clouds hang over all its members at once.
                if !member {
                    purify(&mut body.vitals.enchantments, amount, definition, spells);
                } else if party_share {
                    purify(&mut party.0, amount, definition, spells);
                } else {
                    continue;
                }
                effects.push(Effect::Purified { victim });
            }
            Kind::Enchant => {
                let personal = crate::game::magic::enchantments::personal(spell);
                let turns = duration(rng, definition, strength);
                let rolled = Enchantment {
                    spell,
                    power: strength.power,
                    boost: strength.boost,
                    turns,
                    absorption: if spell == SpellId::GUARDIAN_ANGEL {
                        let rolled =
                            definition.dice.map_or(0, |dice| rng.roll(dice)) * strength.power;
                        rolled + rolled * strength.boost / 100
                    } else {
                        0
                    },
                };
                let changed = if member && !personal {
                    // The party's own slots hold for all its members at once.
                    if !party_share {
                        continue;
                    }
                    party.0.accepts(spell, strength.power) && party.0.apply(rolled)
                } else if body.is_dead() {
                    false
                } else if spell == SpellId::HASTE {
                    let vitals = &mut *body.vitals;
                    crate::game::magic::enchantments::haste(
                        &mut vitals.conditions,
                        &mut vitals.enchantments,
                        rolled,
                        !member,
                    )
                } else if member {
                    body.vitals.enchantments.accepts(spell, strength.power)
                        && body.vitals.enchantments.apply(rolled)
                } else {
                    body.vitals.enchantments.monster_accepts(spell, strength.power)
                        && body.vitals.enchantments.apply_to_monster(rolled)
                };
                // A member's maxima follow the spells on them at once.
                if let Some(character) = &body.character {
                    body.vitals.sync_maxima(character);
                }
                effects.push(if changed {
                    Effect::Enchanted { victim, spell }
                } else {
                    Effect::Nothing { victim }
                });
            }
            Kind::DestroyConsumables => {
                let chance = strength.power * 10;
                let mut totals = [0; 3];
                if let Some(carried) = body.carried.as_mut() {
                    // Retail visits all twelve equipped slots, including alternate hands.
                    for (_, slot) in &mut carried.equipment {
                        if let Some(equipped) = slot.take() {
                            *slot = burn_stack(equipped.stack, items, chance, rng, &mut totals)
                                .map(|stack| crate::game::inventory::Equipped {
                                    stack,
                                    curse_released: equipped.curse_released,
                                });
                        }
                    }
                    for slot in &mut carried.backpack {
                        if let Some(stack) = slot.take() {
                            *slot = burn_stack(stack, items, chance, rng, &mut totals);
                        }
                    }
                }
                if party_share {
                    let pool = &mut inventory.pool.stacks;
                    *pool = std::mem::take(pool)
                        .into_iter()
                        .filter_map(|stack| burn_stack(stack, items, chance, rng, &mut totals))
                        .collect();
                }
                for (class, amount) in
                    [EquipClass::Potion, EquipClass::Scroll, EquipClass::Spellbook]
                        .into_iter()
                        .zip(totals)
                {
                    if amount != 0 {
                        effects.push(Effect::Destroyed { class, amount });
                    }
                }
            }
            Kind::DrainMagic | Kind::GoldLoss => unreachable!("landed for the party above"),
        }
    }
}

/// Death spells: a failed save kills, unless resistance cuts the attempt's
/// duration to nothing (retail `InflictConditionOnTarget` for Dead).
fn slay(
    rng: &mut GameRng,
    body: &mut BodyItem,
    defence: &Defence,
    (spell, strength): (&SpellDefinition, Strength),
    (species, difficulty, fight): (&MonsterCatalog, Difficulty, Option<Entity>),
    improved: &mut Vec<(Entity, crate::content::characters::Skill)>,
    out: (&mut Commands, &mut MessageWriter<DamageApplied>),
) -> Option<Effect> {
    if body.is_dead() {
        return None;
    }
    let rounds = duration(rng, spell, strength);
    let (saved, practised) =
        body.saves(rng, defence, (body::DEATH_SAVE, strength.against), difficulty);
    improved.extend(practised.map(|skill| (body.entity, skill)));
    if saved || body::resist(rng, rounds, defence.resistance, defence.level, strength.against) == 0
    {
        return Some(Effect::Resisted { victim: body.entity });
    }
    let lost = body.slay(rng, (species, fight));
    Some(slain(body, lost, species, out))
}

/// Once a Lifesteal or Might to Magic deed's hits have landed: a monster
/// caster takes 70% of the drained health back, or half of it as spell
/// points; a member's drain goes, once, to the party member most in need
/// (retail chooses after every victim has been drained).
pub fn drain_life(
    mut deeds: Query<(&mut Deed, &Drains), Without<DeedHits>>,
    mut bodies: Query<Body>,
    roster: Single<&PartyRoster>,
    content: If<Res<crate::game::data::Catalogs>>,
) {
    for (mut deed, drains) in &mut deeds {
        let Ok(mut caster) = bodies.get_mut(drains.caster) else { continue };
        if let Some(monster) = caster.monster.as_mut() {
            if drains.lifesteal {
                caster.vitals.heal(drains.drained * 7 / 10);
            } else {
                monster.spell_points += (drains.drained / 2) as i32;
            }
            continue;
        }
        let need = |body: &BodyItem| -> Option<u32> {
            if body.is_dead() {
                return None;
            }
            let percent = if drains.lifesteal {
                body.vitals.health.current * 100 / body.vitals.health.max
            } else {
                let maxima = body.mana_maximum(&content.spells)?;
                let maximum: u32 = maxima.values().sum();
                if maximum == 0 {
                    return None;
                }
                let (_, pool) = body.magic.as_ref()?;
                maxima.iter().map(|(realm, _)| pool.available(realm, &maxima)).sum::<u32>() * 100
                    / maximum
            };
            (percent < 100).then_some(percent)
        };
        let neediest = roster
            .iter()
            .filter_map(|member| need(&bodies.get_mut(member).ok()?).map(|p| (member, p)))
            .min_by_key(|&(_, percent)| percent)
            .map(|(member, _)| member);
        let Some(member) = neediest else { continue };
        let mut body = bodies.get_mut(member).unwrap();
        if drains.lifesteal {
            let amount = body.vitals.heal(drains.drained * LIFESTEAL_SHARE / 100);
            deed.effects.push(Effect::Healed { victim: member, amount });
        } else {
            let (_, pool) = body.magic.as_mut().unwrap();
            let amount = pool.restore_evenly(drains.drained);
            deed.effects.push(Effect::Replenished { victim: member, amount });
        }
    }
}

/// Boiling Blood's last missile: once its deed's hits have landed, each of
/// its victims that died bursts where it fell, at the cast's power.
#[derive(Component, Debug)]
pub struct Bursts {
    pub caster: Entity,
    pub victims: Vec<Entity>,
    pub strength: Strength,
    /// What the burst's strength grows from: its source's level.
    pub figure: u32,
    pub recipients: super::reach::Recipients,
}

/// Burst the dead of each landed Boiling Blood (retail's Boiling Blood
/// explosion): a deed of its own, landing on whoever stands within reach.
#[allow(clippy::too_many_arguments)]
pub fn boil_blood(
    mut commands: Commands,
    deeds: Query<&Bursts, Without<DeedHits>>,
    mut bodies: Query<Body>,
    monsters: Query<
        (Entity, crate::game::world::ActorPose, &crate::game::world::level::LevelOwned),
        With<crate::game::world::population::Monster>,
    >,
    walkers: Query<&crate::game::world::navigation::Walker>,
    levels: Query<(), With<crate::game::world::level::Level>>,
    party: Single<
        (&avian3d::prelude::Position, &PartyRoster),
        With<crate::game::party::body::PartyBody>,
    >,
    groups: Query<&MonsterGroup>,
    content: If<Res<crate::game::data::Catalogs>>,
    (factions, clock): (Res<Factions>, Res<crate::game::time::GameClock>),
    obstacles: crate::game::world::obstacles::WorldObstacles,
) {
    let explosion = &content.spells[SpellId::BOILING_BLOOD_EXPLOSION];
    let (position, roster) = *party;
    let eye = crate::game::party::body::PartyBody::eye(position);
    for bursts in &deeds {
        for &victim in &bursts.victims {
            if !bodies.get_mut(victim).is_ok_and(|body| body.is_dead()) {
                continue;
            }
            let centre = match monsters.get(victim) {
                Ok((_, at, _)) => {
                    crate::game::world::navigation::feet(at.translation(), walkers.get(victim).ok())
                }
                Err(_) => crate::game::party::body::PartyBody::feet_below(eye),
            };
            let strength = Strength::at(
                bursts.figure,
                explosion,
                bursts.strength.power,
                bursts.strength.boost,
            );
            let reach = explosion.radius + explosion.radius_per_power * strength.power as f32;
            let victims = super::reach::radius(
                (centre, centre, reach),
                bursts.recipients,
                Some((eye, roster)),
                &mut bodies,
                &monsters,
                &levels,
                (&content.species, (&groups, &*factions, clock.ms)),
                &obstacles,
            );
            let deed = commands
                .spawn(Deed::new(super::Act::Missile {
                    caster: bursts.caster,
                    spell: SpellId::BOILING_BLOOD_EXPLOSION,
                }))
                .id();
            let party: Vec<Entity> = roster.iter().collect();
            land(
                &mut commands,
                deed,
                (SpellId::BOILING_BLOOD_EXPLOSION, strength, false),
                &victims,
                &party,
            );
        }
    }
}
