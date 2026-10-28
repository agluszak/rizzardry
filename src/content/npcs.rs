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
    pub recruitable: bool,
    pub disposition: u8,
    pub faction: u8,
    pub buy_factor: f32,
    pub sell_factor: f32,
    pub accepted_classes: enumset::EnumSet<super::items::EquipClass>,
    pub voice_script: bool,
    pub stock: Vec<StockRule>,
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
    GiveGold(u32),
    GiveExperience(u32),
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
