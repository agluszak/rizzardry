//! Quest transfers run after script evaluation and before its next continuation.
use super::*;
use crate::{
    content::{characters::Skill, items::ItemId},
    game::{
        characters::{Character, Vitals},
        data::Catalogs,
        inventory::{Carried, ItemIds, PartyInventory, make_item},
        party::{body::PartyBody, roster::PartyRoster},
        rng::GameRng,
        world::{
            items::{self, WorldItem},
            level::Level,
        },
    },
};
use avian3d::prelude::Position;
#[derive(Message)]
pub(super) struct Reward {
    pub session: Entity,
    pub kind: Kind,
}
pub(super) enum Kind {
    Gold { giver: String, amount: u32 },
    Experience(u32),
    Give(ItemId),
    Take(ItemId),
}
#[derive(Message)]
pub(super) enum Practice {
    Greeting,
    Member(Entity, u32),
}
pub(super) fn practise(
    mut requests: MessageReader<Practice>,
    party: Single<&PartyRoster>,
    mut members: Query<(&mut Character, &Vitals)>,
    mut rng: ResMut<GameRng>,
) {
    for request in requests.read() {
        for member in party.iter() {
            let Ok((mut c, v)) = members.get_mut(member) else { continue };
            let n = match request {
                Practice::Greeting if v.can_fight() => 15,
                Practice::Member(e, n) if *e == member => *n,
                _ => continue,
            };
            c.practise(Skill::Communication, n, &mut rng);
        }
    }
}
pub(super) fn deliver(
    mut rewards: MessageReader<Reward>,
    mut conversations: Query<&mut Conversation>,
    mut party: Single<(&PartyRoster, &mut PartyInventory, &Position)>,
    mut characters: Query<(&mut Character, &Vitals)>,
    mut packs: Query<&mut Carried>,
    content: If<Res<Catalogs>>,
    level: Single<Entity, With<Level>>,
    mut ids: ResMut<ItemIds>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    let (roster, inventory, at) = &mut *party;
    for reward in rewards.read() {
        let result = match &reward.kind {
            Kind::Gold { giver, amount } => {
                inventory.gold = inventory.gold.saturating_add(*amount);
                Some(QuestReward::Gold { giver: giver.clone(), amount: *amount })
            }
            Kind::Experience(amount) => {
                for e in roster.iter() {
                    if let Ok((mut c, v)) = characters.get_mut(e)
                        && !v.is_dead()
                    {
                        c.award_experience(*amount);
                    }
                }
                Some(QuestReward::Experience { amount_each: *amount })
            }
            Kind::Give(id) => {
                let d = &content.items[*id];
                let stack = make_item(d, &mut ids, &mut rng, true, true);
                if let Some(rest) = inventory.pool.receive(stack, &content.items) {
                    if d.crucial && inventory.hand.held.is_none() {
                        inventory.hand.held = Some(rest);
                    } else {
                        items::lay(
                            &mut commands,
                            *level,
                            PartyBody::feet(at),
                            WorldItem::loose(rest, false),
                        );
                    }
                }
                Some(QuestReward::Item { name: d.name.clone() })
            }
            Kind::Take(id) => {
                let owner = roster.iter().find_map(|m| {
                    packs
                        .get(m)
                        .ok()?
                        .backpack
                        .iter()
                        .flatten()
                        .find(|s| s.item == *id)
                        .map(|s| (m, s.id))
                });
                if let Some((member, id)) = owner {
                    if let Ok(mut pack) = packs.get_mut(member) {
                        pack.consume_backpack_units(id, 1);
                    }
                    Some(QuestReward::Taken {
                        member: characters.get(member).ok().map(|(c, _)| c.name.clone()),
                    })
                } else if let Some(id) =
                    inventory.pool.stacks.iter().find(|s| s.item == *id).map(|s| s.id)
                {
                    inventory.pool.consume_units(id, 1);
                    Some(QuestReward::Taken { member: None })
                } else {
                    None
                }
            }
        };
        if let Some(result) = result
            && let Ok(mut c) = conversations.get_mut(reward.session)
        {
            c.transcript.push(TranscriptLine::Reward(result));
        }
    }
}
