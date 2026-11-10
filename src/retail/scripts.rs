//! Monster scripts: the text files under `data/monsters/scripts`.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;

use super::LoadError;
use crate::content::scripts::MonsterScript;

#[derive(Default, TypePath)]
pub struct MonsterScriptLoader;

impl AssetLoader for MonsterScriptLoader {
    type Asset = MonsterScript;
    type Settings = ();
    type Error = LoadError;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<MonsterScript, LoadError> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(MonsterScript::parse(&String::from_utf8_lossy(&bytes)))
    }
    fn extensions(&self) -> &[&str] {
        &["msf"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::scripts::ScriptStep;

    /// Every shipped script reads, and every jump in it lands inside it or
    /// at its end.
    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_scripts_read() {
        let files = crate::retail::files::retail_files();
        let names: Vec<&str> = files.names().filter(|name| name.ends_with(".msf")).collect();
        assert!(names.len() > 40, "{}", names.len());
        for name in names {
            let text = files.read(name).unwrap().unwrap();
            let script = MonsterScript::parse(&String::from_utf8_lossy(&text));
            assert!(!script.steps.is_empty() || script.orders.is_some(), "{name}");
            for step in &script.steps {
                if let ScriptStep::Goto(to) = step {
                    assert!(*to <= script.steps.len(), "{name}");
                }
            }
        }
    }
}
