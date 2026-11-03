//! Campaign facts and their journal, independent of any active speaker.
use crate::content::npcs::*;
use bevy::prelude::*;
use std::collections::BTreeMap;
#[derive(Resource, Debug, Default)]
pub struct StoryFacts {
    values: BTreeMap<FactId, u8>,
    pub topics: Vec<(TopicCategory, String)>,
    pub journal: Vec<JournalEntry>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    pub fact: FactId,
    pub was_true: bool,
    pub level: String,
}
impl JournalEntry {
    pub fn description<'a>(&self, catalog: &'a FactCatalog) -> &'a str {
        let d = &catalog[self.fact];
        if self.was_true { &d.alternate_description } else { &d.description }
    }
}
impl StoryFacts {
    pub fn remember_topic(&mut self, category: TopicCategory, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() || self.topics.iter().any(|(_, s)| s.eq_ignore_ascii_case(text)) {
            return false;
        }
        self.topics.push((category, text.to_owned()));
        true
    }
    pub fn forget_topic(&mut self, text: &str) -> bool {
        let before = self.topics.len();
        self.topics.retain(|(_, s)| !s.eq_ignore_ascii_case(text.trim()));
        self.topics.len() != before
    }
    pub fn get(&self, id: FactId, catalog: &FactCatalog) -> Result<u8, String> {
        let d = catalog.get(id).ok_or_else(|| format!("unknown fact {id:?}"))?;
        // Only audited stored facts. Derived facts require their owning gameplay state.
        match d.symbol.as_str() {
            "FACT_VIRGIN"
            | "FACT_QUEST_VIRGIN_1"
            | "FACT_QUEST_VIRGIN_2"
            | "FACT_QUEST_VIRGIN_3" => Ok(*self.values.get(&id).unwrap_or(&1)),
            "FACT_IMPORT"
            | "FACT_BURZ_SAID130"
            | "FACT_TEMPLAR"
            | "FACT_UMISSION_SCUBA_DONE"
            | "FACT_SEXUS_PAID"
            | "FACT_TRYNNIE_MADRAS_WILL_JOIN"
            | "FACT_PEACE_ACHIEVED" => Ok(*self.values.get(&id).unwrap_or(&0)),
            name => Err(format!("unimplemented fact predicate: {name}")),
        }
    }
    pub fn writable(id: FactId, catalog: &FactCatalog) -> bool {
        catalog
            .get(id)
            .is_some_and(|d| matches!(d.symbol.as_str(), "FACT_BURZ_SAID130" | "FACT_TEMPLAR"))
    }
    pub fn set(
        &mut self,
        id: FactId,
        value: u8,
        catalog: &FactCatalog,
        level: &str,
    ) -> Result<(), String> {
        if !Self::writable(id, catalog) {
            return Err(format!("unimplemented fact consequences: {id:?}"));
        }
        let previous = self.get(id, catalog)?;
        self.values.insert(id, value);
        if previous != value {
            self.journal.push(JournalEntry { fact: id, was_true: value != 0, level: level.into() });
        }
        Ok(())
    }
}
pub fn fact_named(story: &StoryFacts, catalog: &FactCatalog, name: &str) -> Result<u8, String> {
    let id = catalog
        .0
        .iter()
        .find(|d| d.symbol == name)
        .ok_or_else(|| format!("missing fact {name}"))?
        .id;
    story.get(id, catalog)
}
