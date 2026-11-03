//! Commit script effects through their owning data. Each operation is complete
//! before the interpreter resumes; none stores a second pending-effect queue.
use super::interaction;
use super::*;
use crate::content::{items::ItemId, npcs::FactId};
use crate::game::{
    characters::{Character, Vitals},
    data::Catalogs,
    factions::{FactionNews, Factions},
    inventory::{Carried, ItemIds, ItemNews, PartyInventory, make_item},
    npcs::{Npc, access::NpcActors},
    party::{body::PartyBody, roster::PartyRoster},
    rng::GameRng,
    time::GameClock,
    world::level::Level,
};
use avian3d::prelude::Position;

pub(super) enum Practice {
    Greeting,
    Member(Entity, u32),
}
pub(super) fn practice(
    In(practice): In<Practice>,
    roster: Single<&PartyRoster>,
    mut members: Query<(&mut Character, &Vitals)>,
    mut rng: ResMut<GameRng>,
) {
    for member in roster.iter() {
        let Ok((mut character, vitals)) = members.get_mut(member) else { continue };
        let usage = match practice {
            Practice::Greeting if vitals.can_fight() => 0xf,
            Practice::Member(who, usage) if who == member => usage,
            _ => continue,
        };
        character.practise(crate::content::characters::Skill::Communication, usage, &mut rng);
    }
}

pub(super) fn set_fact(
    In((id, value)): In<(FactId, u8)>,
    mut story: ResMut<StoryFacts>,
    content: Res<Catalogs>,
    level: Single<&Level>,
) {
    story.set(id, value, &content.facts, level.path.clone()).expect("validated script fact");
}
pub(super) fn mention(
    In((entity, category, text)): In<(Entity, TopicCategory, String)>,
    mut conversations: Query<&mut Conversation>,
    mut story: ResMut<StoryFacts>,
) {
    if let Ok(mut c) = conversations.get_mut(entity) {
        c.learn(&mut story, category, &text);
    }
}
pub(super) fn disposition(In((actor, delta)): In<(Entity, i8)>, mut npcs: Query<&mut Npc>) {
    if let Ok(mut npc) = npcs.get_mut(actor) {
        interaction::shift(&mut npc, delta);
    }
}
pub(super) fn resume(
    In(entity): In<Entity>,
    conversations: Query<&Conversation>,
    actors: NpcActors,
    mut npcs: Query<&mut Npc>,
    factions: Res<Factions>,
) {
    let Ok(c) = conversations.get(entity) else { return };
    let mut npc = npcs.get_mut(c.actor).unwrap();
    if interaction::band(
        npc.disposition,
        c.charm(),
        actors.turned(c.actor),
        actors.definition(c.actor),
        &factions,
    ) != crate::game::factions::Band::Hostile
    {
        npc.disposition = 0x19;
    }
}

pub(super) fn faction(
    In((faction, delta)): In<(crate::content::factions::Faction, i8)>,
    mut factions: ResMut<Factions>,
    clock: Res<GameClock>,
    mut news: MessageWriter<FactionNews>,
) {
    if let Some(change) = factions.adjust(faction, delta, clock.ms) {
        news.write(change);
    }
}

pub(super) fn gold(
    In((actor, amount)): In<(Entity, u32)>,
    actors: Query<&crate::game::npcs::ActorId>,
    content: Res<Catalogs>,
    mut inventory: Single<&mut PartyInventory>,
) -> QuestReward {
    inventory.gold += amount;
    let giver = content.npcs[actors.get(actor).unwrap().npc()].source_name.clone();
    QuestReward::Gold { giver, amount }
}
pub(super) fn experience(
    In((members, amount)): In<(Vec<Entity>, u32)>,
    mut characters: Query<&mut Character>,
) -> QuestReward {
    for member in members {
        if let Ok(mut c) = characters.get_mut(member) {
            c.award_experience(amount);
        }
    }
    QuestReward::Experience { amount_each: amount }
}
pub(super) fn give_item(
    In(item): In<ItemId>,
    mut party: Single<(&mut PartyInventory, &Position)>,
    content: Res<Catalogs>,
    level: Single<Entity, With<Level>>,
    mut ids: ResMut<ItemIds>,
    mut rng: ResMut<GameRng>,
    mut news: MessageWriter<ItemNews>,
    mut commands: Commands,
) -> QuestReward {
    let (inventory, at) = &mut *party;
    let def = &content.items[item];
    let stack = make_item(def, &mut ids, &mut rng, true, true);
    if let Some(rest) = inventory.pool.receive(stack, &content.items) {
        if !def.crucial {
            crate::game::world::items::lay(
                &mut commands,
                *level,
                PartyBody::feet(at),
                crate::game::world::items::WorldItem::loose(rest, false),
            );
        } else {
            if inventory.hand.held.is_none() {
                inventory.hand.held = Some(rest);
            }
            news.write(ItemNews::CannotDrop);
        }
    }
    QuestReward::Item { name: def.name.clone() }
}
pub(super) fn take_item(
    In(item): In<ItemId>,
    mut party: Single<(&PartyRoster, &mut PartyInventory)>,
    characters: Query<&Character>,
    mut packs: Query<&mut Carried>,
) -> Option<QuestReward> {
    let (roster, inventory) = &mut *party;
    let owner = roster.iter().find_map(|member| {
        packs
            .get(member)
            .ok()?
            .backpack
            .iter()
            .flatten()
            .find(|s| s.item == item)
            .map(|s| (member, s.id))
    });
    let member = if let Some((member, id)) = owner {
        packs.get_mut(member).unwrap().consume_backpack_units(id, 1);
        characters.get(member).ok().map(|c| c.name.clone())
    } else {
        let id = inventory.pool.stacks.iter().find(|s| s.item == item)?.id;
        inventory.pool.consume_units(id, 1);
        None
    };
    Some(QuestReward::Taken { member })
}
