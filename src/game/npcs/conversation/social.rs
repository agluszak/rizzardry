use super::*;
use crate::{
    content::{
        characters::Skill,
        items::{EquipClass, ItemId, QuantityKind},
        npcs::NpcDefinition,
    },
    game::{
        characters::{Character, Vitals},
        data::Catalogs,
        factions::{Band, Factions},
        inventory::PartyInventory,
        items::{Items, Lose},
        npcs::{
            Npc,
            access::{NpcActors, Scripts},
        },
        party::roster::PartyRoster,
        time::GameClock,
    },
};
use bevy::ecs::system::SystemParam;
#[derive(SystemParam)]
pub(super) struct Voices<'w, 's> {
    roster: Single<'w, 's, &'static PartyRoster>,
    members: Query<'w, 's, (&'static Character, &'static Vitals)>,
}
impl Voices<'_, '_> {
    pub fn skill(&self) -> u32 {
        trade::communication(self.roster.iter().filter_map(|m| self.members.get(m).ok()))
    }
    fn best(&self) -> Option<Entity> {
        self.roster
            .iter()
            .filter_map(|m| {
                self.members
                    .get(m)
                    .ok()
                    .filter(|(_, v)| v.can_act())
                    .map(|(c, _)| (m, c.skill(Skill::Communication)))
            })
            .max_by_key(|(_, skill)| *skill)
            .map(|(m, _)| m)
    }
    fn level(&self) -> u32 {
        crate::game::party::average_level(
            self.roster
                .iter()
                .filter_map(|m| self.members.get(m).ok())
                .filter(|(_, v)| v.can_fight())
                .map(|(c, _)| c.level()),
        )
    }
}
pub fn approach_ready(at: Option<u64>, now: u64) -> bool {
    at.is_none_or(|at| now - at > 43_200_000)
}
pub(super) fn band(
    own: u8,
    charm: u32,
    turned: bool,
    d: &NpcDefinition,
    factions: &Factions,
) -> Band {
    let score = if turned { 100 } else { (u32::from(own) + charm).min(100) as i32 };
    let score = if d.faction != 0
        && (d.monster_bound || factions.band(i32::from(d.faction)) == Band::Hostile)
    {
        factions.score(i32::from(d.faction)).map_or(score, i32::from)
    } else {
        score
    };
    match score {
        ..=32 => Band::Hostile,
        33..=65 => Band::Neutral,
        _ => Band::Friendly,
    }
}
fn debit(npc: &mut Npc, d: &NpcDefinition, value: u32, skill: u32) {
    let value = u64::from(value) * (500 + u64::from(skill)) / 500;
    npc.trade_pool = (u64::from(npc.trade_pool).max(value) - value) as u16;
    if npc.trade_pool == 0 {
        npc.trade_pool = d.trade_pool;
        npc.disposition = 75;
    }
}
pub(super) fn approach(
    mut conversations: Query<&mut Conversation>,
    actors: NpcActors,
    mut npcs: Query<&mut Npc>,
    voices: Voices,
    factions: Res<Factions>,
    clock: Res<GameClock>,
    mut practice: MessageWriter<rewards::Practice>,
) {
    for mut c in &mut conversations {
        let Some(Talk::Approach { approach }) =
            c.incoming.take_if(|t| matches!(t, Talk::Approach { .. }))
        else {
            continue;
        };
        if !c.ready() || !c.menu {
            continue;
        }
        let (Some(d), Ok(mut npc)) = (actors.definition(c.actor), npcs.get_mut(c.actor)) else {
            continue;
        };
        let (last, scale, usage) = match approach {
            Approach::Charm => (npc.charmed_at, d.charm_scale, 5),
            Approach::SmallTalk => (npc.small_talk_at, d.talk_scale, 8),
        };
        if !approach_ready(last, clock.ms) {
            continue;
        }
        let Some(member) = voices.best() else { continue };
        let scale = i32::from(scale);
        let mut delta = scale + voices.skill() as i32 * (scale.abs() / 5) / 100;
        if approach == Approach::Charm && delta > 0 {
            delta = (voices.level() * delta as u32 / actors.species_level(c.actor)) as i32;
        }
        npc.disposition = (i32::from(npc.disposition) + delta).clamp(0, 99) as u8;
        match approach {
            Approach::Charm => npc.charmed_at = Some(clock.ms),
            Approach::SmallTalk => npc.small_talk_at = Some(clock.ms),
        }
        let standing = band(npc.disposition, c.charm(), actors.turned(c.actor), d, &factions);
        let q = match (standing, approach) {
            (Band::Hostile, _) => {
                c.after = Some(After::End);
                0x18
            }
            (Band::Neutral, Approach::Charm) => 6,
            (Band::Neutral, Approach::SmallTalk) => 3,
            (Band::Friendly, Approach::Charm) => 4,
            (Band::Friendly, Approach::SmallTalk) => {
                if npc.greeting_pending || d.merchant {
                    0
                } else {
                    1
                }
            }
        };
        c.menu = standing != Band::Friendly;
        c.say(q);
        practice.write(rewards::Practice::Member(member, usage));
    }
}
pub(super) fn welcome(
    mut conversations: Query<&mut Conversation>,
    actors: NpcActors,
    npcs: Query<&Npc>,
    factions: Res<Factions>,
) {
    for mut c in &mut conversations {
        let (Some(d), Ok(npc)) = (actors.definition(c.actor), npcs.get(c.actor)) else { continue };
        if c.menu
            && band(npc.disposition, c.charm(), actors.turned(c.actor), d, &factions)
                == Band::Friendly
        {
            c.menu = false;
            c.say(if npc.greeting_pending || d.merchant { 0 } else { 1 });
        }
    }
}
pub(super) fn gold(
    mut conversations: Query<&mut Conversation>,
    actors: NpcActors,
    mut npcs: Query<&mut Npc>,
    voices: Voices,
    factions: Res<Factions>,
    mut inventory: Single<&mut PartyInventory>,
) {
    for mut c in &mut conversations {
        let confirm = matches!(c.wait, ConversationWait::GiveAll { .. });
        let Some(talk) = c.incoming.take_if(|t| {
            matches!(t, Talk::GiveGold { .. }) || (confirm && matches!(t, Talk::PriceAnswer { .. }))
        }) else {
            continue;
        };
        let amount = match talk {
            Talk::GiveGold { amount } if c.ready() => {
                if amount > 0 && amount == inventory.gold {
                    c.wait = ConversationWait::GiveAll { gold: amount };
                    continue;
                }
                amount
            }
            Talk::PriceAnswer { accept } => {
                let ConversationWait::GiveAll { gold } = c.wait else { continue };
                c.wait = ConversationWait::Ready;
                if !accept {
                    continue;
                }
                gold
            }
            _ => continue,
        };
        if amount == 0 || amount > inventory.gold {
            c.result = Some(Outcome::GiftRefused(if amount == 0 {
                TradeError::InvalidQuantity
            } else {
                TradeError::Funds
            }));
            continue;
        }
        let (Some(d), Ok(mut npc)) = (actors.definition(c.actor), npcs.get_mut(c.actor)) else {
            continue;
        };
        let cold = band(npc.disposition, c.charm(), actors.turned(c.actor), d, &factions)
            != Band::Friendly;
        if cold && d.trade_pool == 0 {
            c.paid(7);
            continue;
        }
        inventory.gold -= amount;
        if cold {
            debit(&mut npc, d, amount, voices.skill());
        }
        let quote = if band(npc.disposition, c.charm(), actors.turned(c.actor), d, &factions)
            == Band::Friendly
        {
            0x10
        } else {
            7
        };
        c.paid(quote);
    }
}
pub(super) fn gift(
    mut conversations: Query<&mut Conversation>,
    actors: NpcActors,
    scripts: Scripts,
    mut npcs: Query<&mut Npc>,
    voices: Voices,
    factions: Res<Factions>,
    content: If<Res<Catalogs>>,
    held: Items,
    mut commands: Commands,
) {
    for mut c in &mut conversations {
        let Some(Talk::Give { item: source }) =
            c.incoming.take_if(|t| matches!(t, Talk::Give { .. }))
        else {
            continue;
        };
        if !c.ready() {
            continue;
        }
        let id = trade::for_sale(source, &voices.roster, &held).ok().and_then(|()| held.id(source));
        let Some(stack_item) = id else {
            c.result = Some(Outcome::GiftRefused(TradeError::StaleSelection));
            continue;
        };
        let (Some(d), Some(script), Ok(mut npc)) =
            (actors.definition(c.actor), scripts.get(c.actor), npcs.get_mut(c.actor))
        else {
            continue;
        };
        let item = &content.items[stack_item];
        let count = held.units(source);
        let value = match (item.class, item.quantity) {
            (EquipClass::Ammunition, _) => {
                (item.value * count).div_ceil(crate::content::items::AMMUNITION_BUNDLE)
            }
            (_, QuantityKind::Stack) => item.value * count.max(1),
            _ => item.value,
        };
        let cold = !d.voice_script
            && band(npc.disposition, c.charm(), actors.turned(c.actor), d, &factions)
                != Band::Friendly;
        let authored = script.item_quote(stack_item);
        let (quote, keep, bribe) = if cold && item.crucial {
            (0x11, false, false)
        } else if cold && (d.kind == 20 || (stack_item != ItemId(0x29f) && value < 300)) {
            (7, false, false)
        } else if let Some((q, keep)) = authored {
            (q, keep || cold, false)
        } else if cold {
            (
                if u64::from(value) * (500 + u64::from(voices.skill())) / 500
                    >= u64::from(npc.trade_pool)
                {
                    0x10
                } else {
                    7
                },
                true,
                true,
            )
        } else if d.voice_script {
            continue;
        } else if item.crucial {
            (0x11, false, false)
        } else {
            (0x10, true, false)
        };
        let Some(q) = script.quotes.get(usize::from(quote)) else {
            c.block(quote, 0, "missing gift response".into());
            continue;
        };
        if let Err((entry, error)) = playback::validate(q, &content) {
            c.block(quote, entry, error);
            continue;
        }
        if keep {
            commands.trigger(Lose { item: source, amount: 1 });
        }
        if bribe {
            debit(&mut npc, d, value, voices.skill());
        }
        c.say(quote);
    }
}
