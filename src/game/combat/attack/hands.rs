//! What a member fights with and how well. [`Grip`] is what the two hands
//! hold; [`Fighter`] answers what each moment of a fight asks of it. Retail
//! caches these answers on the character (`CalcAttacks`); nothing here is stored.
use super::*;

/// What a hand holds (retail `SetHandType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Holds {
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

/// The combat skill a weapon skill trains under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Close,
    Ranged,
}

impl Style {
    fn of(skill: Skill) -> Option<Self> {
        match skill {
            Skill::ModernWeapon | Skill::Bow | Skill::ThrowingAndSling => Some(Self::Ranged),
            skill if skill as usize <= Skill::Shield as usize || skill == Skill::MartialArts => {
                Some(Self::Close)
            }
            _ => None,
        }
    }
}

/// What an item in a hand amounts to (retail `SetHandType`). An omnigun's
/// off hand takes a thrown weapon as its ammunition.
fn holds(item: Option<&ItemDefinition>, beside_omnigun: bool) -> Holds {
    let Some(item) = item else { return Holds::Nothing };
    match item.class {
        EquipClass::ShortWeapon | EquipClass::ExtendedWeapon | EquipClass::RangedWeapon => {
            Holds::Weapon
        }
        EquipClass::ThrownWeapon if beside_omnigun => Holds::Ammunition,
        EquipClass::ThrownWeapon => Holds::Weapon,
        EquipClass::Ammunition => Holds::Ammunition,
        EquipClass::Shield => Holds::Shield,
        _ => Holds::Other,
    }
}

/// What a member's two hands hold and which of them can attack. Hand 0 is
/// the main hand. It reads the items as they are when taken: an attack keeps
/// its grip though it shoots away what it holds.
#[derive(Clone, Copy, Debug)]
pub struct Grip<'a> {
    items: [Option<&'a ItemDefinition>; 2],
    holds: [Holds; 2],
    skills: [Option<Skill>; 2],
    in_play: [bool; 2],
}

impl<'a> Grip<'a> {
    /// An off hand attacks only beside a main hand of the same style, never
    /// with a shield or ammunition or beside an omnigun, and an empty one
    /// only beside another. A ranged hand cannot attack with a launcher
    /// lacking its ammunition or a gun out of shots.
    pub fn of(carried: &Carried, catalog: &'a ItemCatalog) -> Self {
        let stacks = [EquipSlot::Primary, EquipSlot::Secondary].map(|slot| carried.worn(slot));
        let items = stacks.map(|stack| stack.map(|stack| &catalog[stack.item]));
        let omnigun = items[0].is_some_and(|item| item.kind == ItemKind::OMNIGUN);
        let holds = [holds(items[0], false), holds(items[1], omnigun)];
        let skills = items.map(|item| item.map_or(Some(Skill::MartialArts), |d| d.weapon_skill));
        let styles = skills.map(|skill| skill.and_then(Style::of));
        let loaded = |hand: usize| {
            let (Some(stack), Some(item)) = (stacks[hand], items[hand]) else { return true };
            let partner = items[1 - hand];
            (!is_launcher(item.kind)
                || partner.is_some_and(|o| crate::game::inventory::partners(item, o)))
                && !(item.quantity == QuantityKind::Shots && stack.remaining_uses() == Some(0))
        };
        let able = |hand: usize| match styles[hand] {
            None => false,
            Some(Style::Close) => true,
            Some(Style::Ranged) => loaded(hand),
        };
        let off_hand = styles[0] == styles[1]
            && !matches!(holds[1], Holds::Shield | Holds::Ammunition)
            && !omnigun
            && (holds[1] != Holds::Nothing || holds[0] == Holds::Nothing);
        Self { items, holds, skills, in_play: [able(0), able(1) && off_hand] }
    }

    pub fn in_play(&self, hand: usize) -> bool {
        self.in_play[hand]
    }

    pub fn weapon_skill(&self, hand: usize) -> Skill {
        self.skills[hand].unwrap_or(Skill::MartialArts)
    }

    fn style(&self, hand: usize) -> Option<Style> {
        self.skills[hand].and_then(Style::of)
    }

    /// Its blows are ranged combat, not close combat.
    pub fn ranged(&self, hand: usize) -> bool {
        self.style(hand) == Some(Style::Ranged)
    }

    pub fn reach(&self, hand: usize) -> Reach {
        match (self.holds[hand], self.items[hand]) {
            (Holds::Weapon | Holds::Ammunition, Some(item)) => item.reach,
            _ => Reach::Touch,
        }
    }

    /// It shoots ammunition from the other hand.
    pub fn paired(&self, hand: usize) -> bool {
        self.in_play[hand] && self.items[hand].is_some_and(|item| is_launcher(item.kind))
    }

    fn ammunition(&self, hand: usize) -> Option<&'a ItemDefinition> {
        if self.paired(hand) { self.items[1 - hand] } else { None }
    }

    /// How many of the monster's kinds its weapon and ammunition slay.
    pub fn slays(&self, hand: usize, kind: crate::content::species::MonsterKind) -> u32 {
        [self.items[hand], self.ammunition(hand)]
            .iter()
            .flatten()
            .filter(|item| item.slays == Some(kind))
            .count() as u32
    }

    /// Swings the weapon adds to each attack.
    pub fn extra_swings(&self, hand: usize) -> i32 {
        self.items[hand].map_or(0, |item| item.extra_swings)
    }

    /// What swinging it weighs, for its stamina.
    pub fn weight(&self, hand: usize) -> Option<u16> {
        self.items[hand].map(|item| item.weight)
    }

    fn kind(&self, hand: usize) -> Option<ItemKind> {
        self.items[hand].map(|item| item.kind)
    }

    /// A sling, which throws by the rules of a bow.
    pub fn sling(&self, hand: usize) -> bool {
        self.kind(hand) == Some(ItemKind::SLING)
    }

    fn dual_wielding(&self) -> bool {
        self.holds == [Holds::Weapon, Holds::Weapon] && self.in_play == [true, true]
    }
}

/// What a hand strikes with, even one that cannot attack itself: a punch or
/// a kick, or the shot a launcher looses.
#[derive(Clone, Copy, Debug)]
pub struct Strike {
    pub dice: Dice,
    /// The ways it strikes; one is chosen per attack.
    pub modes: EnumSet<AttackMode>,
    /// What its blows may do besides damage.
    pub effects: AttackEffects,
}

/// A member with what they hold, as a moment of a fight finds them.
#[derive(Clone, Copy)]
pub struct Fighter<'a> {
    pub stats: &'a CharacterStats<'a>,
    pub grip: &'a Grip<'a>,
    load: Load,
    /// The party's blessings, which guide every hand.
    blessing: i32,
}

impl<'a> Fighter<'a> {
    pub fn new(stats: &'a CharacterStats<'a>, grip: &'a Grip<'a>, load: Load) -> Self {
        Self { stats, grip, load, blessing: 0 }
    }

    pub fn blessed(self, blessing: i32) -> Self {
        Self { blessing, ..self }
    }

    fn attribute(&self, attribute: Attribute) -> i32 {
        self.stats.attribute(attribute) as i32
    }

    /// The hand an attack opens with (retail `ChooseCharacterAttackHand`):
    /// the most attacks, then the bigger initiative, then the main hand.
    pub fn opening_hand(&self) -> Option<usize> {
        let in_play = (0..2).rev().filter(|&hand| self.grip.in_play[hand]);
        in_play.max_by_key(|&hand| (self.attacks(hand), self.initiative(hand)))
    }

    pub fn strike(&self, hand: usize) -> Strike {
        match self.grip.items[hand] {
            Some(weapon) => {
                let ammunition = self.grip.ammunition(hand);
                Strike {
                    dice: ammunition.unwrap_or(weapon).damage,
                    modes: weapon.attack_modes,
                    effects: ammunition.map_or(weapon.effects, |a| weapon.effects.with(&a.effects)),
                }
            }
            None => self.fist(hand),
        }
    }

    /// An empty hand punches, kicks too past Martial Arts 4, and may knock
    /// out by strength. The main hand hits 2 harder.
    fn fist(&self, hand: usize) -> Strike {
        let martial_arts = self.stats.skill(self.grip.weapon_skill(hand));
        let strength = self.attribute(Attribute::Strength);
        let mut dice = unarmed_dice(martial_arts);
        dice.base += if hand == 0 { 2 } else { 0 };
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

    /// The skills a hand fights by: its weapon's with its combat skill, the
    /// other weapon's and Dual Weapons.
    fn prowess(&self, hand: usize) -> i32 {
        let (c, grip) = (self.stats, self.grip);
        let combat = if grip.ranged(hand) { Skill::RangedCombat } else { Skill::CloseCombat };
        let mut score = ((c.skill(combat) + c.skill(grip.weapon_skill(hand)) * 2) * 2 / 3) as i32;
        let mut divisor = 20;
        let other = 1 - hand;
        if grip.holds[other] == Holds::Weapon && grip.in_play[other] {
            score += c.skill(grip.weapon_skill(other)) as i32 >> 1;
            divisor = 25;
        }
        if grip.dual_wielding() {
            score += c.skill(Skill::DualWeapons) as i32;
            divisor += 10;
        }
        score * 10 / divisor
    }

    /// Modern weapons feel load half as much.
    fn load_felt(&self, hand: usize, penalty: i32) -> i32 {
        if self.grip.weapon_skill(hand) == Skill::ModernWeapon { penalty / 2 } else { penalty }
    }

    fn dual_penalty(&self, hand: usize) -> i32 {
        if !self.grip.dual_wielding() {
            return 0;
        }
        -10 * (hand as i32 + 1) - (100 - self.stats.skill(Skill::DualWeapons) as i32) / 4
    }

    /// What every speed of a hand shares.
    fn drill(&self, hand: usize) -> i32 {
        self.dual_penalty(hand)
            + self.stats.physical_levels()
            + self.load_felt(hand, -self.load.attack_penalty())
            + self.prowess(hand)
    }

    /// Attacks in a round.
    pub fn attacks(&self, hand: usize) -> u32 {
        if !self.grip.in_play[hand] {
            return 0;
        }
        let quickness =
            (self.attribute(Attribute::Speed) + self.attribute(Attribute::Dexterity)) >> 1;
        let score = (quickness + self.drill(hand)) / 3;
        match hand {
            0 => 1 + u32::from(score > 49) + u32::from(score > 99),
            _ => 1 + u32::from(score > 74),
        }
    }

    /// The weapon's initiative bonus (retail `damage_bonus`).
    pub fn initiative(&self, hand: usize) -> i32 {
        if !self.grip.in_play[hand] {
            return 0;
        }
        self.grip.items[hand].map_or(self.prowess(hand) / 10, |item| item.damage_bonus)
    }

    /// The character's initiative before the roll, weapon included.
    pub fn round_initiative(&self, hand: usize) -> i32 {
        self.stats.initiative(self.load) + self.initiative(hand)
    }

    /// The most swings an attack rolls for.
    pub fn swings(&self, hand: usize) -> u32 {
        if !self.grip.in_play[hand] {
            return 0;
        }
        if self.grip.kind(0) == Some(ItemKind::ROCKET_LAUNCHER) {
            return 1;
        }
        // Retail reads the initiative its previous pass left, which is this.
        let speed = self.attribute(Attribute::Speed) + self.initiative(hand) * 10;
        let score = (speed + self.drill(hand)) / 3;
        1 + u32::from(score > 66) + u32::from(score > 99)
    }

    fn attack_rating(&self, hand: usize) -> i32 {
        let dexterity = self.attribute(Attribute::Dexterity);
        let physical = self.stats.physical_levels();
        (self.dual_penalty(hand) + dexterity / 2 + self.prowess(hand) * 2 + physical) / 3 + 60
    }

    /// What strength adds to a hand's to-hit and damage percent. Weapons
    /// only thrown, lashed or shot feel half of it, or none if they carry
    /// their own force; an off hand gains half as much from it.
    fn strength_bonus(&self, hand: usize) -> (i32, i32) {
        let item = self.grip.items[hand];
        let distant = item.filter(|item| {
            item.attack_modes.is_subset(AttackMode::Throw | AttackMode::Lash | AttackMode::Shoot)
        });
        if distant.is_some_and(|item| NO_STRENGTH.contains(&item.kind)) {
            return (0, 0);
        }
        let divisor = if distant.is_some() { 2 } else { 1 };
        let strength = self.attribute(Attribute::Strength);
        if strength < 50 {
            (-((50 - strength) / (divisor * 10)), -((50 - strength) / divisor))
        } else if strength > 50 {
            let divisor = divisor * (hand as i32 + 1);
            ((strength - 50) / (divisor * 10), (strength * 2 - 100) / divisor)
        } else {
            (0, 0)
        }
    }

    fn senses_bonus(&self) -> i32 {
        let senses = self.attribute(Attribute::Senses);
        match senses {
            ..30 => -((30 - senses) / 10),
            71.. => (senses - 70) / 10,
            _ => 0,
        }
    }

    /// The sheet's "To Hit": weapon and ammunition, strength, dexterity,
    /// senses, load and trained strikes.
    pub fn to_hit(&self, hand: usize) -> i32 {
        if !self.grip.in_play[hand] {
            return 0;
        }
        let (c, grip) = (self.stats, self.grip);
        let other = 1 - hand;
        let shot = grip.items[other].filter(|_| grip.holds[other] == Holds::Ammunition);
        let weapon = grip.items[hand]
            .map_or(0, |item| item.hit_bonus + shot.map_or(0, |ammunition| ammunition.hit_bonus));
        let load = -self.load.penalty();
        let load = if hand == 1 { load * 3 / 2 } else { load };
        let trained = if grip.ranged(hand) { Skill::EagleEye } else { Skill::PowerStrike };
        let trained = if c.skill_available(trained) { c.skill(trained) as i32 / 20 + 1 } else { 0 };
        weapon
            + self.strength_bonus(hand).0
            + (self.attribute(Attribute::Dexterity) - 50) / 10
            + self.senses_bonus()
            + self.load_felt(hand, load)
            + trained
    }

    /// The sheet's "To Damage (%)": strength, and a launcher's own base.
    pub fn damage_percent(&self, hand: usize) -> i32 {
        if !self.grip.in_play[hand] {
            return 0;
        }
        let launcher = self.grip.items[hand].filter(|item| is_launcher(item.kind));
        self.strength_bonus(hand).1 + launcher.map_or(0, |item| item.damage.base * 10)
    }

    /// The score a hand's blows start from (retail `attack_score` and
    /// `hit_bonus`), blessings included.
    pub fn score(&self, hand: usize) -> i32 {
        self.attack_rating(hand) + (self.to_hit(hand) + self.blessing) * 5
    }
}
