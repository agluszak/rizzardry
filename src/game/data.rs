//! The retail databases the rules read, loaded once.

use bevy::asset::LoadState;
use bevy::prelude::*;

use crate::content::items::{ItemCatalog, LootTables};
use crate::content::npcs::{FactCatalog, NpcCatalog};
use crate::content::species::MonsterCatalog;
use crate::content::spells::SpellCatalog;

/// The databases, present once every one has loaded. A level cannot be
/// entered before, so whatever runs in play may rely on them.
#[derive(Resource, Debug, Default)]
pub struct Catalogs {
    pub items: ItemCatalog,
    pub loot: LootTables,
    pub species: MonsterCatalog,
    pub spells: SpellCatalog,
    pub npcs: NpcCatalog,
    pub facts: FactCatalog,
}

/// The databases while they load.
#[derive(Resource)]
pub struct Loading {
    items: Handle<ItemCatalog>,
    loot: Handle<LootTables>,
    species: Handle<MonsterCatalog>,
    spells: Handle<SpellCatalog>,
    npcs: Handle<NpcCatalog>,
    facts: Handle<FactCatalog>,
}

pub fn load_catalogs(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(Loading {
        items: assets.load(crate::content::paths::ITEMS),
        loot: assets.load(crate::content::paths::LOOT_TABLES),
        species: assets.load(crate::content::paths::SPECIES),
        spells: assets.load(crate::content::paths::SPELLS),
        npcs: assets.load(crate::content::paths::NPCS),
        facts: assets.load(crate::content::paths::FACTS),
    });
}

/// Once every database has loaded, they become the [`Catalogs`]; the game
/// cannot be played without one that failed.
pub fn publish_catalogs(
    mut commands: Commands,
    loading: Res<Loading>,
    assets: Res<AssetServer>,
    (mut items, mut loot, mut species, mut spells): (
        ResMut<Assets<ItemCatalog>>,
        ResMut<Assets<LootTables>>,
        ResMut<Assets<MonsterCatalog>>,
        ResMut<Assets<SpellCatalog>>,
    ),
    (mut npcs, mut facts): (ResMut<Assets<NpcCatalog>>, ResMut<Assets<FactCatalog>>),
    mut exit: MessageWriter<AppExit>,
) {
    let handles = [
        loading.items.id().untyped(),
        loading.loot.id().untyped(),
        loading.species.id().untyped(),
        loading.spells.id().untyped(),
        loading.npcs.id().untyped(),
        loading.facts.id().untyped(),
    ];
    if handles.iter().any(|&id| matches!(assets.get_load_state(id), Some(LoadState::Failed(_)))) {
        error!("a retail database failed to load");
        commands.remove_resource::<Loading>();
        exit.write(AppExit::error());
        return;
    }
    if !handles.iter().all(|&id| assets.is_loaded(id)) {
        return;
    }
    commands.insert_resource(Catalogs {
        items: items.remove(&loading.items).unwrap(),
        loot: loot.remove(&loading.loot).unwrap(),
        species: species.remove(&loading.species).unwrap(),
        spells: spells.remove(&loading.spells).unwrap(),
        npcs: npcs.remove(&loading.npcs).unwrap(),
        facts: facts.remove(&loading.facts).unwrap(),
    });
    commands.remove_resource::<Loading>();
}
