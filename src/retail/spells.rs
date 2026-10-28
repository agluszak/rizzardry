//! The retail spell database (`data/databases/spelltables.dbs`): a count and
//! a version, then 0x2c0-byte records, each a 0x101-byte editor prefix the
//! game skips and a 0x1bf-byte runtime record (retail
//! `InitializeSpellDatabase`, layout `gameplay_databases.h`).

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;
use enumset::EnumSet;

use super::record::{dice_at, reach};
use crate::bytes::{Bytes, DecodeError, c_string, f32_at, i32_at, utf16_string};
use crate::content::UNITS_PER_METRE;
use crate::content::characters::Realm;
use crate::content::spells::{
    PowerChoice, SpellCatalog, SpellDefinition, SpellId, Spellbook, Targeting, Usable,
};

const RECORD: usize = 0x2c0;
const PREFIX: usize = 0x101;

fn read_spell(index: usize, record: &[u8]) -> Result<SpellDefinition, DecodeError> {
    let r = &record[PREFIX..];
    let dice = dice_at(r, 0x51)?;
    let mut books = EnumSet::empty();
    for (at, book) in [
        (0x5a, Spellbook::Wizardry),
        (0x11f, Spellbook::Divinity),
        (0x48, Spellbook::Alchemy),
        (0x120, Spellbook::Psionics),
    ] {
        if r[at] != 0 {
            books.insert(book);
        }
    }
    let missile = i32_at(r, 0x140)?;
    Ok(SpellDefinition {
        id: SpellId(index as u8),
        name: utf16_string(&r[0x9b..0x9b + 128]),
        realm: u8::try_from(i32_at(r, 0x133)?).ok().and_then(|r| Realm::try_from(r).ok()),
        level: i32_at(r, 0x56)?.clamp(0, 7) as u8,
        cost: i32_at(r, 0x49)?.max(0) as u32,
        books,
        power: match i32_at(r, 0x12b)? {
            1 => PowerChoice::ChosenOrRepeat,
            2 => PowerChoice::Maximum,
            3 => PowerChoice::Fixed,
            _ => PowerChoice::Chosen,
        },
        reach: reach(i32_at(r, 0x12f)?.clamp(0, 3) as u8),
        targeting: match i32_at(r, 0x137)? {
            0 => Targeting::Caster,
            1 => Targeting::Ally,
            2 => Targeting::Party,
            3 => Targeting::Enemy,
            4 => Targeting::EnemyGroup,
            5 => Targeting::Cone,
            6 => Targeting::Radius,
            7 => Targeting::AllEnemies,
            8 => Targeting::Point,
            9 => Targeting::Item,
            _ => Targeting::LockOrTrap,
        },
        usable: match i32_at(r, 0x13b)? {
            1 => Usable::InCombat,
            2 => Usable::OutOfCombat,
            3 => Usable::Camped,
            4 => Usable::OnLockOrTrap,
            _ => Usable::Always,
        },
        dice: (dice.base != 0 || (dice.count > 0 && dice.sides > 0)).then_some(dice),
        show_effect_dice: i32_at(r, 0x147)? != 0,
        // The header names these two the other way round; retail's own use
        // (`CastSpellFromSource`, `SpellAffectedTarget`) multiplies 0x44 by
        // power and adds 0x4d.
        duration_per_power: i32_at(r, 0x44)?,
        duration_base: i32_at(r, 0x4d)?,
        radius: f32_at(r, 0x121)? / UNITS_PER_METRE,
        radius_per_power: f32_at(r, 0x127)? / UNITS_PER_METRE,
        missile: (r[0x13f] != 0).then(|| u32::try_from(missile).ok()).flatten(),
        after_visuals: r[0x144] != 0,
        monsters_cast: r[0x126] != 0,
        sound: c_string(&r[0x14b..0x14b + 0x74]).unwrap_or_default(),
    })
}

pub fn read_spell_catalog(data: &[u8]) -> Result<SpellCatalog, DecodeError> {
    let mut bytes = Bytes::new(data);
    let count = bytes.u32()? as usize;
    bytes.u32()?; // version
    if bytes.remaining() < count * RECORD {
        return Err(bytes.invalid("spell database shorter than its count"));
    }
    let records = data[8..8 + count * RECORD].as_chunks::<RECORD>().0;
    Ok(SpellCatalog(
        records.iter().enumerate().map(|(i, r)| read_spell(i, r)).collect::<Result<_, _>>()?,
    ))
}

#[derive(Default, TypePath)]
pub struct SpellCatalogLoader;

impl AssetLoader for SpellCatalogLoader {
    type Asset = SpellCatalog;
    type Settings = ();
    type Error = super::LoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<SpellCatalog, Self::Error> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        Ok(read_spell_catalog(&data)?)
    }

    fn extensions(&self) -> &[&str] {
        &["dbs"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::items::Dice;

    /// The three spells the first casting slice exercises, as retail authors them.
    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_spells_decode_as_recorded() {
        let files = crate::retail::files::retail_files();
        let data = files.read("data/databases/spelltables.dbs").unwrap().unwrap();
        let catalog = read_spell_catalog(&data).unwrap();
        assert_eq!(catalog.0.len(), 150);
        let blast = catalog.get(SpellId::ENERGY_BLAST).unwrap();
        assert_eq!(
            (blast.name.as_str(), blast.realm, blast.targeting, blast.cost, blast.usable),
            ("Energy Blast", Some(Realm::Fire), Targeting::Enemy, 2, Usable::InCombat)
        );
        assert_eq!(blast.dice, Some(Dice { base: 0, count: 1, sides: 7 }));
        assert!(blast.missile.is_some());
        let heal = catalog.get(SpellId::HEAL_WOUNDS).unwrap();
        assert_eq!(
            (heal.realm, heal.targeting, heal.cost, heal.power),
            (Some(Realm::Divine), Targeting::Ally, 3, PowerChoice::ChosenOrRepeat)
        );
        assert_eq!(heal.dice, Some(Dice { base: 0, count: 4, sides: 4 }));
        assert!(!heal.can_backfire());
        let sleep = catalog.get(SpellId::SLEEP).unwrap();
        assert_eq!(
            (sleep.realm, sleep.targeting, sleep.duration_per_power, sleep.duration_base),
            (Some(Realm::Air), Targeting::EnemyGroup, 1, 0)
        );
        assert!(sleep.books.contains(Spellbook::Wizardry));
    }
}

#[derive(Default, TypePath)]
pub struct SpellTextLoader;
impl AssetLoader for SpellTextLoader {
    type Asset = crate::content::spells::SpellText;
    type Settings = ();
    type Error = super::LoadError;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<crate::content::spells::SpellText, super::LoadError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        crate::retail::strings::read_messages(&data)
            .map(crate::content::spells::SpellText)
            .ok_or_else(super::text_loader::malformed_messages)
    }
    fn extensions(&self) -> &[&str] {
        &["dbs"]
    }
}
