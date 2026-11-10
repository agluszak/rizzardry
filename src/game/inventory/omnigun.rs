//! The Gadgeteer's omnigun, which grows with its owner's Gadgeteer level as
//! their advancement is committed (retail `UpgradeProfessionClassItem`).

use bevy::prelude::*;

use crate::content::characters::{Profession, Trait};
use crate::content::items::{ItemKind, WEAPONS};
use crate::game::characters::Character;
use crate::game::data::Catalogs;
use crate::game::items::{
    Charges, Give, Item, ItemNews, Items, Kind, Known, Weight, make, roll_amount, shape,
};
use crate::game::party::roster::PartyRoster;
use crate::game::rng::GameRng;

pub(crate) fn plugin(app: &mut App) {
    app.add_observer(advance);
}

/// A member's advancement has just been committed (retail
/// `UpgradeProfessionClassItem`, from `CommitCharacter`). A level 1
/// Gadgeteer without an omnigun of their own is given one, to them or else
/// the party, unless neither has room. From level 3 the first omnigun found
/// (theirs, worn or carried, else the hand's, each member's in turn, then the
/// pool's) rises to the tier their level earns, never down. The party is
/// told.
#[derive(Event, Clone, Copy, Debug)]
pub struct Advanced {
    pub party: Entity,
    pub member: Entity,
}

fn advance(
    event: On<Advanced>,
    items: Items,
    kinds: Query<&Kind>,
    characters: Query<&Character>,
    rosters: Query<&PartyRoster>,
    content: Res<Catalogs>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<ItemNews>,
    mut commands: Commands,
) {
    let Advanced { party, member } = *event;
    let (Ok(character), Ok(roster)) = (characters.get(member), rosters.get(party)) else {
        return;
    };
    if !character.has_trait(Trait::MergeGadgets) {
        return;
    }
    let omnigun = |&item: &Entity| kinds.get(item).is_ok_and(|kind| kind.0 == ItemKind::OMNIGUN);
    let carries = |owner: Entity| items.carried(owner).find(omnigun);
    let told =
        |tier| ItemNews::Omnigun { member: character.name.clone(), gender: character.gender, tier };
    let level = character.profession_levels[Profession::Gadgeteer];
    if level == 1 {
        let definition = &content.items[WEAPONS.omnigun.first];
        if carries(member).is_some() {
            return;
        }
        let room = items.pack_room(member, definition.id, true, 1)
            + items.pool_room(party, definition.id, true, 1);
        if room == 0 {
            return;
        }
        let given = make(&mut commands, definition, roll_amount(definition, &mut rng, false), true);
        commands.entity(given).insert(Known { identified: true, spell: false, curse: true });
        commands.trigger(Give {
            item: given,
            to: Some(member),
            party,
            pool_first: false,
            told: true,
        });
        news.write(told(0));
        return;
    }
    let Some((id, tier)) = WEAPONS.omnigun.tier(level) else { return };
    let current = carries(member)
        .or_else(|| items.held(party).filter(omnigun))
        .or_else(|| roster.iter().find_map(carries))
        .or_else(|| items.pool(party).iter().copied().find(omnigun));
    let Some(current) = current.filter(|&current| items.id(current).is_some_and(|now| now < id))
    else {
        return;
    };
    // The same gun, grown: what it is made of is read again from the
    // definition it has risen to.
    let definition = &content.items[id];
    commands.entity(current).insert((Item(id), Weight(definition.weight), Kind(definition.kind)));
    shape(&mut commands.entity(current), definition);
    let full = definition.initial_quantity.maximum() as u16;
    commands.entity(current).entry::<Charges>().and_modify(move |mut charges| charges.full = full);
    if let Some(bearer) = items.bearer(current) {
        commands.trigger(crate::game::actors::Refit { entity: bearer });
    }
    news.write(told(tier));
}
