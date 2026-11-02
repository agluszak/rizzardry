//! A body a hit lands on, a party member's or a monster's, and the rules
//! that act on its components. A rule asks for the component it needs; a
//! member's own rules (Iron Will, cheating death, poison strength, spell
//! points) apply where the body has a `Character`, a monster's where it has
//! a `Monster`.

use bevy::ecs::query::QueryData;
use bevy::prelude::*;

use crate::content::characters::{Attribute, Realm, Skill, Trait};
use crate::content::items::ItemCatalog;
use crate::content::species::{MonsterCatalog, MonsterKind};
use crate::content::spells::SpellCatalog;
use crate::game::characters::{Character, Condition, Conditions, Fate, Lasting, Vitals};
use crate::game::combat::Combatant;
use crate::game::difficulty::Difficulty;
use crate::game::factions::Factions;
use crate::game::inventory::Carried;
use crate::game::magic::enchantments::Enchantments;
use crate::game::magic::pointcasts::LureInfluence;
use crate::game::magic::{LearnedSpells, SpellPoints};
use crate::game::rng::GameRng;
use crate::game::world::hostility::{self, Stance};
use crate::game::world::identify::{GroupMember, MonsterGroup};
use crate::game::world::population::Monster;

/// Whatever a hit may read or change of the body it lands on.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct Body {
    pub entity: Entity,
    pub vitals: &'static mut Vitals,
    pub character: Option<&'static mut Character>,
    pub carried: Option<&'static mut Carried>,
    pub magic: Option<(&'static LearnedSpells, &'static mut SpellPoints)>,
    pub monster: Option<&'static mut Monster>,
    pub group: Option<&'static GroupMember>,
    pub fighting: Has<Combatant>,
    pub lure: Option<&'static LureInfluence>,
    pub summoned: Has<crate::game::magic::pointcasts::SummonedBy>,
    pub goal: Option<&'static mut bevy_landmass::AgentTarget3d>,
}

/// Retail's marker for a condition without end (`W8_CONDITION_INDEFINITE`),
/// which resistance cuts like any other duration.
pub const INDEFINITE: u32 = 9999;

/// Each condition's base save (retail `g_condition_resist_base`), by retail
/// number (Drained is 1, Dead 18).
pub const CONDITION_SAVE: [i32; 20] =
    [0, 1, 2, 3, 4, 4, 5, 6, 7, 8, 9, 10, 12, 14, 16, 20, 18, 22, 24, 22];
pub const DEATH_SAVE: i32 = CONDITION_SAVE[18];

/// A condition's base save.
pub fn condition_save(condition: Condition) -> i32 {
    CONDITION_SAVE[condition as usize + 1]
}

/// Whether a monster of `kind` shrugs a condition off (retail
/// `g_condition_immunities`).
pub fn immune(kind: MonsterKind, condition: Condition) -> bool {
    use Condition::*;
    let immune: &[_] = match kind {
        MonsterKind::Undead => {
            &[Diseased, Poisoned, Unconscious, Irritated, Afraid, Insane, Asleep, Nauseated]
        }
        MonsterKind::Android => &[Diseased, Poisoned, Nauseated, Irritated, Afraid, Asleep],
        MonsterKind::Plant => &[Insane, Afraid, Nauseated],
        _ => &[],
    };
    immune.contains(&condition)
}

/// `(level − strength)·3 + base + resistance`, the chance a victim saves.
pub fn condition_chance(level: u32, resistance: i32, against: u32, base: i32) -> i32 {
    (level as i32 - against as i32) * 3 + base + resistance
}

/// A saving throw at `chance` percent, held within 5..95 (retail
/// `TargetResistsCondition`).
pub fn save_roll(rng: &mut GameRng, chance: i32) -> bool {
    (rng.below(100) as i32) < chance.clamp(5, 95)
}

/// Retail `ReduceMagnitudeByResistance`: resistance and the victim's level
/// over the attack's strength take a random share off.
pub fn resist(rng: &mut GameRng, amount: u32, resistance: i32, level: u32, against: u32) -> u32 {
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

/// What the body is, as resistance and saves see it.
pub struct Defence {
    pub level: u32,
    pub resistance: i32,
    /// Its stance toward the party, which difficulty scales its saves by.
    pub side: Stance,
}

impl BodyItem<'_, '_> {
    pub fn is_dead(&self) -> bool {
        self.vitals.is_dead()
    }

    /// The level saves and resistance weigh: a member's, or a monster's
    /// species'.
    pub fn level(&self, species: &MonsterCatalog) -> u32 {
        match (&self.character, &self.monster) {
            (Some(character), _) => u32::from(character.level()),
            (None, Some(monster)) => u32::from(species[monster.species].level),
            (None, None) => 0,
        }
    }

    /// Its resistance to a realm: a member's from their stats, what they
    /// wear and the party's spells; a monster's from its species and spells.
    pub fn resistance(
        &self,
        realm: Realm,
        (items, species, party): (&ItemCatalog, &MonsterCatalog, &Enchantments),
    ) -> i32 {
        match (&self.character, &self.carried, &self.monster) {
            (Some(character), Some(carried), _) => {
                crate::game::characters::resistance::member_resistances(
                    &character.stats(&self.vitals.enchantments),
                    carried,
                    items,
                    party,
                )[realm]
                    .total
            }
            (_, _, Some(monster)) => crate::game::world::population::resistance(
                &species[monster.species],
                &self.vitals,
                realm,
            ),
            _ => 0,
        }
    }

    /// Its stance toward the party at `now`: a member's side is the party's,
    /// a monster's its group's, and Turncoat swaps either.
    pub fn side(
        &self,
        species: &MonsterCatalog,
        (groups, factions, now): (&Query<&MonsterGroup>, &Factions, u64),
    ) -> Stance {
        let own = match &self.monster {
            Some(monster) => {
                let set =
                    self.group.and_then(|member| groups.get(member.0).ok()).and_then(|g| g.set);
                hostility::stance(set, &species[monster.species], factions, now, self.fighting)
            }
            None => Stance::Friendly,
        };
        hostility::own_stance(own, self.vitals.conditions.has(Condition::Turncoat))
    }

    pub fn defence(
        &self,
        realm: Realm,
        (items, species, party): (&ItemCatalog, &MonsterCatalog, &Enchantments),
        sides: (&Query<&MonsterGroup>, &Factions, u64),
    ) -> Defence {
        Defence {
            level: self.level(species),
            resistance: self.resistance(realm, (items, species, party)),
            side: self.side(species, sides),
        }
    }

    /// Whether its kind shrugs the condition off before any save: a
    /// monster's species' immunities.
    pub fn immune(&self, condition: Condition, species: &MonsterCatalog) -> bool {
        self.monster.as_ref().is_some_and(|m| immune(species[m.species].kind, condition))
    }

    /// Whether its kind cannot be slain outright.
    pub fn death_immune(&self, species: &MonsterCatalog) -> bool {
        self.monster.as_ref().is_some_and(|m| species[m.species].death_immune)
    }

    /// A member's constitution and profession immunities, checked after the
    /// save.
    pub fn shrugs(&self, condition: Condition, indefinite: bool) -> bool {
        let Some(character) = &self.character else { return false };
        let android = character.has_trait(Trait::AndroidConditionImmunity);
        match condition {
            Condition::Afraid => character.has_trait(Trait::Fearless) || android,
            Condition::Diseased
            | Condition::Irritated
            | Condition::Nauseated
            | Condition::Poisoned
            | Condition::Asleep => android,
            Condition::Insane if indefinite => false,
            Condition::Insane | Condition::Turncoat => {
                character.has_trait(Trait::MentalConditionImmunity)
            }
            _ => false,
        }
    }

    /// Whether it saves (retail `TargetResistsCondition`), scaled by
    /// difficulty for its side, within 5..95. A member who saves practises
    /// Iron Will; the skill improved is returned with the save.
    pub fn saves(
        &mut self,
        rng: &mut GameRng,
        defence: &Defence,
        (base, against): (i32, u32),
        difficulty: Difficulty,
    ) -> (bool, Option<Skill>) {
        let chance = condition_chance(defence.level, defence.resistance, against, base);
        let chance = difficulty.scale(chance, defence.side);
        let saved = save_roll(rng, chance);
        let improved = (saved
            && self.character.as_mut().is_some_and(|c| c.practise(Skill::IronWill, 2, rng)))
        .then_some(Skill::IronWill);
        (saved, improved)
    }

    /// Take `amount` health. A member takes it as retail
    /// `ApplyDamageToCharacter` (fatigue, Guardian Angel, cheating death)
    /// and may wake; a monster's spells absorb what they can, it may wake,
    /// and dying ends its spells. Returns the health lost.
    pub fn hurt(
        &mut self,
        amount: u32,
        rng: &mut GameRng,
        (species, fight): (&MonsterCatalog, Option<Entity>),
    ) -> u32 {
        if let Some(character) = &self.character {
            let lost = self.vitals.damage(character, amount, &mut Fate { fight, rng: &mut *rng });
            if lost > 0 && !self.vitals.is_dead() {
                let senses =
                    character.stats(&self.vitals.enchantments).attribute(Attribute::Senses);
                self.vitals.conditions.wake_on_hit(senses, rng);
            }
            return lost;
        }
        let Some(monster) = &self.monster else { return 0 };
        if self.vitals.is_dead() || amount == 0 {
            return 0;
        }
        let amount = self.vitals.enchantments.absorb(amount);
        let lost = amount.min(self.vitals.health.current);
        self.vitals.health.current -= lost;
        if lost == 0 {
            return 0;
        }
        if self.vitals.is_dead() {
            self.vitals.enchantments.0.clear();
        } else {
            let senses = u32::from(species[monster.species].attributes.senses);
            self.vitals.conditions.wake_on_hit(senses, rng);
        }
        lost
    }

    /// Drain a member's health for good ([`Vitals::drain_health`]); nothing
    /// wakes them. Other bodies have no lasting maximum to lose.
    pub fn sap(&mut self, amount: u32, rng: &mut GameRng, fight: Option<Entity>) -> Option<u32> {
        let character = self.character.as_mut()?;
        Some(self.vitals.drain_health(character, amount, &mut Fate { fight, rng }))
    }

    /// Die outright, as death effects inflict it: a member may cheat death
    /// (retail `SetCharacterCondition` with Dead); a monster's Guardian
    /// Angel does not save it. Returns the health lost.
    pub fn slay(
        &mut self,
        rng: &mut GameRng,
        (species, fight): (&MonsterCatalog, Option<Entity>),
    ) -> u32 {
        let had = self.vitals.health.current;
        if let Some(character) = &self.character {
            self.vitals.perish(character, &mut Fate { fight, rng });
            return had - self.vitals.health.current.min(had);
        }
        self.vitals
            .enchantments
            .0
            .retain(|e| e.spell != crate::content::spells::SpellId::GUARDIAN_ANGEL);
        self.hurt(had, rng, (species, fight))
    }

    /// Give it a condition for `lasting`. Only a member's poison bites by a
    /// strength; one standing neutral cannot be turned. Whether it took.
    pub fn inflict(
        &mut self,
        (condition, poison): (Condition, u32),
        lasting: Lasting,
        side: Stance,
    ) -> bool {
        if condition == Condition::Turncoat && side == Stance::Neutral {
            return false;
        }
        if self.character.is_some() && condition == Condition::Poisoned {
            return self.vitals.poison(lasting, poison);
        }
        self.vitals.inflict(condition, lasting);
        true
    }

    /// A member's spell point maxima, by realm.
    pub fn mana_maximum(&self, spells: &SpellCatalog) -> Option<enum_map::EnumMap<Realm, u32>> {
        let character = self.character.as_ref()?;
        let (learned, _) = self.magic.as_ref()?;
        Some(character.stats(&self.vitals.enchantments).max_points(learned, spells))
    }

    /// Drain spell points: a member's pool by the share retail takes, a
    /// monster's budget directly. What was taken.
    pub fn drain_magic(
        &mut self,
        amount: u32,
        rng: &mut GameRng,
        (spells, species): (&SpellCatalog, &MonsterCatalog),
    ) -> u32 {
        if let Some(maximum) = self.mana_maximum(spells) {
            let (_, pool) = self.magic.as_mut().unwrap();
            return pool.drain(amount, &maximum, rng);
        }
        let Some(monster) = self.monster.as_mut() else { return 0 };
        let kind = &species[monster.species];
        let lost = monster.spell_budget(kind).min(amount);
        monster.spell_points -= lost as i32;
        lost
    }
}

/// Take `amount` health from a body, with what being hurt brings: a
/// member's damage notice, a lured monster breaking free of the lure. A
/// kill's experience is its species', unless it was summoned.
pub fn wound(
    body: &mut BodyItem,
    amount: u32,
    rng: &mut GameRng,
    (species, fight): (&MonsterCatalog, Option<Entity>),
    (commands, notices): (&mut Commands, &mut MessageWriter<crate::game::party::DamageApplied>),
) -> super::Effect {
    let lost = body.hurt(amount, rng, (species, fight));
    let died = body.is_dead() && lost > 0;
    if body.character.is_some() && lost > 0 {
        notices.write(crate::game::party::DamageApplied {
            member: body.entity,
            amount: lost,
            died: body.is_dead(),
        });
    }
    if lost > 0
        && let Some(&lure) = body.lure
    {
        commands.entity(body.entity).insert(LureInfluence {
            response: crate::game::magic::pointcasts::LureResponse::Resisted,
            ..lure
        });
        if !body.fighting
            && let Some(goal) = body.goal.as_mut()
        {
            **goal = bevy_landmass::AgentTarget3d::None;
        }
    }
    let experience = match &body.monster {
        Some(monster) if died && !body.summoned => species[monster.species].experience,
        _ => 0,
    };
    super::Effect::Damaged {
        victim: body.entity,
        outcome: crate::game::world::damage::DamageOutcome { damage: lost, died, experience },
    }
}

/// The rounds a cure takes off a timed condition: `power²`, boosted.
pub fn cure_rounds(strength: crate::game::magic::Strength) -> u32 {
    let per = strength.power.pow(2);
    per + per * strength.boost / 100
}

/// Retail `TryCureConditionOnTargets`: a timed condition loses `power²`
/// rounds (and goes if that is all of them); a lasting one goes with chance
/// `power·15`%. A forced cure lifts them all.
pub fn cure(
    rng: &mut GameRng,
    conditions: &mut Conditions,
    cured: &[Condition],
    strength: crate::game::magic::Strength,
    forced: bool,
) -> Vec<Condition> {
    let per = cure_rounds(strength);
    let mut lifted = Vec::new();
    for &condition in cured {
        let Some(lasting) = conditions.lasting_mut(condition) else { continue };
        let gone = match lasting {
            _ if forced => true,
            Lasting::Rounds(left) if *left <= per => true,
            Lasting::Rounds(left) => {
                *left -= per;
                false
            }
            Lasting::Indefinite => rng.below(100) < strength.power * 15,
        };
        if gone {
            conditions.lift(condition);
            lifted.push(condition);
        }
    }
    lifted
}
