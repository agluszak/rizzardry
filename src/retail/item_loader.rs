//! Loads the retail item database as an [`ItemCatalog`].

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;
use thiserror::Error;

use super::items::{ItemRecord, item_flag, read_item_tables, read_items};
use super::pvl::PvlError;
use crate::content::items::{
    Dice, EquipClass, ItemCatalog, ItemDefinition, ItemId, LootTable, LootTables, QuantityKind,
    Requirement,
};

#[derive(Default, TypePath)]
pub struct ItemCatalogLoader;

#[derive(Debug, Error)]
pub enum ItemLoadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Items(#[from] PvlError),
    #[error("item {0}: unknown equipment class {1}")]
    EquipClass(usize, u8),
    #[error("item {0}: unknown quantity kind {1}")]
    QuantityKind(usize, u8),
}

fn definition(index: usize, record: &ItemRecord) -> Result<ItemDefinition, ItemLoadError> {
    let has = |flag| record.flags & flag != 0;
    let dice = |d: super::items::Dice| Dice { count: d.count, sides: d.sides, base: d.base };
    let requirement = |r: &super::items::Requirement| Requirement { id: r.id, minimum: r.minimum };
    Ok(ItemDefinition {
        id: ItemId(index as u16),
        name: record.name.clone(),
        kind: record.unidentified_name,
        class: EquipClass::from_retail(record.equip_class)
            .ok_or(ItemLoadError::EquipClass(index, record.equip_class))?,
        auto_identify: has(item_flag::AUTO_IDENTIFY),
        crucial: has(item_flag::NO_DISCARD),
        two_handed: has(item_flag::TWO_HANDED),
        off_hand: has(item_flag::OFF_HAND_ALLOWED),
        merchants_refuse: has(item_flag::MERCHANT_CANNOT_SELL),
        container: has(item_flag::CONTAINER),
        equip_to_use: has(item_flag::MUST_EQUIP_TO_USE),
        never_depletes: has(item_flag::NEVER_DEPLETES),
        weapon_skill: record.weapon_skill,
        range: record.range_category,
        damage: dice(record.damage),
        damage_bonus: record.damage_bonus,
        hit_bonus: record.hit_bonus,
        armor_class_bonus: record.armor_class_bonus,
        quantity: match record.quantity_kind {
            0 => QuantityKind::None,
            1 => QuantityKind::Stack,
            2 => QuantityKind::Charges,
            3 => QuantityKind::Uses,
            4 => QuantityKind::Shots,
            5 => QuantityKind::Unlabelled,
            other => return Err(ItemLoadError::QuantityKind(index, other)),
        },
        initial_quantity: dice(record.initial_quantity),
        maximum_quantity: record.maximum_quantity,
        profession_mask: record.profession_mask,
        race_mask: record.race_mask,
        gender_mask: record.gender_mask,
        attribute_requirements: record.attribute_requirements.iter().map(requirement).collect(),
        skill_requirements: record.skill_requirements.iter().map(requirement).collect(),
        value: record.value,
        weight: record.weight,
        cursed: record.binds_on_equip,
    })
}

pub fn item_catalog(data: &[u8]) -> Result<ItemCatalog, ItemLoadError> {
    let records = read_items(data)?;
    Ok(ItemCatalog(
        records.iter().enumerate().map(|(i, r)| definition(i, r)).collect::<Result<_, _>>()?,
    ))
}

impl AssetLoader for ItemCatalogLoader {
    type Asset = ItemCatalog;
    type Settings = ();
    type Error = ItemLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<ItemCatalog, ItemLoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        item_catalog(&data)
    }

    fn extensions(&self) -> &[&str] {
        &["dbs"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a retail installation"]
    fn every_retail_item_has_a_definition() {
        let root =
            std::env::var("RIZZARDRY_RETAIL").expect("set RIZZARDRY_RETAIL to run this test");
        let files = crate::retail::files::RetailFiles::open(root).unwrap();
        let catalog =
            item_catalog(&files.read("data/databases/items.dbs").unwrap().unwrap()).unwrap();
        let dagger = catalog.get(ItemId(0)).unwrap();
        assert_eq!((dagger.name.as_str(), dagger.class), ("Dagger", EquipClass::ShortWeapon));
        assert!(dagger.off_hand && !dagger.two_handed);
        let stacks = catalog.0.iter().filter(|i| i.quantity == QuantityKind::Stack).count();
        assert!(stacks > 10, "{stacks} stackable items");
        assert!(
            catalog
                .0
                .iter()
                .filter(|i| i.quantity == QuantityKind::Stack)
                .all(|i| i.maximum_quantity > 0)
        );
    }
}

#[derive(Default, TypePath)]
pub struct LootTablesLoader;

impl AssetLoader for LootTablesLoader {
    type Asset = LootTables;
    type Settings = ();
    type Error = ItemLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<LootTables, ItemLoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        let dice = |d: super::items::Dice| Dice { count: d.count, sides: d.sides, base: d.base };
        Ok(LootTables(
            read_item_tables(&data)?
                .into_iter()
                .map(|table| LootTable {
                    name: table.name,
                    entries: table.entries.into_iter().map(|(item, w)| (ItemId(item), w)).collect(),
                    level_scaled: table.level_scaled,
                    item_count: dice(table.item_count),
                    gold: dice(table.gold),
                })
                .collect(),
        ))
    }

    fn extensions(&self) -> &[&str] {
        &["dbs"]
    }
}
