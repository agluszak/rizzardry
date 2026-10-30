//! The retail databases the rules read, loaded once.

use bevy::asset::LoadState;
use bevy::prelude::*;

use crate::content::items::{ItemCatalog, LootTables};
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
}

/// The databases while they load.
#[derive(Resource)]
pub struct Loading {
    items: Handle<ItemCatalog>,
    loot: Handle<LootTables>,
    species: Handle<MonsterCatalog>,
    spells: Handle<SpellCatalog>,
}

pub fn load_catalogs(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(Loading {
        items: assets.load(crate::content::paths::ITEMS),
        loot: assets.load(crate::content::paths::LOOT_TABLES),
        species: assets.load(crate::content::paths::SPECIES),
        spells: assets.load(crate::content::paths::SPELLS),
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
    mut exit: MessageWriter<AppExit>,
) {
    let handles = [
        loading.items.id().untyped(),
        loading.loot.id().untyped(),
        loading.species.id().untyped(),
        loading.spells.id().untyped(),
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
    });
    commands.remove_resource::<Loading>();
}

impl Catalogs {
    /// Run `f` with the catalogs while the world stays writable, for
    /// operations that hold the whole world.
    pub fn scope<T>(world: &mut World, f: impl FnOnce(&mut World, &Catalogs) -> T) -> T {
        world.resource_scope(|world, catalogs: Mut<Catalogs>| f(world, &catalogs))
    }
}
