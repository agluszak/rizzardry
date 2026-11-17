//! Where the party is and what holds it, as magic sees it
//! (`docs/rules/casting.md` section 2): figures on the party, written as
//! their sources change, which casting and the spell view read.
//!
//! Where the party is goes by the first that holds: in a fight; picking a
//! lock or disarming a trap (retail `SpellUsableNow`: the lock and trap
//! flags stop every other spell, a camp's included); talking with someone
//! (retail's dialogue offers only Charm and Mindread, and casts them under
//! the camp flag: `RequestNpcSpellService3`, `RequestNpcSpellService41`);
//! camped; else exploring. Looting what a lock guarded counts as the lock.

use bevy::prelude::*;

use crate::game::camp::Camp;
use crate::game::combat::Battle;
use crate::game::freedom::Freedom;
use crate::game::magic::may::UseContext;
use crate::game::npcs::conversation::DialogueReadOnly;
use crate::game::party::body::PartyMotion;
use crate::game::party::roster::PartyRoster;
use crate::game::world::interaction::{DeviceInteraction, Work};
use crate::game::world::props::PropPart;

/// The party is held by something: a conversation, a flight, work on a
/// device, a level being entered.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Occupied;

/// A conversation waits for the party's word.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct SpokenTo;

/// The party is not on steady ground: in the air, or standing on a prop
/// (retail `HasLevelWalkableContact`, the prop-contact flag).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Unsteady;

/// The party's figures follow what it is doing.
#[allow(clippy::type_complexity)]
pub fn follow(
    party: Single<
        (Entity, &mut UseContext, Has<Occupied>, Has<SpokenTo>, Has<Unsteady>),
        With<PartyRoster>,
    >,
    battle: Option<Res<Battle>>,
    camp: Option<Res<Camp>>,
    attempts: Query<&DeviceInteraction>,
    conversations: Query<DialogueReadOnly>,
    freedom: Freedom,
    footing: Option<Single<&bevy_tnua::TnuaController<PartyMotion>>>,
    parts: Query<(), With<PropPart>>,
    mut commands: Commands,
) {
    let (party, mut context, occupied, spoken_to, unsteady) = party.into_inner();
    let device = attempts.iter().next().map(|attempt| match attempt.work {
        Work::Disarming(_) => UseContext::Trap,
        // The game has kept looting what a lock guarded as work at the lock;
        // the original does not say what may be cast meanwhile.
        Work::Picking(_) | Work::Looting => UseContext::Lock,
    });
    let now = if battle.is_some() {
        UseContext::Combat
    } else if let Some(device) = device {
        device
    } else if !conversations.is_empty() {
        UseContext::Conversation
    } else if camp.is_some() {
        UseContext::Camp
    } else {
        UseContext::World
    };
    context.set_if_neq(now);
    let airborne_or_on_prop = footing.is_some_and(|controller| {
        let airborne = controller.is_airborne().unwrap_or(false);
        let on = controller.basis_access().ok().and_then(|b| b.memory.standing_on_entity());
        airborne || on.is_some_and(|on| parts.contains(on))
    });
    let mut party = commands.entity(party);
    mark::<Occupied>(&mut party, occupied, !freedom.unoccupied());
    mark::<SpokenTo>(&mut party, spoken_to, conversations.iter().any(|talk| talk.idle()));
    mark::<Unsteady>(&mut party, unsteady, airborne_or_on_prop);
}

/// A marker goes on or comes off when whether it holds changes.
fn mark<M: Component + Default>(party: &mut EntityCommands, has: bool, holds: bool) {
    if holds && !has {
        party.insert(M::default());
    } else if has && !holds {
        party.remove::<M>();
    }
}

pub fn plugin(app: &mut App) {
    app.register_required_components::<PartyRoster, UseContext>()
        .add_systems(crate::game::actors::Settle, follow.before(crate::game::actors::Written));
}
