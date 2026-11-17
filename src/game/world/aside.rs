//! Being set aside (`docs/rules/levels.md`): a thing taking no part in
//! anything, as Bevy's `Disabled` makes it, until it is woken. What a holder
//! has shares its state: each relationship whose held things go with their
//! holder says so where it is declared ([`follows`]). A level left, or a
//! companion out of the party, is only its root disabled; what it has
//! follows, and so does anything that comes to it while it is aside.

use bevy::ecs::entity_disabling::Disabled;
use bevy::ecs::query::Allow;
use bevy::ecs::relationship::{Relationship, RelationshipTarget};
use bevy::prelude::*;

/// What `R` holds is set aside and woken with its holder.
pub fn follows<R: Relationship>(app: &mut App) {
    app.add_observer(set_aside_with::<R>)
        .add_observer(woken_with::<R>)
        .add_observer(comes_to_one_aside::<R>);
}

/// A holder set aside sets aside what it holds.
fn set_aside_with<R: Relationship>(
    aside: On<Add<Disabled>>,
    holders: Query<Option<&R::RelationshipTarget>, Allow<Disabled>>,
    mut commands: Commands,
) -> Result {
    if let Some(held) = holders.get(aside.entity)? {
        for thing in held.iter() {
            commands.entity(thing).insert(Disabled);
        }
    }
    Ok(())
}

/// A holder woken wakes what it holds.
fn woken_with<R: Relationship>(
    woken: On<Remove<Disabled>>,
    holders: Query<Option<&R::RelationshipTarget>, Allow<Disabled>>,
    mut commands: Commands,
) -> Result {
    if let Some(held) = holders.get(woken.entity)? {
        for thing in held.iter() {
            commands.entity(thing).try_remove::<Disabled>();
        }
    }
    Ok(())
}

/// A thing that comes to a holder set aside is set aside with it.
fn comes_to_one_aside<R: Relationship>(
    held: On<Insert<R>>,
    things: Query<&R, Allow<Disabled>>,
    holders: Query<Has<Disabled>, Allow<Disabled>>,
    mut commands: Commands,
) -> Result {
    let holder = things.get(held.entity)?.get();
    if holders.get(holder)? {
        commands.entity(held.entity).insert(Disabled);
    }
    Ok(())
}
