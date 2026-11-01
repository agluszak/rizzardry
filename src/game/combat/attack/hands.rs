//! What a member fights with and how well. [`Weapons`] is what the two hands
//! hold, a [`Wielded`] weapon is one that can attack, and each moment of a
//! fight asks it what it needs. Retail caches these answers on the character
//! (`CalcAttacks`); nothing here is stored.
use super::*;

/// The weapons that need ammunition from the other hand (retail
/// `ItemHasSingledOutGenericName`): longbow, crossbow, sling, greatbow and
/// omnigun.
fn is_launcher(kind: ItemKind) -> bool {
    matches!(
        kind,
        ItemKind::LONGBOW
            | ItemKind::CROSSBOW
            | ItemKind::SLING
            | ItemKind::GREATBOW
            | ItemKind::OMNIGUN
    )
}

/// Unarmed damage by Martial Arts level / 11 (retail
/// `g_unarmed_damage_dice`), with 2 more in the main hand.
fn unarmed_dice(martial_arts: u32) -> Dice {
    const DICE: [(i32, i32, i32); 12] = [
        (0, 1, 2),
        (0, 1, 3),
        (0, 2, 2),
        (0, 2, 3),
        (0, 2, 4),
        (0, 3, 3),
        (1, 3, 3),
        (2, 3, 3),
        (0, 3, 5),
        (0, 4, 4),
        (2, 4, 4),
        (4, 4, 4),
    ];
    let (base, count, sides) = DICE[(martial_arts as usize / 11).min(11)];
    Dice { base, count, sides }
}

/// What a hand strikes with: a bare fist, or an item with what it shoots
/// from the other hand, if it is a loaded launcher.
#[derive(Clone, Copy, Debug)]
pub enum Weapon<'a> {
    Fist,
    Item { item: &'a ItemDefinition, shot: Option<&'a ItemDefinition> },
}

/// What a blow does: its dice, the ways it strikes (one is chosen per
/// attack) and its effects besides damage.
#[derive(Clone, Copy, Debug)]
pub struct Strike {
    pub dice: Dice,
    pub modes: EnumSet<AttackMode>,
    pub effects: AttackEffects,
}

impl Weapon<'_> {
    /// A launcher takes its dice and adds the effects of what it shoots. An
    /// empty hand punches, kicks too past Martial Arts 4, and may knock out
    /// by strength; the main hand hits 2 harder.
    fn strike(self, c: &CharacterStats<'_>, main_hand: bool) -> Strike {
        match self {
            Self::Item { item, shot } => Strike {
                dice: shot.unwrap_or(item).damage,
                modes: item.attack_modes,
                effects: shot.map_or(item.effects, |a| item.effects.with(&a.effects)),
            },
            Self::Fist => {
                let martial_arts = c.skill(Skill::MartialArts);
                let strength = c.attribute(Attribute::Strength) as i32;
                let mut dice = unarmed_dice(martial_arts);
                dice.base += if main_hand { 2 } else { 0 };
                let mut effects = AttackEffects::default();
                if strength > 49 {
                    effects.chances[AttackEffect::KnockOut] = ((strength - 50) / 5) as u32;
                }
                let modes = if martial_arts > 4 {
                    AttackMode::Punch | AttackMode::Kick
                } else {
                    EnumSet::only(AttackMode::Punch)
                };
                Strike { dice, modes, effects }
            }
        }
    }
}

/// A weapon that can attack, as its hand and the other one leave it.
#[derive(Clone, Copy, Debug)]
pub struct Wielded<'a> {
    pub weapon: Weapon<'a>,
    pub skill: Skill,
    /// Its blows are ranged combat, not close combat.
    pub ranged: bool,
    pub reach: Reach,
    off_hand: bool,
    /// The skill of an attacking weapon in the other hand.
    beside: Option<Skill>,
    dual_wielding: bool,
    /// What ammunition in the other hand adds to its to-hit.
    ammunition_hit: i32,
    /// A rocket launcher in the main hand allows only one swing.
    rocket: bool,
}

/// What a member's two hands hold, read when taken: an attack keeps its
/// weapons though it shoots away what they fire.
#[derive(Clone, Copy, Debug)]
pub struct Weapons<'a> {
    held: [Weapon<'a>; 2],
    skills: [Skill; 2],
    /// Hand 0 is the main hand. A hand that cannot attack has none.
    pub hands: [Option<Wielded<'a>>; 2],
}

fn is_ranged(skill: Skill) -> Option<bool> {
    match skill {
        Skill::ModernWeapon | Skill::Bow | Skill::ThrowingAndSling => Some(true),
        skill if skill as usize <= Skill::Shield as usize || skill == Skill::MartialArts => {
            Some(false)
        }
        _ => None,
    }
}

impl<'a> Weapons<'a> {
    /// An off hand attacks only beside a main hand of the same style, never
    /// with a shield or ammunition or beside an omnigun, and an empty one
    /// only beside another. A ranged hand cannot attack with a launcher
    /// lacking its ammunition or a gun out of shots.
    pub fn of(carried: &Carried, catalog: &'a ItemCatalog) -> Self {
        let stacks = [EquipSlot::Primary, EquipSlot::Secondary].map(|slot| carried.worn(slot));
        let items = stacks.map(|stack| stack.map(|stack| &catalog[stack.item]));
        let omnigun = items[0].is_some_and(|item| item.kind == ItemKind::OMNIGUN);
        let class = |hand: usize| items[hand].map(|item| item.class);
        let ammunition = |hand: usize| match class(hand) {
            Some(EquipClass::Ammunition) => true,
            Some(EquipClass::ThrownWeapon) => hand == 1 && omnigun,
            _ => false,
        };
        let blade = |hand: usize| {
            !ammunition(hand)
                && matches!(
                    class(hand),
                    Some(
                        EquipClass::ShortWeapon
                            | EquipClass::ExtendedWeapon
                            | EquipClass::RangedWeapon
                            | EquipClass::ThrownWeapon
                    )
                )
        };
        let skills = items.map(|item| item.map_or(Some(Skill::MartialArts), |d| d.weapon_skill));
        let styles = skills.map(|skill| skill.and_then(is_ranged));
        let loaded = |hand: usize| {
            let (Some(stack), Some(item)) = (stacks[hand], items[hand]) else { return true };
            let partner = items[1 - hand];
            (!is_launcher(item.kind)
                || partner.is_some_and(|o| crate::game::inventory::partners(item, o)))
                && !(item.quantity == QuantityKind::Shots && stack.remaining_uses() == Some(0))
        };
        let able =
            |hand: usize| styles[hand] == Some(false) || styles[hand] == Some(true) && loaded(hand);
        let off_hand = styles[0] == styles[1]
            && !ammunition(1)
            && class(1) != Some(EquipClass::Shield)
            && !omnigun
            && (items[1].is_some() || items[0].is_none());
        let in_play = [able(0), able(1) && off_hand];
        let skill = |hand: usize| skills[hand].unwrap_or(Skill::MartialArts);
        let held = [0, 1].map(|hand| match items[hand] {
            None => Weapon::Fist,
            Some(item) => {
                let loaded = is_launcher(item.kind) && in_play[hand];
                Weapon::Item { item, shot: items[1 - hand].filter(|_| loaded) }
            }
        });
        let dual_wielding = blade(0) && blade(1) && in_play == [true, true];
        let rocket = items[0].is_some_and(|item| item.kind == ItemKind::ROCKET_LAUNCHER);
        let wielded = |hand: usize| {
            in_play[hand].then(|| Wielded {
                weapon: held[hand],
                skill: skill(hand),
                ranged: styles[hand] == Some(true),
                reach: items[hand]
                    .filter(|_| blade(hand) || ammunition(hand))
                    .map_or(Reach::Touch, |item| item.reach),
                off_hand: hand == 1,
                beside: (blade(1 - hand) && in_play[1 - hand]).then(|| skill(1 - hand)),
                dual_wielding,
                ammunition_hit: items[1 - hand]
                    .filter(|_| items[hand].is_some() && ammunition(1 - hand))
                    .map_or(0, |item| item.hit_bonus),
                rocket,
            })
        };
        Self { held, skills: [skill(0), skill(1)], hands: [wielded(0), wielded(1)] }
    }

    /// The skill of what a hand holds, attacking or not.
    pub fn skill(&self, hand: usize) -> Skill {
        self.skills[hand]
    }

    /// What a hand strikes with, even one that cannot attack itself: a
    /// punch or a kick, or the modes of what a launcher shoots.
    pub fn strike(&self, c: &CharacterStats<'_>, hand: usize) -> Strike {
        self.held[hand].strike(c, hand == 0)
    }

    /// The hand an attack opens with (retail `ChooseCharacterAttackHand`):
    /// the most attacks, then the bigger initiative, then the main hand.
    pub fn opening_hand(&self, c: &CharacterStats<'_>, load: Load) -> Option<usize> {
        let able = (0..2).rev().filter_map(|hand| self.hands[hand].map(|weapon| (hand, weapon)));
        able.max_by_key(|(_, weapon)| (weapon.attacks(c, load), weapon.initiative(c)))
            .map(|(hand, _)| hand)
    }
}

impl Wielded<'_> {
    fn item(&self) -> Option<&ItemDefinition> {
        match self.weapon {
            Weapon::Fist => None,
            Weapon::Item { item, .. } => Some(item),
        }
    }

    /// It shoots ammunition from the other hand.
    pub fn launcher(&self) -> bool {
        self.item().is_some_and(|item| is_launcher(item.kind))
    }

    /// A sling, which shoots by the rules of a bow.
    pub fn sling(&self) -> bool {
        self.item().is_some_and(|item| item.kind == ItemKind::SLING)
    }

    /// How many of the monster's kinds its weapon and ammunition slay.
    pub fn slays(&self, kind: crate::content::species::MonsterKind) -> u32 {
        let Weapon::Item { item, shot } = self.weapon else { return 0 };
        u32::from(item.slays == Some(kind)) + u32::from(shot.is_some_and(|a| a.slays == Some(kind)))
    }

    /// Swings the weapon adds to each attack.
    pub fn extra_swings(&self) -> i32 {
        self.item().map_or(0, |item| item.extra_swings)
    }

    /// What swinging it weighs, for its stamina.
    pub fn weight(&self) -> Option<u16> {
        self.item().map(|item| item.weight)
    }

    pub fn strike(&self, c: &CharacterStats<'_>) -> Strike {
        self.weapon.strike(c, !self.off_hand)
    }

    /// The skills it fights by: its own with its combat skill, the other
    /// weapon's and Dual Weapons.
    fn prowess(&self, c: &CharacterStats<'_>) -> i32 {
        let combat = if self.ranged { Skill::RangedCombat } else { Skill::CloseCombat };
        let mut score = ((c.skill(combat) + c.skill(self.skill) * 2) * 2 / 3) as i32;
        let mut divisor = 20;
        if let Some(other) = self.beside {
            score += c.skill(other) as i32 >> 1;
            divisor = 25;
        }
        if self.dual_wielding {
            score += c.skill(Skill::DualWeapons) as i32;
            divisor += 10;
        }
        score * 10 / divisor
    }

    /// Modern weapons feel load half as much.
    fn load_felt(&self, penalty: i32) -> i32 {
        if self.skill == Skill::ModernWeapon { penalty / 2 } else { penalty }
    }

    fn dual_penalty(&self, c: &CharacterStats<'_>) -> i32 {
        if !self.dual_wielding {
            return 0;
        }
        let hand = if self.off_hand { 2 } else { 1 };
        -10 * hand - (100 - c.skill(Skill::DualWeapons) as i32) / 4
    }

    /// What every speed of a hand shares.
    fn drill(&self, c: &CharacterStats<'_>, load: Load) -> i32 {
        self.dual_penalty(c)
            + c.physical_levels()
            + self.load_felt(-load.attack_penalty())
            + self.prowess(c)
    }

    /// Attacks in a round.
    pub fn attacks(&self, c: &CharacterStats<'_>, load: Load) -> u32 {
        let [speed, dexterity] =
            [Attribute::Speed, Attribute::Dexterity].map(|a| c.attribute(a) as i32);
        let score = (((speed + dexterity) >> 1) + self.drill(c, load)) / 3;
        if self.off_hand {
            1 + u32::from(score > 74)
        } else {
            1 + u32::from(score > 49) + u32::from(score > 99)
        }
    }

    /// The weapon's initiative bonus (retail `damage_bonus`).
    pub fn initiative(&self, c: &CharacterStats<'_>) -> i32 {
        self.item().map_or(self.prowess(c) / 10, |item| item.damage_bonus)
    }

    /// The most swings an attack rolls for.
    pub fn swings(&self, c: &CharacterStats<'_>, load: Load) -> u32 {
        if self.rocket {
            return 1;
        }
        // Retail reads the initiative its previous pass left, which is this.
        let speed = c.attribute(Attribute::Speed) as i32 + self.initiative(c) * 10;
        let score = (speed + self.drill(c, load)) / 3;
        1 + u32::from(score > 66) + u32::from(score > 99)
    }

    /// What strength adds to its to-hit and damage percent. Weapons only
    /// thrown, lashed or shot feel half of it, or none if they carry their
    /// own force; an off hand gains half as much from it.
    fn strength_bonus(&self, c: &CharacterStats<'_>) -> (i32, i32) {
        let distant = self.item().filter(|item| {
            item.attack_modes.is_subset(AttackMode::Throw | AttackMode::Lash | AttackMode::Shoot)
        });
        if distant.is_some_and(|item| NO_STRENGTH.contains(&item.kind)) {
            return (0, 0);
        }
        let divisor = if distant.is_some() { 2 } else { 1 };
        let strength = c.attribute(Attribute::Strength) as i32;
        if strength < 50 {
            (-((50 - strength) / (divisor * 10)), -((50 - strength) / divisor))
        } else if strength > 50 {
            let divisor = if self.off_hand { divisor * 2 } else { divisor };
            ((strength - 50) / (divisor * 10), (strength * 2 - 100) / divisor)
        } else {
            (0, 0)
        }
    }

    /// The sheet's "To Hit": weapon and ammunition, strength, dexterity,
    /// senses, load and trained strikes.
    pub fn to_hit(&self, c: &CharacterStats<'_>, load: Load) -> i32 {
        let weapon = self.item().map_or(0, |item| item.hit_bonus) + self.ammunition_hit;
        let senses = c.attribute(Attribute::Senses) as i32;
        let senses = match senses {
            ..30 => -((30 - senses) / 10),
            71.. => (senses - 70) / 10,
            _ => 0,
        };
        let load = if self.off_hand { -load.penalty() * 3 / 2 } else { -load.penalty() };
        let trained = if self.ranged { Skill::EagleEye } else { Skill::PowerStrike };
        let trained = if c.skill_available(trained) { c.skill(trained) as i32 / 20 + 1 } else { 0 };
        weapon
            + self.strength_bonus(c).0
            + (c.attribute(Attribute::Dexterity) as i32 - 50) / 10
            + senses
            + self.load_felt(load)
            + trained
    }

    /// The sheet's "To Damage (%)": strength, and a launcher's own base.
    pub fn damage_percent(&self, c: &CharacterStats<'_>) -> i32 {
        let launcher = self.item().filter(|item| is_launcher(item.kind));
        self.strength_bonus(c).1 + launcher.map_or(0, |item| item.damage.base * 10)
    }

    /// The score its blows start from (retail `attack_score` and
    /// `hit_bonus`), with the party's blessings.
    pub fn score(&self, c: &CharacterStats<'_>, load: Load, blessing: i32) -> i32 {
        let dexterity = c.attribute(Attribute::Dexterity) as i32;
        let rating =
            (self.dual_penalty(c) + dexterity / 2 + self.prowess(c) * 2 + c.physical_levels()) / 3;
        rating + 60 + (self.to_hit(c, load) + blessing) * 5
    }
}
