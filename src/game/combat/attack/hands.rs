//! What a member fights with: each hand's weapon or bare fist, its reach,
//! swings, dice and initiative (retail `ComputeCharacterHands`).
use super::*;

/// What a hand holds (retail `SetHandType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Holds {
    #[default]
    Nothing,
    Weapon,
    Shield,
    Ammunition,
    /// Anything else, which cannot strike.
    Other,
}

/// The weapons that need ammunition from the other hand (retail
/// `ItemHasSingledOutGenericName`): longbow, crossbow, sling, greatbow and
/// omnigun.
pub(super) fn is_launcher(kind: ItemKind) -> bool {
    matches!(
        kind,
        ItemKind::LONGBOW
            | ItemKind::CROSSBOW
            | ItemKind::SLING
            | ItemKind::GREATBOW
            | ItemKind::OMNIGUN
    )
}

/// One hand's attack, as the character sheet works it out (retail
/// `W8HandAttack`).
#[derive(Clone, Debug, PartialEq)]
pub struct Hand {
    pub holds: Holds,
    /// The hand can attack.
    pub in_play: bool,
    pub weapon_skill: Skill,
    /// Its blows are ranged combat, not close combat.
    pub ranged: bool,
    pub reach: Reach,
    pub dice: Dice,
    /// The ways it strikes; one is chosen per attack.
    pub modes: EnumSet<AttackMode>,
    pub attacks: u32,
    pub swings: u32,
    pub attack_rating: i32,
    /// The sheet's "To Hit": weapon, strength, dexterity and senses.
    pub to_hit: i32,
    /// The sheet's "To Damage (%)": strength.
    pub damage_percent: i32,
    /// The weapon's initiative byte (retail `damage_bonus`).
    pub initiative: i32,
    /// What its blows may do besides damage: a fist's knockout chance, or
    /// the weapon's effects with its ammunition's.
    pub effects: AttackEffects,
    /// The monster kinds its weapon and ammunition slay.
    pub slays: [Option<crate::content::species::MonsterKind>; 2],
    /// Swings the weapon adds to each attack.
    pub extra_swings: i32,
    /// It shoots ammunition from the other hand.
    pub paired: bool,
    /// What swinging it weighs, for its stamina.
    pub weight: Option<u16>,
    /// The weapon's retail kind.
    pub kind: Option<ItemKind>,
}

/// A member's two hands (retail `CalcAttacks`).
#[derive(Clone, Debug, PartialEq)]
pub struct Hands {
    pub hands: [Hand; 2],
    pub dual_wielding: bool,
}

impl Hands {
    /// The hand an attack opens with (retail `ChooseCharacterAttackHand`):
    /// the one with the most attacks, then the bigger initiative.
    pub fn choose(&self) -> Option<usize> {
        let mut best = None;
        for (index, hand) in self.hands.iter().enumerate() {
            let better = match best {
                None => hand.attacks > 0,
                Some(b) => {
                    let b: &Hand = &self.hands[b];
                    b.attacks < hand.attacks
                        || (hand.attacks == b.attacks && hand.initiative > b.initiative)
                }
            };
            if hand.in_play && better {
                best = Some(index);
            }
        }
        best
    }
}

/// Unarmed damage by Martial Arts level / 11 (retail
/// `g_unarmed_damage_dice`), with 2 more in the main hand.
pub(super) fn unarmed_dice(martial_arts: u32) -> Dice {
    const DICE: [(i16, u8, u8); 12] = [
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

/// Both hands' numbers (retail `CalcAttacks`). An empty hand fights with
/// Martial Arts; an off hand attacks only beside a main hand of the same
/// kind (close or ranged), and never with a shield or ammunition.
pub fn hands(
    c: &CharacterStats<'_>,
    carried: &Carried,
    catalog: &ItemCatalog,
    load: Load,
) -> Hands {
    const SLOTS: [EquipSlot; 2] = [EquipSlot::Primary, EquipSlot::Secondary];
    let worn = SLOTS.map(|slot| carried.worn(slot).map(|s| (s, &catalog[s.item])));
    let definition = |hand: usize| worn[hand].map(|(_, d)| d);
    let holds = |hand: usize| match definition(hand) {
        None => Holds::Nothing,
        Some(d) => match d.class {
            EquipClass::ShortWeapon | EquipClass::ExtendedWeapon | EquipClass::RangedWeapon => {
                Holds::Weapon
            }
            EquipClass::ThrownWeapon
                if hand == 1 && definition(0).is_some_and(|p| p.kind == ItemKind::OMNIGUN) =>
            {
                Holds::Ammunition
            }
            EquipClass::ThrownWeapon => Holds::Weapon,
            EquipClass::Ammunition => Holds::Ammunition,
            EquipClass::Shield => Holds::Shield,
            _ => Holds::Other,
        },
    };
    let holds = [holds(0), holds(1)];
    let weapon_skill =
        |hand: usize| definition(hand).map_or(Some(Skill::MartialArts), |d| d.weapon_skill);
    // Which combat skill each hand fights with: close, ranged, or none.
    let combat = |hand: usize| -> Option<bool> {
        match weapon_skill(hand)? {
            skill if skill as usize <= Skill::Shield as usize || skill == Skill::MartialArts => {
                Some(false)
            }
            Skill::ModernWeapon | Skill::Bow | Skill::ThrowingAndSling => Some(true),
            _ => None,
        }
    };
    let ranged = [combat(0), combat(1)];
    // A ranged hand cannot attack with a launcher lacking its ammunition,
    // or a gun out of shots; it keeps its combat skill all the same.
    let loaded = |hand: usize| {
        let Some((stack, d)) = worn[hand] else { return true };
        let other = definition(1 - hand);
        (!is_launcher(d.kind) || other.is_some_and(|o| crate::game::inventory::partners(d, o)))
            && !(d.quantity == QuantityKind::Shots && stack.remaining_uses() == Some(0))
    };
    let mut in_play = [0, 1].map(|hand| match ranged[hand] {
        None => false,
        Some(false) => true,
        Some(true) => loaded(hand),
    });
    if ranged[0] != ranged[1]
        || matches!(holds[1], Holds::Shield | Holds::Ammunition)
        || definition(0).is_some_and(|d| d.kind == ItemKind::OMNIGUN)
        || (holds[1] == Holds::Nothing && holds[0] != Holds::Nothing)
    {
        in_play[1] = false;
    }
    let dual_wielding = holds == [Holds::Weapon, Holds::Weapon] && in_play[0] && in_play[1];

    let physical = c.physical_levels();
    let dual = c.skill(Skill::DualWeapons) as i32;
    let [strength, dexterity, speed, senses] =
        [Attribute::Strength, Attribute::Dexterity, Attribute::Speed, Attribute::Senses]
            .map(|a| c.attribute(a) as i32);
    let hand = |index: usize| -> Hand {
        let skill = weapon_skill(index).unwrap_or(Skill::MartialArts);
        let ranged = ranged[index] == Some(true);
        let d = definition(index);
        // A launcher takes its damage, effects and slaying from what it shoots.
        let paired = d.is_some_and(|d| is_launcher(d.kind)) && in_play[index];
        let ammunition = if paired { definition(1 - index) } else { None };
        let mut hand = Hand {
            holds: holds[index],
            in_play: in_play[index],
            weapon_skill: skill,
            ranged,
            reach: if matches!(holds[index], Holds::Weapon | Holds::Ammunition) {
                d.map_or(Reach::Touch, |d| d.reach)
            } else {
                Reach::Touch
            },
            dice: Dice::default(),
            modes: EnumSet::empty(),
            attacks: 0,
            swings: 0,
            attack_rating: 0,
            to_hit: 0,
            damage_percent: 0,
            initiative: 0,
            effects: AttackEffects::default(),
            slays: [d.and_then(|d| d.slays), ammunition.and_then(|a| a.slays)],
            extra_swings: d.map_or(0, |d| d.extra_swings),
            paired,
            weight: d.map(|d| d.weight),
            kind: d.map(|d| d.kind),
        };
        // What it strikes with, which a punch, a kick or the shot a launcher
        // looses reads from a hand that cannot attack itself.
        match d {
            Some(d) => {
                hand.dice = ammunition.unwrap_or(d).damage;
                hand.modes = d.attack_modes;
                hand.effects = ammunition.map_or(d.effects, |a| d.effects.with(&a.effects));
            }
            None => {
                let martial_arts = c.skill(skill);
                // An empty hand punches, and kicks too past Martial Arts 4.
                hand.modes = if martial_arts > 4 {
                    AttackMode::Punch | AttackMode::Kick
                } else {
                    EnumSet::only(AttackMode::Punch)
                };
                hand.dice = unarmed_dice(martial_arts);
                if index == 0 {
                    hand.dice.base += 2;
                }
                if strength > 49 {
                    hand.effects.chances[AttackEffect::KnockOut] = ((strength - 50) / 5) as u8;
                }
            }
        }
        if !hand.in_play {
            return hand;
        }
        let other = 1 - index;
        let combat_skill = if ranged { Skill::RangedCombat } else { Skill::CloseCombat };
        let mut score = (c.skill(combat_skill) as i32 + c.skill(skill) as i32 * 2) * 2 / 3;
        let mut divisor = 20;
        if holds[other] == Holds::Weapon && in_play[other] {
            score += c.skill(weapon_skill(other).unwrap_or(Skill::MartialArts)) as i32 >> 1;
            divisor = 25;
        }
        if dual_wielding {
            score += dual;
            divisor += 10;
        }
        let combined = score * 10 / divisor;
        // Modern weapons feel load half as much.
        let modern = skill == Skill::ModernWeapon;
        let halved = |penalty: i32| if modern { penalty / 2 } else { penalty };
        let load_penalty = halved(-load.attack_penalty());
        let dual_penalty =
            if dual_wielding { -10 * (index as i32 + 1) - (100 - dual) / 4 } else { 0 };
        hand.attack_rating = (dual_penalty + dexterity / 2 + combined * 2 + physical) / 3 + 60;
        let score = ((speed + dexterity) >> 1) + dual_penalty + physical + load_penalty + combined;
        let score = score / 3;
        hand.attacks = match index {
            0 => 1 + u32::from(score > 49) + u32::from(score > 99),
            _ => 1 + u32::from(score > 74),
        };
        hand.initiative = d.map_or(combined / 10, |d| i32::from(d.damage_bonus));
        // Retail reads the initiative its previous pass left, which is this.
        let score =
            (speed + hand.initiative * 10 + dual_penalty + physical + load_penalty + combined) / 3;
        hand.swings = 1 + u32::from(score > 66) + u32::from(score > 99);
        if definition(0).is_some_and(|d| d.kind == ItemKind::ROCKET_LAUNCHER) {
            hand.swings = 1;
        }
        let mut strength_divisor = 1;
        let mut strength_helps = true;
        if let Some(d) = d {
            hand.to_hit = i32::from(d.hit_bonus);
            if holds[other] == Holds::Ammunition
                && let Some(o) = definition(other)
            {
                hand.to_hit += i32::from(o.hit_bonus);
            }
            if is_launcher(d.kind) {
                hand.damage_percent += i32::from(d.damage.base) * 10;
            }
            // Weapons only thrown, lashed or shot.
            if d.attack_modes.is_subset(AttackMode::Throw | AttackMode::Lash | AttackMode::Shoot) {
                if NO_STRENGTH.contains(&d.kind) {
                    strength_helps = false;
                } else {
                    strength_divisor = 2;
                }
            }
        }
        if strength_helps {
            if strength < 50 {
                hand.to_hit -= (50 - strength) / (strength_divisor * 10);
                hand.damage_percent -= (50 - strength) / strength_divisor;
            } else if strength > 50 {
                strength_divisor *= index as i32 + 1;
                hand.to_hit += (strength - 50) / (strength_divisor * 10);
                hand.damage_percent += (strength * 2 - 100) / strength_divisor;
            }
        }
        hand.to_hit += (dexterity - 50) / 10;
        if senses < 30 {
            hand.to_hit -= (30 - senses) / 10;
        } else if senses > 70 {
            hand.to_hit += (senses - 70) / 10;
        }
        let mut load_penalty = -load.penalty();
        if index == 1 {
            load_penalty = load_penalty * 3 / 2;
        }
        hand.to_hit += halved(load_penalty);
        if ranged && c.skill_available(Skill::EagleEye) {
            hand.to_hit += c.skill(Skill::EagleEye) as i32 / 20 + 1;
        }
        if !ranged && c.skill_available(Skill::PowerStrike) {
            hand.to_hit += c.skill(Skill::PowerStrike) as i32 / 20 + 1;
        }
        hand
    };
    Hands { hands: [hand(0), hand(1)], dual_wielding }
}

/// The main hand's numbers, as the character sheet shows them.
pub fn primary_hand(
    c: &CharacterStats<'_>,
    carried: &Carried,
    catalog: &ItemCatalog,
    load: Load,
) -> Hand {
    let [primary, _] = hands(c, carried, catalog, load).hands;
    primary
}

/// The character's initiative before the roll, weapon included.
pub fn character_initiative(
    c: &crate::game::characters::CharacterStats<'_>,
    hand: &Hand,
    load: Load,
) -> i32 {
    c.initiative(load) + hand.initiative
}
