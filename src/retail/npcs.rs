//! Checked NPC database and NSF decoding. See the original ReadNpcScriptQuote
//! (0x0055a140), LoadNpcScriptFile (0x0055a480) and packed database records.
use super::LoadError;
use crate::{
    bytes::{Bytes, i32_at, lenient_string, u16_at, u32_at, utf16_units},
    content::{items::ItemId, npcs::*},
};
use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    reflect::TypePath,
};

fn invalid(text: &str) -> LoadError {
    LoadError::Invalid(format!("invalid NPC content: {text}"))
}
fn wide(bytes: &[u8]) -> Result<String, LoadError> {
    String::from_utf16(&utf16_units(bytes)).map_err(|_| invalid("invalid UTF-16"))
}
pub fn read_npcs(data: &[u8]) -> Result<NpcCatalog, LoadError> {
    let mut b = Bytes::new(data);
    let count = b.u32()? as usize;
    if count > u16::MAX as usize || count > b.remaining() / 0x309 {
        return Err(invalid("NPC count"));
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let r = b.take(0x309)?;
        let version = u16_at(r, 0)?;
        if !(1..=2).contains(&version) {
            return Err(invalid("NPC record version"));
        }
        let mut stock = Vec::new();
        if r[0x9d] == 0 && version > 1 {
            let n = b.u32()? as usize;
            stock = b
                .records(n, 6, |b| Ok((b.i32()?, b.u8()?, b.u8()?)))?
                .into_iter()
                .map(|(item, quantity, persistent)| {
                    Ok(StockRule {
                        item: ItemId(u16::try_from(item).map_err(|_| invalid("stock item id"))?),
                        quantity,
                        persistent: persistent != 0,
                    })
                })
                .collect::<Result<_, LoadError>>()?;
        }
        let buy_factor = f32::from_bits(u32_at(r, 0x2ce)?);
        let sell_factor = f32::from_bits(u32_at(r, 0x2d2)?);
        if !buy_factor.is_finite() || !sell_factor.is_finite() {
            return Err(invalid("price factor"));
        }
        out.push(NpcDefinition {
            source_name: wide(&r[4..0x54])?,
            display_name: lenient_string(&r[0x9e..0xc4]),
            monster_bound: r[0x54] != 0,
            owns_stock: r[0x55] != 0,
            merchant: r[0x56] != 0,
            character: if r[0x57] != 0 { Some(npc_character(r)?) } else { None },
            services: u32_at(r, 0x68)?,
            min_party_level: r[0x6f],
            disposition: r[0x5c],
            faction: r[0x5f],
            buy_factor,
            sell_factor,
            accepted_classes: enumset::EnumSet::from_repr_truncated(u32_at(r, 0x2d6)?),
            voice_script: r[0x2ea] != 0,
            stock,
            // One-based; zero carries nothing.
            item_table: u16::try_from(i32_at(r, 0x64)?).ok().and_then(|id| id.checked_sub(1)),
            gold: u32::try_from(i32_at(r, 0x2eb)?).unwrap_or(0),
        });
    }
    if b.remaining() != 0 {
        return Err(invalid("trailing NPC bytes"));
    }
    Ok(NpcCatalog(out))
}
/// The record's character block (retail `W8NpcCharacterTemplate`, at
/// 0xc4), which `InitializeNpcCharacter` expands when the NPC joins.
fn npc_character(r: &[u8]) -> Result<NpcCharacter, LoadError> {
    use crate::content::characters::{Attribute, Gender, Profession, Race, Skill};
    use enum_map::Enum;
    fn index<T: Enum>(value: i32, count: usize, what: &str) -> Result<T, LoadError> {
        usize::try_from(value)
            .ok()
            .filter(|&i| i < count)
            .map(T::from_usize)
            .ok_or_else(|| invalid(what))
    }
    let byte = |value: i32| u8::try_from(value.clamp(0, 255)).unwrap_or(0);
    let item = |present: u16, id: u16| (present != 0 && id != 0xffff).then_some(ItemId(id));
    let mut equipment = [None; 12];
    for (slot, worn) in equipment.iter_mut().enumerate() {
        *worn = item(u16_at(r, 0x26a + 2 * slot)?, u16_at(r, 0x282 + 2 * slot)?);
    }
    let mut backpack = [None; 8];
    for (slot, carried) in backpack.iter_mut().enumerate() {
        *carried = item(u16_at(r, 0x29a + 2 * slot)?, u16_at(r, 0x2aa + 2 * slot)?);
    }
    let mut wanted = Vec::new();
    for slot in 0..3 {
        // One-based; zero wants nothing.
        let id = u16_at(r, 0x2bb + 2 * slot)?;
        if let Some(id) = id.checked_sub(1).filter(|&id| id < 0x7fff) {
            wanted.push(ItemId(id));
        }
    }
    Ok(NpcCharacter {
        name: wide(&r[0xc4..0xd8])?,
        profession: index::<Profession>(i32_at(r, 0x128)?, 15, "NPC profession")?,
        race: index::<Race>(i32_at(r, 0x12c)?, 16, "NPC race")?,
        gender: index::<Gender>(i32::from(r[0x2ba] as i8), 2, "NPC gender")?,
        portrait: usize::try_from(i32_at(r, 0x130)?).map_err(|_| invalid("NPC portrait"))?,
        level: byte(i32_at(r, 0x134)?).max(1),
        attributes: enum_map::EnumMap::from_fn(|a: Attribute| {
            byte(i32_at(r, 0x138 + 4 * a.into_usize()).unwrap_or(0))
        }),
        skills: enum_map::EnumMap::from_fn(|s: Skill| {
            byte(i32_at(r, 0x154 + 4 * s.into_usize()).unwrap_or(0))
        }),
        // One flag per spell, the first for spell 1.
        spells: (0..0x72u8)
            .filter(|&i| r[0x1f8 + usize::from(i)] != 0)
            .map(|i| crate::content::spells::SpellId(i + 1))
            .collect(),
        equipment,
        backpack,
        wanted,
    })
}
fn narrow(b: &mut Bytes) -> Result<String, LoadError> {
    let n = b.u16()? as usize;
    // NSF narrow strings in the shipped English corpus are ASCII.
    let raw = b.take(n)?;
    String::from_utf8(raw.to_vec()).map_err(|_| invalid("non-UTF8 NSF keyword"))
}
fn operation(
    kind: u8,
    args: [i32; 3],
    subs: Vec<(i32, Option<String>)>,
) -> Result<DialogueOp, LoadError> {
    let id = |v| u16::try_from(v).map_err(|_| invalid("negative/out-of-range reference"));
    let words = || {
        subs.iter()
            .map(|(_, s)| s.clone().ok_or_else(|| invalid("missing keyword")))
            .collect::<Result<Vec<_>, _>>()
    };
    let response = || -> Result<Response, LoadError> {
        match (args[1], args[2]) {
            (2, last) if last >= args[0] => {
                Ok(Response::Random { first: id(args[0])?, last: id(last)? })
            }
            (1, 0) => Ok(Response::Fixed(id(args[0])?)),
            _ => Err(invalid("unsupported response mode")),
        }
    };
    let unsupported =
        || DialogueOp::Unsupported { opcode: kind, operands: args, subentries: subs.clone() };
    Ok(match kind {
        1 => DialogueOp::Parent(id(args[0])?),
        2 => match response() {
            Ok(r) => DialogueOp::Next(r),
            Err(_) => unsupported(),
        },
        3 if subs.len().is_multiple_of(2) && args[2] != 4 => {
            let facts = subs
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| {
                    Ok((
                        FactId(id(p[0].0)?),
                        u8::try_from(p[1].0).map_err(|_| invalid("fact value"))?,
                    ))
                })
                .collect::<Result<_, LoadError>>()?;
            match response() {
                Ok(response) => DialogueOp::Conditional { facts, response },
                Err(_) => unsupported(),
            }
        }
        4 => DialogueOp::Keywords(words()?),
        5 => DialogueOp::Options(words()?),
        6 if (1..=3).contains(&args[2]) => {
            DialogueOp::Reply { response: id(args[0])?, role: args[2], words: words()? }
        }
        7 => DialogueOp::SetFact(
            FactId(id(args[0])?),
            u8::try_from(args[1]).map_err(|_| invalid("fact value"))?,
        ),
        8 => DialogueOp::Close,
        18 | 30 if args[0] >= 0 => DialogueOp::Price { gold: args[0] as u32, always: kind == 30 },
        23 => DialogueOp::Decline(id(args[0])?),
        25..=28 | 31..=34 if subs.len() == 1 => DialogueOp::Mention {
            category: match kind {
                25 | 31 => TopicCategory::Person,
                26 | 32 => TopicCategory::Item,
                27 | 33 => TopicCategory::Place,
                _ => TopicCategory::Misc,
            },
            text: words()?.remove(0),
        },
        _ => unsupported(),
    })
}
pub fn read_dialogue(data: &[u8]) -> Result<DialogueScript, LoadError> {
    let mut b = Bytes::new(data);
    let h = b.take(14)?;
    let count = u16_at(h, 4)? as usize;
    let name = if h[6..10] != [0; 4] { Some(narrow(&mut b)?) } else { None };
    if count > b.remaining() / 12 {
        return Err(invalid("quote count"));
    }
    let mut quotes = Vec::with_capacity(count);
    for _ in 0..count {
        let r = b.take(12)?;
        let mut text = Vec::new();
        if r[1..5] != [0; 4] {
            let n = b.u8()?;
            for _ in 0..n {
                let len = b.u16()? as usize;
                text.push(wide(b.take(len * 2)?)?);
            }
        }
        let count = u16_at(r, 9)? as usize;
        if count > b.remaining() / 18 {
            return Err(invalid("entry count"));
        }
        let mut operations = Vec::with_capacity(count);
        for _ in 0..count {
            let e = b.take(18)?;
            let args = [i32_at(e, 1)?, i32_at(e, 5)?, i32_at(e, 9)?];
            let mut subs = Vec::new();
            for _ in 0..e[13] {
                let value = b.i32()?;
                let text = if b.u32()? != 0 { Some(narrow(&mut b)?) } else { None };
                subs.push((value, text));
            }
            operations.push(operation(e[0], args, subs)?);
        }
        quotes.push(Quote { text, operations, alert: r[11] != 0 });
    }
    if b.remaining() != 0 {
        return Err(invalid("trailing NSF bytes"));
    }
    for q in &quotes {
        for op in &q.operations {
            let refs: Vec<u16> = match op {
                DialogueOp::Parent(q)
                | DialogueOp::Decline(q)
                | DialogueOp::Reply { response: q, .. } => vec![*q],
                DialogueOp::Next(r) | DialogueOp::Conditional { response: r, .. } => match r {
                    Response::Fixed(q) => vec![*q],
                    Response::Random { first, last } => vec![*first, *last],
                },
                _ => vec![],
            };
            if refs.iter().any(|&i| i as usize >= quotes.len()) {
                return Err(invalid("quote reference outside script"));
            }
        }
    }
    Ok(DialogueScript { name, quotes })
}
#[derive(Default, TypePath)]
pub struct NpcCatalogLoader;
#[derive(Default, TypePath)]
pub struct DialogueLoader;
impl AssetLoader for NpcCatalogLoader {
    type Asset = NpcCatalog;
    type Settings = ();
    type Error = LoadError;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<NpcCatalog, LoadError> {
        let mut b = Vec::new();
        reader.read_to_end(&mut b).await?;
        read_npcs(&b)
    }
}
impl AssetLoader for DialogueLoader {
    type Asset = DialogueScript;
    type Settings = ();
    type Error = LoadError;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<DialogueScript, LoadError> {
        let mut b = Vec::new();
        reader.read_to_end(&mut b).await?;
        read_dialogue(&b)
    }
    fn extensions(&self) -> &[&str] {
        &["nsf"]
    }
}
pub fn read_facts(data: &[u8]) -> Result<FactCatalog, LoadError> {
    let mut b = Bytes::new(data);
    let n = b.u32()? as usize;
    let records = b.records(n, 0x1d8, |b| b.take(0x1d8))?;
    let mut facts = Vec::with_capacity(n);
    let mut ids = std::collections::HashSet::new();
    for r in records {
        let id = FactId(u16::try_from(u32_at(r, 0)?).map_err(|_| invalid("fact id"))?);
        if !ids.insert(id) {
            return Err(invalid("duplicate fact id"));
        }
        facts.push(FactDefinition {
            highlight_when_true: r[0x36] != 0,
            visibility: match r[0x37] as i8 {
                ..=0 => crate::content::npcs::JournalVisibility::AllDifficulties,
                1 => crate::content::npcs::JournalVisibility::NoviceAndNormal,
                2 => crate::content::npcs::JournalVisibility::NoviceOnly,
                _ => crate::content::npcs::JournalVisibility::Hidden,
            },
            id,
            symbol: lenient_string(&r[4..0x36]),
            alternate_description: wide(&r[0x38..0x100])?,
            description: wide(&r[0x100..0x1d8])?,
        });
    }
    if b.remaining() != 0 {
        return Err(invalid("trailing fact bytes"));
    }
    Ok(FactCatalog(facts))
}
#[derive(Default, TypePath)]
pub struct FactCatalogLoader;
impl AssetLoader for FactCatalogLoader {
    type Asset = FactCatalog;
    type Settings = ();
    type Error = LoadError;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<FactCatalog, LoadError> {
        let mut b = Vec::new();
        reader.read_to_end(&mut b).await?;
        read_facts(&b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_interleaved_strings_conditions_and_preserves_unknown_operations() {
        // Synthetic NSF fixture; no retail dialogue/assets are redistributed.
        let bytes = include_bytes!("../../tests/fixtures/dialogue.nsf");
        let script = read_dialogue(bytes).unwrap();
        assert_eq!(script.quotes[0].text, ["FIRST"]);
        assert_eq!(script.topic("shop"), Some(1));
        assert_eq!(
            script.quotes[0].operations[0],
            DialogueOp::Conditional { facts: vec![(FactId(91), 1)], response: Response::Fixed(1) }
        );
        assert!(matches!(
            &script.quotes[1].operations[1],
            DialogueOp::Unsupported { opcode: 99, operands: [17, -7, 0], .. }
        ));
        let mut invalid = bytes.to_vec();
        invalid[40..44].copy_from_slice(&2i32.to_le_bytes());
        assert!(read_dialogue(&invalid).is_err(), "out-of-range authored continuation is rejected");
    }
    #[test]
    fn refuses_truncation() {
        assert!(read_npcs(&[1, 0, 0, 0]).is_err());
        assert!(read_dialogue(&[0; 13]).is_err());
    }
    #[test]
    #[ignore = "requires a retail installation"]
    fn burz_content_is_decoded_not_authored_in_rust() {
        let files = crate::retail::files::retail_files();
        let npcs = read_npcs(&files.read("data/databases/npc.dbs").unwrap().unwrap()).unwrap();
        let burz = &npcs.0[80];
        assert_eq!(burz.source_name, "Burz");
        assert_eq!(burz.disposition, 80);
        assert_eq!(burz.stock.len(), 35);
        let script = read_dialogue(&files.read(&burz.script_path()).unwrap().unwrap()).unwrap();
        assert_eq!(script.quotes.len(), 214);
        assert_eq!(script.topic("MONASTERY"), Some(127));
        assert_eq!(script.quotes[0].text, ["Pssst... hey!"]);
        assert!(
            matches!(&script.quotes[127].operations[2],DialogueOp::Conditional{facts,..} if facts==&[(FactId(514),0)])
        );
    }
}
