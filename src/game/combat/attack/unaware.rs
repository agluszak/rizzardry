//! Catching the target unaware (`docs/rules/attack.md` section 3).

use crate::game::rng::GameRng;

/// Whether a member who has tried `tries` times already this round notices:
/// certain the first time, then with chance `Senses - 25 * tries` percent
/// (retail `CharacterNoticesAttacker`, which compares it as an unsigned
/// number: once it falls below 0, noticing is certain).
pub fn notices(rng: &mut GameRng, senses: u32, tries: u32) -> bool {
    if tries == 0 {
        return true;
    }
    let chance = senses as i32 - tries as i32 * 25;
    chance < 0 || rng.below(100) < chance as u32
}
