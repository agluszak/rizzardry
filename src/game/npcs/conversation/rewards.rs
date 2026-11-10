//! Quest transfers run after script evaluation and before its next continuation.
use super::*;
use crate::{
    content::{characters::Skill, items::ItemId},
    game::{
        characters::{Character, Practise, Vitals},
        data::Catalogs,
        inventory::PartyInventory,
        items::{Give, Items, Lose, Place, Put, make, roll_amount},
        party::{body::PartyBody, roster::PartyRoster},
        rng::GameRng,
        world::{items::CLEARANCE, level::Level},
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
    members: Query<&Vitals>,
    mut commands: Commands,
) {
    for request in requests.read() {
        for member in party.iter() {
            let Ok(v) = members.get(member) else { continue };
            let n = match request {
                Practice::Greeting if v.can_fight() => 15,
                Practice::Member(e, n) if *e == member => *n,
                _ => continue,
            };
            commands.trigger(Practise { member, skill: Skill::Communication, usage: n });
        }
    }
}
pub(super) fn deliver(
    mut rewards: MessageReader<Reward>,
    mut conversations: Query<&mut Conversation>,
    mut party: Single<(Entity, &PartyRoster, &mut PartyInventory, &Position)>,
    mut characters: Query<(&mut Character, &Vitals)>,
    held: Items,
    content: If<Res<Catalogs>>,
    level: Single<Entity, With<Level>>,
    mut rng: ResMut<GameRng>,
    mut commands: Commands,
) {
    let (party, roster, inventory, at) = &mut *party;
    for reward in rewards.read() {
        let result = match &reward.kind {
            Kind::Gold { giver, amount } => {
                inventory.gold += *amount;
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
                let item = make(&mut commands, d, roll_amount(d, &mut rng, true), true);
                // The pool takes what it has room for; what it cannot is
                // held in the hand if it is crucial, else lies at their feet.
                let rests = if d.crucial && held.held(*party).is_none() {
                    Place::Hand { party: *party }
                } else {
                    Place::Ground { level: *level, at: PartyBody::feet(at) + Vec3::Y * CLEARANCE }
                };
                commands.trigger(Put { item, at: rests });
                commands.trigger(Give {
                    item,
                    to: None,
                    party: *party,
                    pool_first: true,
                    told: false,
                });
                Some(QuestReward::Item { name: d.name.clone() })
            }
            Kind::Take(id) => {
                let is = |&item: &Entity| held.id(item) == Some(*id);
                let owner = roster.iter().find_map(|m| {
                    held.pack(m).into_iter().flatten().find(is).map(|item| (Some(m), item))
                });
                let pooled = held.pool(*party).iter().copied().find(is).map(|item| (None, item));
                if let Some((member, item)) = owner.or(pooled) {
                    commands.trigger(Lose { item, amount: 1 });
                    Some(QuestReward::Taken {
                        member: member
                            .and_then(|member| characters.get(member).ok())
                            .map(|(c, _)| c.name.clone()),
                    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::Race;
    use crate::game::characters::{Gender, Profession};
    use crate::game::party::roster::{PartyMember, test_party};
    use bevy::ecs::system::RunSystemOnce;

    /// Experience is for the members of the party; a character with no seat
    /// in it is given none.
    #[test]
    fn experience_is_for_those_with_a_seat() {
        let mut app = App::new();
        app.add_message::<Reward>()
            .insert_resource(Catalogs::default())
            .insert_resource(GameRng::seeded(1));
        let world = app.world_mut();
        let character = || {
            let sheet = Character::new("A", Race::Human, Gender::Female, Profession::Fighter);
            let vitals = Vitals::new(&sheet);
            (sheet, vitals)
        };
        let (seated, seatless) = (world.spawn(character()).id(), world.spawn(character()).id());
        let party = test_party(world, &[seated]);
        world.entity_mut(party).insert(Position(Vec3::ZERO));
        world.spawn(Level {
            path: String::new(),
            definition: default(),
            navmeshes: vec![],
            script: default(),
            messages: None,
        });
        assert!(!world.entity(seatless).contains::<PartyMember>());
        let session = world.spawn_empty().id();
        world.write_message(Reward { session, kind: Kind::Experience(500) });
        world.run_system_once(deliver).unwrap();
        assert_eq!(world.get::<Character>(seated).unwrap().experience, 500);
        assert_eq!(world.get::<Character>(seatless).unwrap().experience, 0);
    }
}
