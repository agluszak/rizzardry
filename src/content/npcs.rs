//! NPC definitions and authored dialogue. Numbers are content keys, never runtime slots.
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
    pub faction: u8,
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
    Random { first: u16, last: u16 },
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
    pub name: Option<String>,
    pub quotes: Vec<Quote>,
}
impl DialogueScript {
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
