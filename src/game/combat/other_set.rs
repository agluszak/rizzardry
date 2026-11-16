//! A member's strike, and the other weapon set (`docs/rules/monsters.md`
//! section 8, `docs/rules/items.md`). A member whose strike finds nobody
//! tries their other set when Turncoat holds them (retail
//! `GetConditionInterrupt`, `Combat.cpp:3030`) or when the fight's own swap
//! is on (retail `ChooseFallbackMonsterTarget`, `Targeting.cpp:3340`): the
//! sets swap, and once their arms follow they take up their attacks anew and
//! strike again; finding nobody again, the sets swap back.

use bevy::prelude::*;

use super::action::{Fights, Interrupt};
use super::swings::{AmongFellows, AttackBy, Choosing, Found, Nobody, Seeks, begin};
use super::turn::{Berserk, Done, Shown, TakesUpAttacks};
use super::{Battle, CombatNews, CombatSettings};
use crate::game::characters::{Condition, Conditions, Identity, Vitals};
use crate::game::inventory::EquipSlot;
use crate::game::inventory::weapon_sets::{SwapNews, SwapsWeaponSets, WeaponsHeld};
use crate::game::items::{ItemNews, Items};
use crate::game::world::hostility::Stance;

/// A member waiting for their arms to follow the weapon set they swapped
/// to, to strike again as they meant to.
#[derive(Component, Clone, Copy, Debug)]
pub struct OtherSet(pub Fights);

/// This moment's strike is with the set swapped to.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct WithOtherSet;

/// The fight's own swap left the stored set in use: it swaps back as the
/// fight ends (retail `weapon_swap_pending`).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct SwappedForFight;

/// A member strikes: at their friends when berserk, else at their foes; one
/// turned or berserk only among their fellow members. Set on every member
/// as they are made.
pub(crate) fn member_strikes(
    strike: On<super::swings::Strike>,
    mut commands: Commands,
    members: Query<(Has<Berserk>, &Conditions, Has<super::turn::AttacksLeft>)>,
) -> Result {
    let super::swings::Strike { actor, fights } = *strike;
    let (berserk, conditions, has_attacks) = members.get(actor)?;
    // One ordered to cast or breathe took up no attacks: a strike their
    // ailments forced takes them up now (retail `PrepareCharacterAttacks`,
    // `Combat.cpp:2224`).
    if !has_attacks {
        commands.trigger(TakesUpAttacks { actor });
    }
    let looks_for = if berserk { Stance::Friendly } else { Stance::Hostile };
    let attack = begin(&mut commands, actor, fights, looks_for);
    let mut made = commands.entity(attack);
    if berserk || conditions.has(Condition::Turncoat) {
        made.insert(AmongFellows);
    }
    made.observe(found).observe(nobody);
    commands.trigger(Seeks { attack });
    Ok(())
}

/// A member found someone to strike. One Turncoat holds is told so
/// (interrupt 9). With the set swapped to, they say so: a Turncoat that
/// they swapped to strike, otherwise that the fight's swap found someone,
/// which stays in use until the fight ends.
fn found(
    found: On<Found>,
    mut commands: Commands,
    attacks: Query<&AttackBy>,
    members: Query<(&Identity, &Conditions, Has<WithOtherSet>, Has<SwappedForFight>)>,
    mut news: MessageWriter<CombatNews>,
    mut items: MessageWriter<ItemNews>,
) -> Result {
    let &AttackBy(actor) = attacks.get(found.attack)?;
    let (identity, conditions, with_other, swapped) = members.get(actor)?;
    let turned = conditions.has(Condition::Turncoat);
    if with_other {
        let what = if turned { SwapNews::SwappedToStrike } else { SwapNews::SwappedToFind };
        items.write(ItemNews::WeaponSets { member: identity.name.clone(), what });
        let mut member = commands.entity(actor);
        member.remove::<WithOtherSet>();
        if !turned {
            match swapped {
                true => member.remove::<SwappedForFight>(),
                false => member.insert(SwappedForFight),
            };
        }
    }
    if turned {
        news.write(CombatNews::Interrupted { actor, interrupt: Interrupt::Turncoat });
    }
    Ok(())
}

/// A member's strike found nobody. One whose eye missed someone it could
/// have struck is done. With the set swapped to, the sets swap back. One
/// Turncoat holds, or with the fight's swap on, who has weapons to swap and
/// no curse holding one in use, swaps and waits for their arms. One
/// Turncoat holds who can strike nobody loses the action (interrupt 10).
#[allow(clippy::type_complexity)]
fn nobody(
    nobody: On<Nobody>,
    mut commands: Commands,
    attacks: Query<(&AttackBy, &Choosing)>,
    members: Query<(&Conditions, Has<WithOtherSet>, Has<WeaponsHeld>)>,
    settings: Res<CombatSettings>,
    items: Items,
    mut news: MessageWriter<CombatNews>,
) -> Result {
    let attack = nobody.attack;
    let (&AttackBy(actor), &Choosing(fights)) = attacks.get(attack)?;
    commands.entity(attack).despawn();
    let (conditions, with_other, held) = members.get(actor)?;
    let turned = conditions.has(Condition::Turncoat);
    let hands = [
        EquipSlot::Primary,
        EquipSlot::Secondary,
        EquipSlot::AlternatePrimary,
        EquipSlot::AlternateSecondary,
    ];
    let has_weapons = hands.into_iter().any(|slot| items.worn(actor, slot).is_some());
    if with_other {
        commands.entity(actor).remove::<WithOtherSet>();
        commands.trigger(SwapsWeaponSets { member: actor, told: false });
    } else if !nobody.evaded && (turned || settings.autoswap_weapons) && has_weapons && !held {
        commands.trigger(SwapsWeaponSets { member: actor, told: false });
        commands.entity(actor).insert(OtherSet(fights));
        return Ok(());
    }
    if turned {
        news.write(CombatNews::Interrupted { actor, interrupt: Interrupt::CannotStrike });
        commands.trigger(Shown { actor });
    }
    commands.trigger(Done { actor });
    Ok(())
}

/// Once a member's arms follow the set swapped to, they take up their
/// attacks anew (retail `PrepareCharacterAttacks`) and strike again.
pub(super) fn strike_with_other_set(waiting: Query<(Entity, &OtherSet)>, mut commands: Commands) {
    for (member, &OtherSet(fights)) in &waiting {
        commands.entity(member).remove::<OtherSet>().insert(WithOtherSet);
        commands.trigger(TakesUpAttacks { actor: member });
        commands.trigger(super::swings::Strike { actor: member, fights });
    }
}

/// As the fight ends, the fight's swap is undone for each living member it
/// left on the stored set (retail `ReconcilePartyEquipmentAfterCombat`).
fn swap_back(
    _: On<Remove<Battle>>,
    mut commands: Commands,
    members: Query<(Entity, &Vitals), With<SwappedForFight>>,
    settings: Res<CombatSettings>,
) {
    for (member, vitals) in &members {
        commands.entity(member).remove::<SwappedForFight>();
        if settings.autoswap_weapons && !vitals.is_dead() {
            commands.trigger(SwapsWeaponSets { member, told: false });
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_observer(swap_back);
}
