//! The party along the bottom of the screen: each member's portrait, name
//! and health. Retail frames them in its main-screen artwork; that frame is
//! not drawn yet.

use bevy::prelude::*;

use crate::content::ui::SpriteSheet;
use crate::game::characters::Character;
use crate::game::party::roster::PartyRoster;

const BAR_WIDTH: f32 = 50.0;

/// One member's place on the party bar.
#[derive(Component, Debug)]
pub struct MemberPanel {
    portrait: Handle<SpriteSheet>,
    shown: bool,
}

#[derive(Component, Debug)]
pub struct HealthBar {
    member: Entity,
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
                    portrait: assets
                        .load(format!("retail://data/portraits/small/s{}.sti", character.portrait)),
                    shown: false,
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
        let track = commands
            .spawn((
                Node {
                    width: Val::Px(BAR_WIDTH),
                    height: Val::Px(3.0),
                    margin: UiRect::bottom(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.25, 0.05, 0.05)),
                Pickable::IGNORE,
                ChildOf(panel),
            ))
            .id();
        commands.spawn((
            HealthBar { member },
            Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
            BackgroundColor(Color::srgb(0.75, 0.1, 0.1)),
            Pickable::IGNORE,
            ChildOf(track),
        ));
    }
}

/// Show each portrait once its image has loaded.
pub fn show_portraits(
    mut panels: Query<(&mut MemberPanel, &Children)>,
    mut images: Query<(&mut ImageNode, &mut Node)>,
    sheets: Res<Assets<SpriteSheet>>,
) {
    for (mut panel, children) in &mut panels {
        if panel.shown {
            continue;
        }
        let Some(sheet) = sheets.get(&panel.portrait) else { continue };
        let (Some(image), Some(frame)) = (sheet.images.first(), sheet.frames.first()) else {
            continue;
        };
        let Some(&first) = children.first() else { continue };
        if let Ok((mut node_image, mut node)) = images.get_mut(first) {
            node_image.image = image.clone();
            node.width = Val::Px(frame.width as f32);
            node.height = Val::Px(frame.height as f32);
            panel.shown = true;
        }
    }
}

pub fn update_health_bars(mut bars: Query<(&HealthBar, &mut Node)>, members: Query<&Character>) {
    for (bar, mut node) in &mut bars {
        if let Ok(character) = members.get(bar.member) {
            let fraction = character.hp as f32 / character.max_hp().max(1) as f32;
            node.width = Val::Percent(100.0 * fraction.clamp(0.0, 1.0));
        }
    }
}
