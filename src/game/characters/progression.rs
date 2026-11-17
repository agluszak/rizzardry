//! What a level brings ([`super::draft`] spends it): the spell picks saved,
//! the experience goal, and who is ready.
use super::*;
use crate::content::spells::Spellbook;
use crate::game::magic;
use enumset::EnumSet;

/// Saved picks retain their school when changing profession. Bishop awards
/// are unrestricted, matching skill_costs and magic_bonus_pool in retail.
#[derive(bevy::prelude::Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct SpellPicks {
    books: EnumMap<Spellbook, usize>,
    general: usize,
}
impl SpellPicks {
    pub fn total(&self) -> usize {
        self.books.values().sum::<usize>() + self.general
    }
    pub(super) fn grant(&mut self, profession: Profession, amount: usize) {
        if profession.row().picks_from_any_book {
            self.general += amount;
        } else if let Some(book) = profession.row().books.iter().next() {
            self.books[book] += amount;
        }
    }
    /// The picks one following their profession may spend now: none below
    /// caster level 1.
    pub fn available(&self, calling: &super::Calling) -> usize {
        if magic::caster_level(calling, calling.profession) < 1 {
            return 0;
        }
        self.general
            + calling.profession.row().books.iter().map(|book| self.books[book]).sum::<usize>()
    }
    /// A pick is charged to a spell of these books: to one of the
    /// profession's books with picks, else to the general picks, else, for a
    /// profession that picks from any book, to any of its books.
    pub fn spend(&mut self, calling: &super::Calling, books: EnumSet<Spellbook>) -> bool {
        let active = calling.profession.row().books;
        if let Some(book) = (active & books).iter().find(|&b| self.books[b] > 0) {
            self.books[book] -= 1;
        } else if self.general > 0 {
            self.general -= 1;
        } else if calling.profession.row().picks_from_any_book {
            let Some(book) = active.iter().find(|&b| self.books[b] > 0) else {
                return false;
            };
            self.books[book] -= 1;
        } else {
            return false;
        }
        true
    }
}

/// CalcXPGoal adds the current profession's increment to the threshold
/// actually passed; switching profession never reprices previous levels.
pub(super) fn experience_increment(profession: Profession, level: u8) -> u64 {
    let weight = profession.row().experience;
    if level == 1 {
        return weight;
    }
    let mut value = weight << (level.min(10) - 2);
    for _ in 10..level {
        value = value * 12 / 10;
    }
    value
}
pub fn ready(calling: &super::Calling, experience: &super::Experience, v: &Vitals) -> bool {
    // Retail offers a level past 50 and then clamps it away; it is not
    // offered here.
    !v.is_dead()
        && calling.level() < super::MAX_LEVEL
        && experience.earned >= calling.experience_goal(experience)
}
