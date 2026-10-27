//! Where the content assets are: the asset paths that name them.

/// A file of the retail installation, by its retail path.
pub fn retail(path: &str) -> String {
    format!("retail://{path}")
}

pub const ITEMS: &str = "retail://data/databases/items.dbs";
pub const LOOT_TABLES: &str = "retail://data/databases/itemtables.dbs";
pub const SPECIES: &str = "retail://data/databases/monsters.dbs";
pub const SPELLS: &str = "retail://data/databases/spelltables.dbs";
pub const SPELL_EFFECTS: &str = "retail://data/databases/spelleffect.dbs";
pub const SPELL_DESCRIPTIONS: &str = "retail://data/databases/spelldesc.dbs";
pub const NPCS: &str = "retail://data/databases/npc.dbs";
pub const FACTS: &str = "retail://data/databases/fact.dbs";
/// The game's string table.
pub const STRINGS: &str = "retail://data/strings/stringdata.dat";

/// A species' model and body, by its model name.
pub fn monster(model: &str) -> String {
    format!("retail://data/monsters/{model}.mls")
}

/// A level folder's trigger messages.
pub fn level_messages(folder: &str) -> String {
    format!("retail://data/messages/{}.msg", folder.to_lowercase())
}
