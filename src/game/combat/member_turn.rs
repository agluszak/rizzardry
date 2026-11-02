//! A member's turn when their initiative comes ([`Acting`]): revalidated
//! ([`member_turns`]), then their action's system: an attack
//! ([`super::swings::member_attacks`]), a cast ([`start_member_cast`]) or a
//! breath ([`start_breath`]). Defending takes no turn.

use avian3d::prelude::Position;
use bevy::prelude::*;

use super::round::{Acting, PartyTurn, RoundAction};
use super::{ACTION_SECONDS, Battle, CombatNews, Combatants};
use crate::content::spells::SpellId;
use crate::game::data::Catalogs;
use crate::game::hits::prototypes::{Definitions, Uses};
use crate::game::hits::spells::reach;
use crate::game::hits::{Act, Body, Deed};
use crate::game::inventory::{Carried, EquipSlot, PartyInventory};
use crate::game::magic::casting::{
    Cast, CastBy, Chosen, MagicNews, Refusal, UseContext, candidates, in_cone,
};
use crate::game::party::body::PartyBody;
use crate::game::party::roster::PartyRoster;
use crate::game::world::level::Level;
use crate::game::world::obstacles::WorldObstacles;

/// The acting member's accepted turn, revalidated (retail
/// `ExecuteCharacterAction`): one no longer in the party or able, or whose
/// weapons changed since the attack was accepted, loses it; defending takes
/// no turn. An action still standing is its own system's to make.
pub(super) fn member_turns(
    mut commands: Commands,
    mut actors: Query<
        (Entity, &mut PartyTurn, &crate::game::characters::Vitals, &Carried),
        With<Acting>,
    >,
    roster: Single<&PartyRoster>,
) {
    for (member, mut turn, vitals, carried) in &mut actors {
        let able = roster.contains(&member)
            && vitals.can_fight()
            && match turn.action {
                // The weapons accepted, or nothing where one was used up.
                RoundAction::Attack { weapons, .. } => [EquipSlot::Primary, EquipSlot::Secondary]
                    .into_iter()
                    .zip(weapons)
                    .all(|(slot, accepted)| {
                        let now = carried.worn(slot).map(|item| item.id);
                        now.is_none() || now == accepted
                    }),
                RoundAction::Defend => false,
                RoundAction::Magic(_) | RoundAction::Breathe(_) => true,
            };
        if !able {
            turn.end();
            commands.entity(member).remove::<Acting>();
        }
    }
}

/// The acting member casts or uses an item: a cast that the cast stages
/// revalidate and roll; a point cast, which needs the whole world, at once.
/// A cast refused falls back to defending ([`defend_instead`]).
pub(super) fn start_member_cast(
    mut commands: Commands,
    mut actors: Query<(Entity, &mut PartyTurn, &Carried), With<Acting>>,
    mut battle: Single<&mut Battle>,
    inventory: Single<&PartyInventory>,
    content: If<Res<Catalogs>>,
    mut news: MessageWriter<MagicNews>,
) {
    for (member, mut turn, carried) in &mut actors {
        let RoundAction::Magic(choice) = turn.action else { continue };
        turn.end();
        commands.entity(member).remove::<Acting>();
        battle.wait = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
        let Some(spell) = choice.source.spell_id(carried, &inventory, &content.items) else {
            turn.action = RoundAction::Defend;
            news.write(MagicNews::Refused { caster: member, refusal: Refusal::NoItem });
            continue;
        };
        if crate::game::magic::pointcasts::spell(spell) {
            commands.queue(move |world: &mut World| {
                if crate::game::magic::execute_in_world(world, member, choice).is_err() {
                    world.get_mut::<PartyTurn>(member).unwrap().action = RoundAction::Defend;
                }
            });
            continue;
        }
        commands.spawn((
            Cast { spell, context: UseContext::Combat },
            Chosen(choice),
            CastBy(member),
        ));
    }
}

/// A member whose cast was refused defends instead (retail
/// `ExecuteCharacterAction`), the turn spent.
pub(super) fn defend_instead(mut news: MessageReader<MagicNews>, mut turns: Query<&mut PartyTurn>) {
    for news in news.read() {
        if let MagicNews::Refused { caster, .. } = *news
            && let Ok(mut turn) = turns.get_mut(caster)
            && matches!(turn.action, RoundAction::Magic(_))
        {
            turn.action = RoundAction::Defend;
        }
    }
}

/// The acting member breathes at their chosen target: an act reaching it
/// and whoever stands beside it in the cone (retail special-attack entry
/// 30), costing a fifth of their stamina. One who cannot breathe, or has
/// nobody in the cone, defends instead.
pub(super) fn start_breath(
    mut commands: Commands,
    mut actors: Query<(Entity, &mut PartyTurn), With<Acting>>,
    mut bodies: Query<Body>,
    battle: Single<(&mut Battle, Option<&Combatants>)>,
    stand: Single<&Position, With<PartyBody>>,
    levels: Query<(), With<Level>>,
    obstacles: WorldObstacles,
    content: If<Res<Catalogs>>,
    definitions: Res<Definitions>,
    mut news: MessageWriter<CombatNews>,
) {
    let (mut battle, combatants) = battle.into_inner();
    let combatants = combatants.map_or(&[][..], Combatants::entities);
    let eye = PartyBody::eye(*stand);
    for (member, mut turn) in &mut actors {
        let RoundAction::Breathe(target) = turn.action else { continue };
        turn.end();
        commands.entity(member).remove::<Acting>();
        battle.wait = Timer::from_seconds(ACTION_SECONDS, TimerMode::Once);
        let breather = bodies.get(member).unwrap();
        let character = breather.character.unwrap();
        let able = super::breath::available(character, breather.vitals);
        let strength =
            crate::game::hits::specials::strength(character.level(), breather.vitals.stamina);
        let cone = &content.spells[SpellId::SPECIAL_ATTACK_CONE];
        let monsters =
            candidates(eye, combatants, &mut bodies, &levels, &content.species, &obstacles);
        let aimed = monsters
            .iter()
            .find(|c| c.entity == target && c.distance <= cone.reach.metres())
            .copied();
        let victims: Vec<_> = aimed
            .map(|aimed| {
                monsters
                    .iter()
                    .filter(|c| {
                        c.distance <= cone.reach.metres() && in_cone(aimed.at - eye, c.at - eye)
                    })
                    .map(|c| (c.entity, None))
                    .collect()
            })
            .unwrap_or_default();
        if !able || victims.is_empty() {
            turn.action = RoundAction::Defend;
            continue;
        }
        let act =
            commands.spawn((Deed::new(Act::Fight), Uses(definitions.breath()), strength)).id();
        reach(&mut commands, act, &victims);
        let mut breather = bodies.get_mut(member).unwrap();
        let tired = breather.vitals.stamina.max / 5;
        breather.vitals.tire(tired);
        battle.attacked(true);
        news.write(CombatNews::Breathed { actor: member });
    }
}
