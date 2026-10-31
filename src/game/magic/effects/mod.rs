//! What spells do to their victims (retail `ProcessSpellEffectTargets`,
//! `ResolveSpellMissileHit` and the effect functions of `Magic Effects.cpp`),
//! table-driven by spell. Each spell resolves in retail's passes: every
//! victim's damage, then every victim's conditions.

use super::{LearnedSpells, SpellPoints, Strength};
use crate::content::characters::{Attribute, Realm, Skill};
use crate::content::items::{Dice, EquipClass, ItemCatalog};
use crate::content::species::{MonsterCatalog, MonsterKind, SpecialAttackKind};
use crate::content::spells::{SpellDefinition, SpellId};
use crate::game::characters::{Condition, Conditions, Fate, Lasting};
use crate::game::difficulty::Difficulty;
use crate::game::inventory::PartyInventory;
use crate::game::party::DamageApplied;
use crate::game::party::Members;
use crate::game::party::formation::Formation;
use crate::game::rng::GameRng;
use crate::game::world::damage::{DamageOutcome, MonsterDamage};
use crate::game::world::hostility::Stance;
use bevy::prelude::*;

mod harm;
mod remedy;
mod special;
pub use harm::*;
pub use remedy::*;
pub use special::*;

/// What a spell does, by retail's effect functions.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
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
fn kind(spell: SpellId) -> Option<Kind> {
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

/// What a would-be victim lacks or already suffers, as a caster weighs a
/// spell on it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Standing {
    pub hurt: bool,
    pub tired: bool,
    pub conditions: Conditions,
    pub enchantments: super::enchantments::Enchantments,
}

/// Whether a spell would do anything to a victim (retail
/// `MonsterSpellTargetOK`): healing only the hurt, rest only the tired,
/// cures only what they cure, and no condition or enchantment it already
/// has. Pandemonium still works on the frightened while they are sane.
pub fn worthwhile(spell: SpellId, standing: &Standing) -> bool {
    use Condition::*;
    let has = |condition| standing.conditions.has(condition);
    match kind(spell) {
        Some(Kind::Heal) => standing.hurt,
        Some(Kind::Stamina) => standing.tired,
        Some(Kind::Inflict(condition, _)) => !has(condition),
        Some(Kind::Fear { .. }) => !has(Afraid),
        Some(Kind::Pandemonium) => !has(Afraid) || !has(Insane),
        Some(Kind::Enchant) => !standing.enchantments.has(spell),
        Some(Kind::Cure(conditions)) => conditions.iter().any(|&c| has(c)),
        _ => true,
    }
}

/// Whom a spell lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Victim {
    Member(Entity),
    Monster(Entity),
}

impl Victim {
    pub fn entity(self) -> Entity {
        match self {
            Victim::Member(entity) | Victim::Monster(entity) => entity,
        }
    }
}

/// What a spell did to one victim.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    Purified {
        victim: Victim,
    },
    Enchanted {
        victim: Victim,
        spell: SpellId,
    },
    Healed {
        victim: Victim,
        amount: u32,
    },
    Refreshed {
        victim: Victim,
        amount: u32,
    },
    Replenished {
        victim: Victim,
        amount: u32,
    },
    Damaged {
        victim: Victim,
        outcome: DamageOutcome,
    },
    Inflicted {
        victim: Victim,
        condition: Condition,
    },
    Cured {
        victim: Victim,
        condition: Condition,
    },
    Raised {
        victim: Victim,
    },
    GoldLost {
        amount: u32,
    },
    Destroyed {
        class: EquipClass,
        amount: u32,
    },
    Drained {
        victim: Victim,
        amount: u32,
    },
    /// Stamina a blow drained.
    Tired {
        victim: Victim,
        amount: u32,
    },
    Resisted {
        victim: Victim,
    },
    Reflected {
        victim: Victim,
    },
    Nothing {
        victim: Victim,
    },
}

/// What resolving an action needs and changes: the members and monsters
/// it acts on, the party's shared effects, items, purse and formation, the
/// rule catalogs, the played level and the fight under way.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Field<'w, 's> {
    pub members: Members<'w, 's>,
    /// The members' spells and points.
    pub points: Query<'w, 's, (&'static LearnedSpells, &'static mut SpellPoints)>,
    pub monsters: MonsterDamage<'w, 's>,
    /// The current roster in order, including dead members eligible for revival.
    pub party: Single<'w, 's, &'static crate::game::party::roster::PartyRoster>,
    content: If<Res<'w, crate::game::data::Catalogs>>,
    difficulty: Res<'w, Difficulty>,
    pub enchantments: Single<'w, 's, &'static mut super::enchantments::PartyEffects>,
    /// The party's pool and purse.
    pub inventory: Single<'w, 's, &'static mut PartyInventory>,
    pub formation: Single<'w, 's, &'static mut Formation>,
    pub party_damage: MessageWriter<'w, DamageApplied>,
    pub news: MessageWriter<'w, super::casting::MagicNews>,
    pub visibility: crate::game::world::obstacles::WorldObstacles<'w, 's>,
    /// Missiles leave in launch order, so equal arrivals land in it too.
    pub flight_order: ResMut<'w, super::casting::FlightOrder>,
    /// Skills that improved while resolving an action, for its notices.
    pub improved: Local<'s, Vec<(Entity, Skill)>>,
    level: Single<
        'w,
        's,
        (Entity, Option<&'static crate::game::world::triggers::LevelState>),
        With<crate::game::world::level::Level>,
    >,
    fights: Query<'w, 's, Entity, With<crate::game::combat::Battle>>,
    provocations: MessageWriter<'w, crate::game::world::hostility::Provocation>,
    clock: Res<'w, crate::game::time::GameClock>,
}

impl<'w> Field<'w, '_> {
    /// The rule catalogs, held for the whole run apart from the field's
    /// other borrows.
    pub fn catalogs(&self) -> &'w crate::game::data::Catalogs {
        Res::into_inner(Res::clone(&self.content.0))
    }

    pub fn items(&self) -> &'w ItemCatalog {
        &self.catalogs().items
    }

    pub fn species(&self) -> &'w MonsterCatalog {
        &self.catalogs().species
    }

    pub fn spells(&self) -> &'w crate::content::spells::SpellCatalog {
        &self.catalogs().spells
    }

    pub fn difficulty(&self) -> Difficulty {
        *self.difficulty
    }

    /// The level being played, which spell missiles belong to.
    pub fn level(&self) -> Entity {
        self.level.0
    }

    /// Whether the level is under water, where fire fails.
    pub fn underwater(&self) -> bool {
        self.level.1.is_some_and(|state| state.camera_sway)
    }

    /// The game time the action happens at, which a kill's offence is
    /// dated by.
    pub fn now(&self) -> u64 {
        self.clock.ms
    }

    /// The fight under way.
    pub fn fight(&self) -> Option<Entity> {
        self.fights.single().ok()
    }

    /// What monster loot tables scale by: the party's average level.
    pub fn party_level(&self) -> u32 {
        crate::game::party::average_level(
            self.members.iter_many(self.party.iter()).matched().map(|(c, ..)| c.level()),
        )
    }

    /// The party acts against this monster. Whether that provokes its group
    /// is settled now, as the action is accepted, so nothing the action then
    /// does (a kill, a Turncoat) undoes it (retail `MakeTargetGroupHostile`):
    /// a living monster not under Turncoat does.
    pub fn provoke(&mut self, monster: Entity) {
        if self.monsters.provoke(monster, self.clock.ms) {
            self.provocations.write(crate::game::world::hostility::Provocation(monster));
        }
    }
}

impl Field<'_, '_> {
    /// The member's spell point maximum in each realm.
    pub fn mana_maximum(&self, member: Entity) -> enum_map::EnumMap<Realm, u32> {
        let (profile, vitals, ..) = self.members.get(member).unwrap();
        let (learned, _) = self.points.get(member).unwrap();
        profile.stats(&vitals.enchantments).max_points(learned, self.spells())
    }
}

/// The magnitude a spell rolls at `power` (retail `RollEffectMagnitude`): its
/// dice count times power, then raised by the boost percent.
pub(crate) fn magnitude(rng: &mut GameRng, dice: Option<Dice>, strength: Strength) -> u32 {
    let Some(dice) = dice else { return 0 };
    let count = (u32::from(dice.count) * u32::from(strength.power)).min(255) as u8;
    let rolled = rng.roll(Dice { count, ..dice });
    rolled + rolled * strength.boost / 100
}

/// Retail `ReduceMagnitudeByResistance`: resistance and the victim's level
/// over the cast's strength take a random share off.
pub(super) fn resist(
    rng: &mut GameRng,
    amount: u32,
    resistance: i32,
    level: u32,
    against: u32,
) -> u32 {
    let mut c = resistance + (level as i32 - against as i32) * 3;
    if c <= 0 {
        return amount;
    }
    c += 1;
    let half = c / 2;
    let share =
        (rng.below((c - half) as u32) as i32 + half + rng.below(half as u32) as i32).min(100);
    amount - amount * share as u32 / 100
}

/// Rounds a condition spell inflicts (retail `RollEffectDuration`).
pub(super) fn duration(rng: &mut GameRng, spell: &SpellDefinition, strength: Strength) -> u32 {
    let base = spell.duration_per_power * i32::from(strength.power) + spell.duration_base;
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

/// Resolve a spell on the victims captured when it was cast, in their
/// captured order. Victims that died or left since are skipped; nobody new
/// is added.
pub fn resolve(
    origin: Option<Victim>,
    backfired: bool,
    spell: &SpellDefinition,
    strength: Strength,
    victims: &[Victim],
    rng: &mut GameRng,
    world: &mut Field,
) -> Vec<Effect> {
    if victims.is_empty() {
        return Vec::new();
    }
    let (Some(kind), Some(realm)) = (kind(spell.id), spell.realm) else { return Vec::new() };
    let kind = if backfired && super::enchantments::personal(spell.id) {
        if spell.id == SpellId::DRACON_BREATH {
            Kind::Inflict(Condition::Nauseated, 100)
        } else if spell.id == SpellId::HASTE {
            Kind::Inflict(Condition::Slowed, 100)
        } else if spell.id == SpellId::RAZOR_CLOAK {
            Kind::Damage
        } else {
            return Vec::new();
        }
    } else {
        kind
    };
    let mut effects = Vec::new();
    let mut each = |f: &mut dyn FnMut(Victim, &mut GameRng, &mut Field) -> Option<Effect>| {
        for &victim in victims {
            if let Some(effect) = f(victim, rng, world) {
                effects.push(effect);
            }
        }
    };
    match kind {
        Kind::Purify => effects = purify(spell, strength, victims, rng, world),
        Kind::Enchant => effects = enchant(spell, strength, victims, rng, world),
        Kind::GoldLoss => {
            if victims.iter().any(|v| matches!(v, Victim::Member(_))) {
                // Retail's second roll when the first is below 100 is intentional.
                let first = magnitude(rng, spell.dice, strength);
                let percent = if first < 100 { magnitude(rng, spell.dice, strength) } else { 100 };
                let inventory = &mut world.inventory;
                // Malformed/modded percentages cannot wrap the unsigned purse.
                let lost = (u64::from(inventory.gold) * u64::from(percent.min(100)) / 100) as u32;
                inventory.gold -= lost;
                effects.push(Effect::GoldLost { amount: lost });
            }
        }
        Kind::DestroyConsumables => effects = destroy_consumables(strength, victims, rng, world),
        Kind::DrainMagic => {
            if victims.iter().any(|v| matches!(v, Victim::Member(_))) {
                let base = u32::from(strength.power) * 50;
                let amount = base + base * strength.boost / 100;
                // Anti-Magic drains the whole living party, not only captured victims.
                for member in world.party.iter() {
                    if !world.members.get(member).unwrap().1.is_dead() {
                        let maximum = world.mana_maximum(member);
                        let (_, mut pool) = world.points.get_mut(member).unwrap();
                        let amount = pool.drain(amount, &maximum, rng);
                        effects.push(Effect::Drained { victim: Victim::Member(member), amount });
                    }
                }
                // Anti-Magic clears persistent party slots, not combat or actor enchantments.
                world.enchantments.0.0.retain(|e| super::enchantments::combat_only(e.spell));
            }
        }
        Kind::RestoreMagic => each(&mut |v, rng, world| {
            let Victim::Member(member) = v else { return None };
            let (_, vitals, ..) = world.members.get(member).unwrap();
            if vitals.is_dead() {
                return None;
            }
            // Retail rolls even when every realm is already full.
            let amount = magnitude(rng, spell.dice, strength);
            let (_, mut pool) = world.points.get_mut(member).unwrap();
            let amount = pool.restore_evenly(amount);
            Some(if amount == 0 {
                Effect::Nothing { victim: v }
            } else {
                Effect::Replenished { victim: v, amount }
            })
        }),
        Kind::RestoreHealth => {
            // Separate passes preserve retail target and RNG order.
            each(&mut |v, rng, world| {
                let amount = restore(rng, world, v, Pool::Health, spell.dice, strength)?;
                Some(Effect::Healed { victim: v, amount })
            });
            let dice = spell.dice.map(|dice| Dice { count: dice.count.saturating_mul(2), ..dice });
            each(&mut |v, rng, world| {
                let amount = restore(rng, world, v, Pool::Stamina, dice, strength)?;
                Some(Effect::Refreshed { victim: v, amount })
            });
        }
        Kind::Damage => each(&mut |v, rng, world| {
            let rolled = magnitude(rng, spell.dice, strength);
            damage(rng, world, v, realm, rolled, strength.against)
        }),
        Kind::DamageUndead { banish } => each(&mut |v, rng, world| {
            let Victim::Monster(m) = v else { return None };
            let (body, ..) = world.monsters.monsters().get(m).unwrap();
            let kind = world.species()[body.species].kind;
            if !(UNDEAD_KINDS.contains(&kind) || (banish && kind == BANISHED_KIND)) {
                return Some(Effect::Nothing { victim: v });
            }
            let rolled = magnitude(rng, spell.dice, strength);
            damage(rng, world, v, realm, rolled, strength.against)
        }),
        Kind::Heal | Kind::Stamina | Kind::Restoration => {
            each(&mut |v, rng, world| restore_victim(kind, spell, strength, v, rng, world))
        }
        Kind::Inflict(condition, chance) => each(&mut |v, rng, world| {
            attack(rng, world, spell, v, realm, condition, chance, strength)
        }),
        Kind::DamageThen(..) | Kind::NoxiousFumes => {
            each(&mut |v, rng, world| {
                let rolled = magnitude(rng, spell.dice, strength);
                damage(rng, world, v, realm, rolled, strength.against)
            });
            let attacks = match kind {
                Kind::DamageThen(condition, chance) => vec![(condition, chance)],
                _ => vec![(Condition::Nauseated, 90), (Condition::Unconscious, 25)],
            };
            for (condition, chance) in attacks {
                each(&mut |v, rng, world| {
                    attack(rng, world, spell, v, realm, condition, chance, strength)
                });
            }
        }
        Kind::Fear { second } => each(&mut |v, rng, world| {
            let landed = attack(rng, world, spell, v, realm, Condition::Afraid, 100, strength);
            let again = matches!(
                (v, second),
                (Victim::Member(_), Condition::Unconscious)
                    | (Victim::Monster(_), Condition::Afraid)
            );
            if matches!(landed, Some(Effect::Inflicted { .. })) && again {
                let half = Strength { against: strength.against / 2, ..strength };
                attack(rng, world, spell, v, realm, second, 100, half);
            }
            landed
        }),
        Kind::Pandemonium => each(&mut |v, rng, world| match attack(
            rng,
            world,
            spell,
            v,
            realm,
            Condition::Insane,
            100,
            strength,
        ) {
            Some(Effect::Resisted { .. }) => {
                let target = target(world, v, realm, Some(Condition::Afraid));
                let rounds = duration(rng, spell, strength);
                let rounds = resist(rng, rounds, target.resistance, target.level, strength.against);
                if rounds == 0 || target.immune {
                    return Some(Effect::Resisted { victim: v });
                }
                let lasting = Lasting::Rounds(rounds.max(2).min(u32::from(u16::MAX)) as u16);
                inflict_on(world, v, (Condition::Afraid, 0), lasting)?;
                Some(Effect::Inflicted { victim: v, condition: Condition::Afraid })
            }
            other => other,
        }),
        Kind::Slay => each(&mut |v, rng, world| slay(rng, world, spell, v, realm, strength)),
        Kind::Affliction => each(&mut |v, rng, world| {
            let roll = rng.below(100);
            if roll >= 50 {
                let rolled = magnitude(rng, spell.dice, strength);
                return damage(rng, world, v, realm, rolled, strength.against);
            }
            let pick = crate::game::rng::weighted(roll, AFFLICTIONS.iter().map(|&(_, w)| w))
                .and_then(|index| AFFLICTIONS[index].0);
            match pick {
                Some(condition) => attack(rng, world, spell, v, realm, condition, 100, strength),
                None => slay(rng, world, spell, v, realm, strength),
            }
        }),
        Kind::Vortex => each(&mut |v, rng, world| {
            match v {
                Victim::Monster(_) => {
                    let rolled = magnitude(rng, spell.dice, strength);
                    damage(rng, world, v, realm, rolled, strength.against)
                }
                // Members take the plain roll, without the cast's boost.
                Victim::Member(_) => {
                    let rolled = magnitude(rng, spell.dice, Strength { boost: 0, ..strength });
                    let target = target(world, v, realm, None);
                    if target.dead {
                        return None;
                    }
                    match resist(rng, rolled, target.resistance, target.level, strength.against) {
                        0 => Some(Effect::Resisted { victim: v }),
                        amount => sap(rng, world, v, amount),
                    }
                }
            }
        }),
        Kind::Renewal => each(&mut |v, rng, world| {
            let lifted = match v {
                Victim::Member(m) => {
                    let (mut character, mut vitals, ..) = world.members.get_mut(m).unwrap();
                    vitals.lift_drain(&mut character)
                }
                Victim::Monster(m) => {
                    let conditions = &mut *world.monsters.conditions_mut(m);
                    !cure(rng, conditions, &[Condition::Drained], strength, true).is_empty()
                }
            };
            Some(if lifted {
                Effect::Cured { victim: v, condition: Condition::Drained }
            } else {
                Effect::Nothing { victim: v }
            })
        }),
        Kind::Cure(conditions) => each(&mut |v, rng, world| {
            let lifted = match v {
                Victim::Member(m) => {
                    let (_, mut vitals, ..) = world.members.get_mut(m).unwrap();
                    // Taking rounds off a poison weakens it as aging does.
                    if conditions.contains(&Condition::Poisoned) {
                        vitals.weaken_poison(cure_rounds(strength), rng);
                    }
                    cure(rng, &mut vitals.conditions, conditions, strength, false)
                }
                Victim::Monster(m) => {
                    cure(rng, &mut world.monsters.conditions_mut(m), conditions, strength, false)
                }
            };
            Some(match lifted.first() {
                Some(&condition) => Effect::Cured { victim: v, condition },
                None => Effect::Nothing { victim: v },
            })
        }),
        Kind::Resurrect => each(&mut |v, _, world| {
            let Victim::Member(member) = v else { return None };
            let (character, mut vitals, ..) = world.members.get_mut(member).unwrap();
            if !vitals.is_dead() {
                return Some(Effect::Nothing { victim: v });
            }
            // Retail lifts Dead with health and stamina at
            // `max·((PIE + VIT) >> 2)/100`, at least 1.
            let stats = character.stats(&vitals.enchantments);
            let share =
                (stats.attribute(Attribute::Piety) + stats.attribute(Attribute::Vitality)) >> 2;
            vitals.hp = (character.stats(&vitals.enchantments).max_hp() * share / 100).max(1);
            vitals.stamina =
                (character.stats(&vitals.enchantments).max_stamina() * share / 100).max(1);
            world.formation.update_seat(member, false);
            Some(Effect::Raised { victim: v })
        }),
        Kind::Lifesteal | Kind::MightToMagic => {
            let mut drained = 0;
            each(&mut |v, rng, world| {
                let target = target(world, v, realm, None);
                if target.dead {
                    return None;
                }
                let rolled = magnitude(rng, spell.dice, strength);
                let effect = damage(rng, world, v, realm, rolled, strength.against)?;
                if let Effect::Damaged { outcome, .. } = effect {
                    drained += outcome.damage;
                }
                Some(effect)
            });
            if let Some(Victim::Monster(entity)) = origin {
                if kind == Kind::Lifesteal {
                    world.monsters.restore_health(entity, drained * 7 / 10);
                } else {
                    world.monsters.restore_magic(entity, drained / 2);
                }
                return effects;
            }
            if origin.is_none() {
                return effects;
            }
            // Retail chooses once after every target has been drained. Do not
            // heal a different member after each hit or round each share separately.
            if let Some(member) = neediest(world, kind == Kind::Lifesteal) {
                let (c, mut v, ..) = world.members.get_mut(member).unwrap();
                if kind == Kind::Lifesteal {
                    let maximum = c.stats(&v.enchantments).max_hp();
                    let before = v.hp;
                    v.heal(drained * LIFESTEAL_SHARE / 100, maximum);
                    effects.push(Effect::Healed {
                        victim: Victim::Member(member),
                        amount: v.hp - before,
                    });
                } else {
                    let (_, mut pool) = world.points.get_mut(member).unwrap();
                    let amount = pool.restore_evenly(drained);
                    effects.push(Effect::Replenished { victim: Victim::Member(member), amount });
                }
            }
        }
    }
    effects
}

/// Whether anything the spell did counts as affecting a victim, which is
/// what practice needs (retail `SpellAffectedTarget`).
pub fn affected(effects: &[Effect]) -> bool {
    effects.iter().any(|e| !matches!(e, Effect::Nothing { .. }))
}
