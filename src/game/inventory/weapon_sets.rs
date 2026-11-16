//! A member's two weapon sets: the one in use, which attacks, and the one
//! stored (`docs/rules/items.md`). Swapping them is one rule (retail
//! `SwapWeaponSetSlots`, `PC Item.cpp:4101`), asked for by the player, by a
//! Turncoat member who can strike nobody, and by the fight's own swap when
//! a member finds nobody to strike.

use bevy::prelude::*;

use super::EquipSlot;
use crate::game::characters::{Calling, Identity, Vitals};
use crate::game::items::{Cursed, Fit, ItemNews, Items, Known, Place, Put, hold_together};
use crate::game::party::roster::{Party, PartyRoster};

/// A curse holds a weapon it has in use (retail `CanUnequipSlotItem`): the
/// sets cannot be swapped while its wearer lives.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct WeaponsHeld;

/// Whether a curse holds a weapon in use follows what is worn there.
pub(crate) fn held(
    members: Query<(Entity, &Vitals, Has<WeaponsHeld>), With<Calling>>,
    items: Items,
    curses: Query<&Cursed>,
    mut commands: Commands,
) {
    for (member, vitals, was) in &members {
        let cursed = |slot| {
            items
                .worn(member, slot)
                .is_some_and(|item| curses.get(item).is_ok_and(|curse| !curse.released))
        };
        let now = !vitals.is_dead() && (cursed(EquipSlot::Primary) || cursed(EquipSlot::Secondary));
        if now && !was {
            commands.entity(member).insert(WeaponsHeld);
        } else if !now && was {
            commands.entity(member).remove::<WeaponsHeld>();
        }
    }
}

/// What the player hears of a swap of weapon sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapNews {
    /// Nothing in either set (retail string `0x1eb`).
    NothingToSwap,
    /// The sets swapped (`0x1ec`).
    Swapped,
    /// A curse holds a weapon in use (`0x1ee`).
    Held,
    /// A Turncoat member swapped to strike (`0x23d`).
    SwappedToStrike,
    /// The fight's own swap found someone to strike (`0x26d`).
    SwappedToFind,
}

/// A member swaps the weapon set in use for the stored one; `told` when the
/// player asked, who hears why it cannot be done.
#[derive(EntityEvent, Clone, Copy, Debug)]
pub struct SwapsWeaponSets {
    #[event_target]
    pub member: Entity,
    pub told: bool,
}

/// Each hand swaps its weapon for the stored one. A member with nothing in
/// either set has nothing to swap; a curse holding a weapon in use stops
/// the whole swap, and the party learns of it. A weapon in use that pairs
/// with the stored off hand while the stored main hand is empty stays (and
/// likewise the off hand with the stored main hand), so the set in use
/// still holds together.
#[allow(clippy::type_complexity)]
fn swap(
    swaps: On<SwapsWeaponSets>,
    members: Query<(&Identity, Has<WeaponsHeld>)>,
    items: Items,
    fit: Query<Fit>,
    mut commands: Commands,
    mut news: MessageWriter<ItemNews>,
) -> Result {
    let SwapsWeaponSets { member, told } = *swaps;
    let (identity, held) = members.get(member)?;
    let tell = |what| ItemNews::WeaponSets { member: identity.name.clone(), what };
    let worn = |slot| items.worn(member, slot);
    let [primary, secondary, stored_primary, stored_secondary] = [
        EquipSlot::Primary,
        EquipSlot::Secondary,
        EquipSlot::AlternatePrimary,
        EquipSlot::AlternateSecondary,
    ]
    .map(worn);
    if [primary, secondary, stored_primary, stored_secondary].iter().all(Option::is_none) {
        if told {
            news.write(tell(SwapNews::NothingToSwap));
        }
        return Ok(());
    }
    if held {
        for item in [primary, secondary].into_iter().flatten() {
            commands.entity(item).entry::<Known>().and_modify(|mut known| known.curse = true);
        }
        if told {
            news.write(tell(SwapNews::Held));
        }
        return Ok(());
    }
    let pairs = |a: Option<Entity>, b: Option<Entity>| match (a, b) {
        (Some(a), Some(b)) => Ok(hold_together(&fit.get(a)?, &fit.get(b)?)),
        _ => Ok::<bool, BevyError>(false),
    };
    let keeps_primary = stored_primary.is_none() && pairs(primary, stored_secondary)?;
    let keeps_secondary = stored_secondary.is_none() && pairs(secondary, stored_primary)?;
    let exchange =
        |slot: EquipSlot, stored: EquipSlot, (now, there): (Option<Entity>, Option<Entity>)| {
            let mut moves = Vec::new();
            if let Some(item) = now {
                moves.push(Put { item, at: Place::Slot { member, slot: stored } });
            }
            if let Some(item) = there {
                moves.push(Put { item, at: Place::Slot { member, slot } });
            }
            moves
        };
    let mut moves = Vec::new();
    if !keeps_primary {
        let primaries = (primary, stored_primary);
        moves.extend(exchange(EquipSlot::Primary, EquipSlot::AlternatePrimary, primaries));
    }
    if !keeps_secondary {
        let secondaries = (secondary, stored_secondary);
        moves.extend(exchange(EquipSlot::Secondary, EquipSlot::AlternateSecondary, secondaries));
    }
    for put in moves {
        commands.trigger(put);
    }
    if told {
        news.write(tell(SwapNews::Swapped));
    }
    Ok(())
}

/// The player asks a member to swap weapon sets.
#[derive(Message, Clone, Copy, Debug)]
pub struct SwapWeapons(pub Entity);

/// The player asks every member to swap weapon sets.
#[derive(Message, Clone, Copy, Debug)]
pub struct SwapAllWeapons;

/// The player's swap (retail `BindCharacterItems`, `BindEveryPartyItem`,
/// `PC Item.cpp:4175`, `:2578`): in a fight only while a round is played or
/// the party moves, else refused (string `0x1f6`). A member's swap says how
/// it went; everyone's swaps each quietly and says it once (`0x1ed`). What
/// the fight's own swap left in use is the player's choice from then on.
fn swap_as_asked(
    mut one: MessageReader<SwapWeapons>,
    mut all: MessageReader<SwapAllWeapons>,
    fight: Option<Single<(&crate::game::combat::Round, &crate::game::combat::PartyMove)>>,
    roster: Single<&PartyRoster, With<Party>>,
    mut news: MessageWriter<ItemNews>,
    mut commands: Commands,
) {
    let asked: Vec<Entity> = one.read().map(|&SwapWeapons(member)| member).collect();
    let everyone = all.read().count() > 0;
    if asked.is_empty() && !everyone {
        return;
    }
    let between_rounds = fight.is_some_and(|fight| {
        let (round, party_move) = *fight;
        !round.executing() && !matches!(party_move, crate::game::combat::PartyMove::Moving { .. })
    });
    if between_rounds {
        news.write(ItemNews::SwapsBlocked);
        return;
    }
    let swap = |member: Entity, told: bool, commands: &mut Commands| {
        commands.entity(member).remove::<crate::game::combat::other_set::SwappedForFight>();
        commands.trigger(SwapsWeaponSets { member, told });
    };
    for member in asked.into_iter().filter(|member| roster.contains(member)) {
        swap(member, true, &mut commands);
    }
    if everyone {
        for member in roster.iter() {
            swap(member, false, &mut commands);
        }
        news.write(ItemNews::AllSwapped);
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.add_message::<SwapWeapons>()
        .add_message::<SwapAllWeapons>()
        .add_observer(swap)
        .add_systems(Update, swap_as_asked)
        .add_systems(crate::game::actors::Settle, held.in_set(crate::game::actors::Written));
}
