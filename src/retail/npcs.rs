//! Checked NPC database and NSF decoding. See the original ReadNpcScriptQuote
//! (0x0055a140), LoadNpcScriptFile (0x0055a480) and packed database records.
use super::LoadError;
use crate::{
    bytes::{Bytes, i32_at, lenient_string, u16_at, u32_at, utf16_units},
    content::{factions::Faction, items::ItemId, npcs::*},
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
            recruitable: r[0x57] != 0,
            disposition: r[0x5c],
            charm_scale: r[0x5d] as i8,
            talk_scale: r[0x5e] as i8,
            faction: r[0x5f],
            trade_pool: u16_at(r, 2)?,
            kind: u32_at(r, 0x58)? as i32,
            buy_factor,
            sell_factor,
            accepted_classes: enumset::EnumSet::from_repr_truncated(u32_at(r, 0x2d6)?),
            voice_script: r[0x2ea] != 0,
            stock,
        });
    }
    if b.remaining() != 0 {
        return Err(invalid("trailing NPC bytes"));
    }
    Ok(NpcCatalog(out))
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
    let named = || {
        subs.first()
            .and_then(|(_, text)| text.as_deref()?.get(4..))
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
    };
    // Retail's response chooser (`RunNpcScriptLine`, `SelectNpcQuoteResponse`):
    // mode 2 picks at random up to the bound; otherwise a bound of 4 hands
    // the dialogue to the NPC the first sub-entry names (its text past a
    // four-character prefix); anything else answers with the first operand.
    let response = || -> Result<Response, LoadError> {
        match (args[1], args[2]) {
            (2, last) if last >= args[0] => {
                Ok(Response::Random { first: id(args[0])?, last: id(last)? })
            }
            (2, _) => Err(invalid("empty random response range")),
            (_, 4) => {
                let npc = named().ok_or_else(|| invalid("speaker change without a speaker"))?;
                Ok(Response::Speaker { npc, quote: id(args[0])? })
            }
            _ => Ok(Response::Fixed(id(args[0])?)),
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
        3 => {
            // A speaker change names its speaker before the fact pairs.
            let pairs = &subs[usize::from(args[1] != 2 && args[2] == 4).min(subs.len())..];
            if !pairs.len().is_multiple_of(2) {
                return Ok(unsupported());
            }
            let facts = pairs
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
        9..=11 | 15 => match u16::try_from(args[0]).map(ItemId) {
            Ok(item) if kind == 9 => DialogueOp::GiveItem(item),
            Ok(item) if kind == 10 => DialogueOp::TakeItem(item),
            Ok(item) => DialogueOp::ItemQuote { item, keeps: kind == 11 },
            Err(_) => unsupported(),
        },
        // Retail names the other NPC by its sub-entry past a four-character
        // prefix (`ProcessNpcQuoteEntry`, `RunNpcScriptLine`).
        13 | 21 if named().is_some() => {
            let npc = named().unwrap();
            if kind == 13 {
                DialogueOp::LeaveParty { npc: Some(npc) }
            } else {
                // `ApplyNpcInteraction` passes the operand as a signed char.
                DialogueOp::AdjustDisposition { npc, delta: args[0] as i8 }
            }
        }
        12 if args[2] == 4 => DialogueOp::MonsterScriptLabel(None),
        12 => DialogueOp::MonsterScriptLabel(Some(
            words()?
                .into_iter()
                .next()
                .ok_or_else(|| invalid("monster script label without a label"))?,
        )),
        14 => DialogueOp::CloseAndResume,
        19 => DialogueOp::KeywordInput(words()?),
        20 => DialogueOp::LeaveParty { npc: None },
        24 => match u16::try_from(args[0]) {
            Ok(event) => DialogueOp::PartyRemark(event),
            Err(_) => unsupported(),
        },
        16 if args[0] >= 0 => DialogueOp::GiveGold(args[0] as u32),
        17 if args[0] >= 0 => DialogueOp::GiveExperience(args[0] as u32),
        // `AdjustFactionDisposition` takes the operand as a char.
        22 if subs.len() == 1 => {
            DialogueOp::AdjustFaction { faction: faction(&words()?[0]), delta: args[0] as i8 }
        }
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
/// A faction by its script name, any case (retail `FindFactionByName`).
fn faction(name: &str) -> Option<Faction> {
    Some(match name.to_ascii_uppercase().as_str() {
        "UNALIGNED" => Faction::Unaligned,
        "FACTION_PARTY" => Faction::Party,
        "FACTION_DARK_SAVANT" => Faction::DarkSavant,
        "FACTION_COSMIC_LORDS" => Faction::CosmicLords,
        "FACTION_UMPANI" => Faction::Umpani,
        "FACTION_TRANG" => Faction::Trang,
        "FACTION_MOOK" => Faction::Mook,
        "FACTION_RATTKIN_COMMON" => Faction::RattkinCommon,
        "FACTION_RATTKIN_MAFIA" => Faction::RattkinMafia,
        "FACTION_BROTHERHOOD" => Faction::Brotherhood,
        "FACTION_HIGARDI_BANK" => Faction::HigardiBank,
        "FACTION_HIGARDI_HLL" => Faction::HigardiLunarLegion,
        "FACTION_HIGARDI_COMMON" => Faction::HigardiCommon,
        "FACTION_TRYNNIE" => Faction::Trynnie,
        "FACTION_MAD_MARTEN" => Faction::MadMarten,
        "FACTION_RAPAX_COMMON" => Faction::RapaxCommon,
        "FACTION_RAPAX_TEMPLAR" => Faction::RapaxTemplar,
        "FACTION_RAPAX_ARMY" => Faction::RapaxArmy,
        "FACTION_KINGS_ASSASINS" => Faction::KingsAssassins,
        _ => return None,
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
                    // The quote belongs to the other speaker's script.
                    Response::Speaker { .. } => vec![],
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
    /// Retail's response chooser: a random range in mode 2, else a bound of
    /// 4 names another speaker, else the first operand is the answer.
    #[test]
    fn decodes_every_response_mode_and_speaker_changes() {
        let named = || vec![(0, Some("NPC_Gadrock".into()))];
        assert_eq!(operation(2, [5, 0, 0], vec![]).unwrap(), DialogueOp::Next(Response::Fixed(5)));
        assert_eq!(operation(2, [5, 3, 7], vec![]).unwrap(), DialogueOp::Next(Response::Fixed(5)));
        assert_eq!(
            operation(2, [5, 2, 7], vec![]).unwrap(),
            DialogueOp::Next(Response::Random { first: 5, last: 7 })
        );
        assert_eq!(
            operation(2, [5, 1, 4], named()).unwrap(),
            DialogueOp::Next(Response::Speaker { npc: "Gadrock".into(), quote: 5 })
        );
        let mut subs = named();
        subs.extend([(91, None), (1, None)]);
        assert_eq!(
            operation(3, [5, 1, 4], subs).unwrap(),
            DialogueOp::Conditional {
                facts: vec![(FactId(91), 1)],
                response: Response::Speaker { npc: "Gadrock".into(), quote: 5 }
            }
        );
        assert!(matches!(operation(2, [5, 1, 4], vec![]).unwrap(), DialogueOp::Unsupported { .. }));
    }

    #[test]
    fn decodes_world_actions_dispositions_remarks_and_keyword_questions() {
        let named = || vec![(0, Some("NPC_Gadrock".into()))];
        assert_eq!(
            operation(13, [0, 0, 0], named()).unwrap(),
            DialogueOp::LeaveParty { npc: Some("Gadrock".into()) }
        );
        assert_eq!(operation(20, [0, 0, 0], vec![]).unwrap(), DialogueOp::LeaveParty { npc: None });
        assert_eq!(
            operation(21, [0xf6, 0, 0], named()).unwrap(),
            DialogueOp::AdjustDisposition { npc: "Gadrock".into(), delta: -10 },
            "the operand is a signed char"
        );
        assert_eq!(operation(14, [0, 0, 0], vec![]).unwrap(), DialogueOp::CloseAndResume);
        assert_eq!(
            operation(12, [0, 0, 0], vec![(0, Some("WalkHome".into()))]).unwrap(),
            DialogueOp::MonsterScriptLabel(Some("WalkHome".into()))
        );
        assert_eq!(
            operation(12, [0, 0, 4], vec![(0, Some("WalkHome".into()))]).unwrap(),
            DialogueOp::MonsterScriptLabel(None)
        );
        assert_eq!(operation(24, [511, 0, 0], vec![]).unwrap(), DialogueOp::PartyRemark(511));
        assert_eq!(
            operation(19, [0, 0, 0], vec![(0, Some("YES".into()))]).unwrap(),
            DialogueOp::KeywordInput(vec!["YES".into()])
        );
        assert!(matches!(
            operation(21, [5, 0, 0], vec![]).unwrap(),
            DialogueOp::Unsupported { .. }
        ));
    }

    #[test]
    fn decodes_item_gifts_removals_and_item_answers() {
        assert_eq!(
            operation(9, [0x1f0, 0, 0], vec![]).unwrap(),
            DialogueOp::GiveItem(ItemId(0x1f0))
        );
        assert_eq!(operation(10, [7, 0, 0], vec![]).unwrap(), DialogueOp::TakeItem(ItemId(7)));
        assert_eq!(
            operation(11, [7, 0, 0], vec![]).unwrap(),
            DialogueOp::ItemQuote { item: ItemId(7), keeps: true }
        );
        assert_eq!(
            operation(15, [7, 0, 0], vec![]).unwrap(),
            DialogueOp::ItemQuote { item: ItemId(7), keeps: false }
        );
        assert!(matches!(
            operation(9, [-1, 0, 0], vec![]).unwrap(),
            DialogueOp::Unsupported { .. }
        ));
    }

    #[test]
    fn decodes_rewards_and_signed_faction_changes_without_guessing_bad_operands() {
        assert_eq!(operation(16, [6000, 0, 0], vec![]).unwrap(), DialogueOp::GiveGold(6000));
        assert_eq!(
            operation(17, [300000, 0, 0], vec![]).unwrap(),
            DialogueOp::GiveExperience(300000)
        );
        assert_eq!(
            operation(22, [-50, 0, 0], vec![(0, Some("faction_umpani".into()))]).unwrap(),
            DialogueOp::AdjustFaction { faction: Some(Faction::Umpani), delta: -50 }
        );
        assert_eq!(
            operation(22, [5, 0, 0], vec![(0, Some("FACTION_HIGARDI".into()))]).unwrap(),
            DialogueOp::AdjustFaction { faction: None, delta: 5 },
            "retail's misspelt name changes nothing"
        );
        assert!(matches!(
            operation(16, [-1, 0, 0], vec![]).unwrap(),
            DialogueOp::Unsupported { .. }
        ));
        assert_eq!(
            operation(22, [0x1fb, 0, 0], vec![(0, Some("FACTION_UMPANI".into()))]).unwrap(),
            DialogueOp::AdjustFaction { faction: Some(Faction::Umpani), delta: -5 },
            "retail truncates the operand to a char"
        );
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
