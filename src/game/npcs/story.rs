//! Campaign facts, journal and keyword notebook.
use crate::content::npcs::*;
use bevy::prelude::*;
use std::collections::BTreeMap;

/// Engine semantics refer to the database's symbolic keys; script operands and
/// campaign storage keep the actual decoded identifiers. No Burz record or
/// fact number is embedded in the interpreter.
#[derive(Resource, Debug, Default)]
pub struct StoryFacts {
    values: BTreeMap<FactId, u8>,
    pub topics: Vec<(TopicCategory, String)>,
    pub journal: Vec<JournalEntry>,
}
/// A fact transition, retaining the level it happened on and whether the
/// fact became true, which picks its description (retail `W8JournalEntry`'s
/// `alternate_text`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    pub fact: FactId,
    pub was_true: bool,
    pub level: String,
}
impl JournalEntry {
    pub fn description<'a>(&self, catalog: &'a FactCatalog) -> &'a str {
        let fact = &catalog[self.fact];
        if self.was_true { &fact.alternate_description } else { &fact.description }
    }
}
impl StoryFacts {
    /// The keyword notebook outlives a conversation. Learning/discussing a
    /// keyword does not consume it; only the explicit minus button removes it.
    pub fn remember_topic(&mut self, category: TopicCategory, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() || self.topics.iter().any(|(_, known)| known.eq_ignore_ascii_case(text))
        {
            return false;
        }
        self.topics.push((category, text.to_owned()));
        true
    }

    /// Forget a notebook entry without changing discovered facts or journal history.
    pub fn forget_topic(&mut self, text: &str) -> bool {
        let text = text.trim();
        let before = self.topics.len();
        self.topics.retain(|(_, known)| !known.eq_ignore_ascii_case(text));
        self.topics.len() != before
    }

    /// A fact's value before anything sets it: a new game starts its
    /// virgin facts true.
    fn initial(fact: &crate::content::npcs::FactDefinition) -> u8 {
        u8::from(matches!(
            fact.symbol.as_str(),
            "FACT_VIRGIN" | "FACT_QUEST_VIRGIN_1" | "FACT_QUEST_VIRGIN_2" | "FACT_QUEST_VIRGIN_3"
        ))
    }
    pub fn get(&self, id: FactId, catalog: &FactCatalog) -> Result<u8, String> {
        let fact = catalog.get(id).ok_or_else(|| format!("missing fact definition {id:?}"))?;
        match fact.symbol.as_str() {
            // Plain stored facts (retail `EvaluateFact` reads them unchanged).
            "FACT_VIRGIN"
            | "FACT_IMPORT"
            | "FACT_BURZ_SAID130"
            | "FACT_TEMPLAR"
            | "FACT_UMISSION_SCUBA_DONE"
            | "FACT_SEXUS_PAID"
            | "FACT_TRYNNIE_MADRAS_WILL_JOIN"
            | "FACT_PEACE_ACHIEVED" => {
                Ok(self.values.get(&id).copied().unwrap_or(Self::initial(fact)))
            }
            name => Err(format!("unported fact predicate {name}")),
        }
    }
    pub(super) fn can_set(id: FactId, catalog: &FactCatalog) -> bool {
        catalog
            .get(id)
            .is_some_and(|f| matches!(f.symbol.as_str(), "FACT_BURZ_SAID130" | "FACT_TEMPLAR"))
    }
    pub(super) fn set(
        &mut self,
        id: FactId,
        value: u8,
        catalog: &FactCatalog,
        level: String,
    ) -> Result<(), String> {
        if !Self::can_set(id, catalog) {
            return Err(format!("unported fact assignment {id:?}"));
        }
        let initial = catalog.get(id).map_or(0, Self::initial);
        let previous = self.values.insert(id, value).unwrap_or(initial);
        if previous != value {
            self.journal.push(JournalEntry { fact: id, was_true: value != 0, level });
        }
        Ok(())
    }
}
/// A fact by its symbol, for rules that name one.
pub fn fact_named(story: &StoryFacts, catalog: &FactCatalog, symbol: &str) -> Result<u8, String> {
    let id = catalog
        .0
        .iter()
        .find(|fact| fact.symbol == symbol)
        .ok_or_else(|| format!("missing fact {symbol}"))?
        .id;
    story.get(id, catalog)
}
