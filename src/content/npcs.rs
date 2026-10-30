//! NPC definitions and authored dialogue. Numbers are content keys, never runtime slots.
use super::factions::Faction;
use super::items::ItemId;
use bevy::prelude::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NpcId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FactId(pub u16);

#[derive(Clone, Debug, Default)]
pub struct NpcDefinition {
    pub source_name: String,
    pub display_name: String,
    pub monster_bound: bool,
    pub owns_stock: bool,
    pub merchant: bool,
    /// The character the NPC brings when it joins the party; only NPCs
    /// with one can join (retail `has_group`).
    pub character: Option<NpcCharacter>,
    pub disposition: u8,
    /// How much charming it and making small talk move its disposition.
    pub charm_scale: i8,
    pub talk_scale: i8,
    pub faction: u8,
    /// How much it takes in bribes before it warms to the party.
    pub trade_pool: u16,
    /// Retail's NPC kind; one kind takes no bribes in kind.
    pub kind: i32,
    /// One bit per region whose service the NPC offers; it will not leave
    /// that region's post to join.
    pub services: u32,
    /// The average party level before the NPC will join; 0 for any.
    pub min_party_level: u8,
    pub buy_factor: f32,
    pub sell_factor: f32,
    pub accepted_classes: enumset::EnumSet<super::items::EquipClass>,
    pub voice_script: bool,
    pub stock: Vec<StockRule>,
    /// The loot table whose entries the NPC carries, if any.
    pub item_table: Option<u16>,
    /// The purse the party gets when the NPC dies.
    pub gold: u32,
}
/// The character a recruitable NPC brings to the party (retail
/// `W8NpcCharacterTemplate`).
#[derive(Clone, Debug)]
pub struct NpcCharacter {
    pub name: String,
    pub profession: super::characters::Profession,
    pub race: super::characters::Race,
    pub gender: super::characters::Gender,
    /// Index into the portrait table.
    pub portrait: usize,
    pub level: u8,
    pub attributes: enum_map::EnumMap<super::characters::Attribute, u8>,
    pub skills: enum_map::EnumMap<super::characters::Skill, u8>,
    pub spells: Vec<super::spells::SpellId>,
    /// What it wears, in retail's slot order.
    pub equipment: [Option<ItemId>; 12],
    pub backpack: [Option<ItemId>; 8],
    /// Items it keeps when it leaves the party.
    pub wanted: Vec<ItemId>,
}

impl NpcDefinition {
    pub fn script_path(&self) -> String {
        format!(
            "data/npc scripts/{}_{}.nsf",
            if self.voice_script { "voc" } else { "npc" },
            self.source_name.to_lowercase()
        )
    }
    pub fn voice_stem(&self) -> String {
        format!(
            "{}_{}",
            if self.voice_script { "voc" } else { "npc" },
            self.source_name.to_lowercase()
        )
    }
}
#[derive(Clone, Debug)]
pub struct StockRule {
    pub item: ItemId,
    pub quantity: u8,
    pub persistent: bool,
}
#[derive(Asset, TypePath, Debug, Default)]
pub struct NpcCatalog(pub Vec<NpcDefinition>);
impl NpcCatalog {
    pub fn get(&self, id: NpcId) -> Option<&NpcDefinition> {
        self.0.get(id.0 as usize)
    }
}
/// The definition of an NPC that exists.
impl std::ops::Index<NpcId> for NpcCatalog {
    type Output = NpcDefinition;
    fn index(&self, id: NpcId) -> &NpcDefinition {
        &self.0[id.0 as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopicCategory {
    Person,
    Item,
    Place,
    Misc,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    Fixed(u16),
    Random {
        first: u16,
        last: u16,
    },
    /// The NPC with this display name takes over the conversation and says
    /// this quote of its own script.
    Speaker {
        npc: String,
        quote: u16,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogueOp {
    /// Retail kind 1 is metadata used by the interaction matcher, not a jump.
    Parent(u16),
    Next(Response),
    Conditional {
        facts: Vec<(FactId, u8)>,
        response: Response,
    },
    Keywords(Vec<String>),
    Options(Vec<String>),
    Reply {
        response: u16,
        role: i32,
        words: Vec<String>,
    },
    SetFact(FactId, u8),
    /// The NPC hands the party an item, identified and at full quantity.
    GiveItem(ItemId),
    /// One of the item leaves the party.
    TakeItem(ItemId),
    /// This quote answers the party offering the item. The NPC keeps what it
    /// asked for and hands back what it only remarks on.
    ItemQuote {
        item: ItemId,
        keeps: bool,
    },
    GiveGold(u32),
    GiveExperience(u32),
    /// The party answers by typing one of these words, not by choosing.
    KeywordInput(Vec<String>),
    /// The dialogue closes and the NPC goes back to what it was doing.
    CloseAndResume,
    /// The named NPC, or this one, leaves the party if it is in it.
    LeaveParty {
        npc: Option<String>,
    },
    /// The NPC's monster script carries on from this label; retail ignores
    /// the entry (`None`) when its bound is 4.
    MonsterScriptLabel(Option<String>),
    /// The named NPC's disposition toward the party changes.
    AdjustDisposition {
        npc: String,
        delta: i8,
    },
    /// A party member remarks on the moment with this character event.
    PartyRemark(u16),
    /// A change to a faction's standing. Retail ignores a name it does not
    /// know, such as the shipped `FACTION_HIGARDI`, so none is changed.
    AdjustFaction {
        faction: Option<Faction>,
        delta: i8,
    },
    Close,
    Price {
        gold: u32,
        always: bool,
    },
    Decline(u16),
    Mention {
        category: TopicCategory,
        text: String,
    },
    /// Keep every operand so unsupported mandatory content has an honest diagnostic.
    Unsupported {
        opcode: u8,
        operands: [i32; 3],
        subentries: Vec<(i32, Option<String>)>,
    },
}
#[derive(Clone, Debug, Default)]
pub struct Quote {
    pub text: Vec<String>,
    pub operations: Vec<DialogueOp>,
    pub alert: bool,
}
#[derive(Asset, TypePath, Clone, Debug, Default)]
pub struct DialogueScript {
    pub quotes: Vec<Quote>,
}
impl DialogueScript {
    /// The first quote that answers the offered item, and whether the NPC
    /// keeps it (retail `FindNpcScriptItemQuote`).
    pub fn item_quote(&self, offered: ItemId) -> Option<(u16, bool)> {
        self.quotes.iter().enumerate().find_map(|(index, quote)| {
            quote.operations.iter().find_map(|op| match *op {
                DialogueOp::ItemQuote { item, keeps } if item == offered => {
                    Some((index as u16, keeps))
                }
                _ => None,
            })
        })
    }
    pub fn topic(&self, text: &str) -> Option<u16> {
        self.quotes.iter().position(|q| q.operations.iter().any(|op| matches!(op,
            DialogueOp::Keywords(words) if words.iter().any(|w| w.eq_ignore_ascii_case(text.trim())))))
            .map(|i| i as u16)
    }
}

/// Which difficulty settings include this historical entry in the journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalVisibility {
    AllDifficulties,
    NoviceAndNormal,
    NoviceOnly,
    Hidden,
}

#[derive(Clone, Debug)]
pub struct FactDefinition {
    pub highlight_when_true: bool,
    pub visibility: JournalVisibility,
    pub id: FactId,
    pub symbol: String,
    pub description: String,
    pub alternate_description: String,
}
#[derive(Asset, TypePath, Debug, Default)]
pub struct FactCatalog(pub Vec<FactDefinition>);
impl FactCatalog {
    pub fn get(&self, id: FactId) -> Option<&FactDefinition> {
        self.0.iter().find(|f| f.id == id)
    }
}
/// The definition of a fact the game has recorded.
impl std::ops::Index<FactId> for FactCatalog {
    type Output = FactDefinition;
    fn index(&self, id: FactId) -> &FactDefinition {
        self.get(id).expect("a fact the catalog defines")
    }
}
