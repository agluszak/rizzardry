//! Headless scenarios across the game's plugins, kept in the library's own
//! test binary so the suite builds the crate once and links no game binary.

mod level_lifecycle;
mod retail_rules;
mod scenarios;

/// What presentation is quoted for a trade, for tests that then confirm it.
fn quote(
    app: &mut bevy::prelude::App,
    session: bevy::prelude::Entity,
    source: crate::game::npcs::trade::TradeSource,
) -> crate::game::npcs::trade::TradeQuote {
    use crate::game::npcs::conversation::Trading;
    use bevy::ecs::system::RunSystemOnce;
    app.world_mut()
        .run_system_once(move |trading: Trading| trading.quote(session, source, 1))
        .unwrap()
        .unwrap()
}
