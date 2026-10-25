//! The party along the bottom of the screen: each member's portrait, name,
//! health and stamina. The dead show their race's skull and the unconscious
//! are dimmed. Retail frames them in its main-screen artwork; that frame is
//! not drawn yet.

use bevy::prelude::*;

use crate::content::ui::SpriteSheet;
use crate::game::characters::{Character, Condition, Race};
use crate::game::party::roster::PartyRoster;

const BAR_WIDTH: f32 = 50.0;

/// One member's place on the party bar.
#[derive(Component, Debug)]
pub struct MemberPanel {
    member: Entity,
    portrait: Handle<SpriteSheet>,
    skull: Handle<SpriteSheet>,
}

/// A bar following one of a member's pools.
#[derive(Component, Debug)]
pub struct PoolBar {
    member: Entity,
    pool: Pool,
}

#[derive(Clone, Copy, Debug)]
enum Pool {
    Health,
    Stamina,
}

/// The skull a dead member's portrait becomes. Which races share the plain
/// skull is inferred from the file names; the selection code was not found.
fn skull(race: Race) -> &'static str {
    match race {
        Race::Lizardman => "_liz",
        Race::Dracon => "_drac",
        Race::Felpurr => "_felp",
        Race::Rawulf => "_raw",
        Race::Mook => "_mook",
        Race::Trynnie => "_tryn",
        Race::TRang => "_trang",
        Race::Umpani => "_ump",
        Race::Rapax => "_rap",
        Race::Android => "_and",
        _ => "",
    }
}

#[derive(Component, Debug)]
pub struct PartyBar;

pub fn spawn_party_bar(
    mut commands: Commands,
    roster: Res<PartyRoster>,
    members: Query<&Character>,
    bars: Query<(), With<PartyBar>>,
    assets: Res<AssetServer>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    mut font: Local<Option<Handle<SpriteSheet>>>,
) {
    if !bars.is_empty() || roster.0.is_empty() {
        return;
    }
    let font = font.get_or_insert_with(|| assets.load("retail://data/fonts/wiz_text_font.sti"));
    let Some(font) = sheets.get(font) else { return };
    let bar = commands
        .spawn((
            PartyBar,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(4.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(6.0),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .id();
    for &member in &roster.0 {
        let Ok(character) = members.get(member) else { continue };
        let name = font.text_image(&character.name);
        let name_size = name.size_f32();
        let panel = commands
            .spawn((
                MemberPanel {
                    member,
                    portrait: assets
                        .load(format!("retail://data/portraits/small/s{}.sti", character.portrait)),
                    skull: assets.load(format!(
                        "retail://data/portraits/small/sskull{}.sti",
                        skull(character.race)
                    )),
                },
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(1.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                Pickable::IGNORE,
                ChildOf(bar),
            ))
            .id();
        commands.spawn((ImageNode::default(), Node::default(), Pickable::IGNORE, ChildOf(panel)));
        commands.spawn((
            ImageNode::new(images.add(name)),
            Node { width: Val::Px(name_size.x), height: Val::Px(name_size.y), ..default() },
            Pickable::IGNORE,
            ChildOf(panel),
        ));
        for (pool, color) in [
            (Pool::Health, Color::srgb(0.75, 0.1, 0.1)),
            (Pool::Stamina, Color::srgb(0.75, 0.65, 0.1)),
        ] {
            let track = commands
                .spawn((
                    Node { width: Val::Px(BAR_WIDTH), height: Val::Px(3.0), ..default() },
                    BackgroundColor(color.darker(0.5)),
                    Pickable::IGNORE,
                    ChildOf(panel),
                ))
                .id();
            commands.spawn((
                PoolBar { member, pool },
                Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
                BackgroundColor(color),
                Pickable::IGNORE,
                ChildOf(track),
            ));
        }
    }
}

/// Show each member's portrait, or skull, once loaded; dim the unconscious.
pub fn update_portraits(
    panels: Query<(&MemberPanel, &Children)>,
    members: Query<&Character>,
    mut images: Query<(&mut ImageNode, &mut Node)>,
    sheets: Res<Assets<SpriteSheet>>,
) {
    for (panel, children) in &panels {
        let Ok(character) = members.get(panel.member) else { continue };
        let shown = if character.is_dead() { &panel.skull } else { &panel.portrait };
        let Some(sheet) = sheets.get(shown) else { continue };
        let (Some(image), Some(frame)) = (sheet.images.first(), sheet.frames.first()) else {
            continue;
        };
        let Some(&first) = children.first() else { continue };
        let Ok((mut node_image, mut node)) = images.get_mut(first) else { continue };
        if node_image.image != *image {
            node_image.image = image.clone();
            node.width = Val::Px(frame.width as f32);
            node.height = Val::Px(frame.height as f32);
        }
        let dim = character.worst_condition() == Some(Condition::Unconscious);
        node_image.color = if dim { Color::srgb(0.4, 0.4, 0.4) } else { Color::WHITE };
    }
}

pub fn update_pool_bars(mut bars: Query<(&PoolBar, &mut Node)>, members: Query<&Character>) {
    for (bar, mut node) in &mut bars {
        if let Ok(character) = members.get(bar.member) {
            let (now, max) = match bar.pool {
                Pool::Health => (character.hp, character.max_hp()),
                Pool::Stamina => (character.stamina, character.max_stamina()),
            };
            let fraction = now as f32 / max.max(1) as f32;
            node.width = Val::Percent(100.0 * fraction.clamp(0.0, 1.0));
        }
    }
}
