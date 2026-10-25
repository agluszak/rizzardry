//! One-line notices in the retail text font: trigger messages ("Access
//! Denied."), and "Nothing happens." when using something does nothing.

use bevy::prelude::*;

use crate::content::ui::{SpriteSheet, TextTable};
use crate::game::world::level::Level;
use crate::game::world::triggers::TriggerEffect;

/// Retail's "Nothing happens." in the game's string table.
const NOTHING_HAPPENS: usize = 0x964;
/// How long a notice stays, fading over its last second.
const NOTICE_SECONDS: f32 = 4.0;

#[derive(Resource)]
pub struct NoticeArt {
    font: Handle<SpriteSheet>,
    strings: Handle<TextTable>,
}

pub fn load_notice_art(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(NoticeArt {
        font: assets.load("retail://data/fonts/wiz_text_font.sti"),
        strings: assets.load("retail://data/strings/stringdata.dat"),
    });
}

#[derive(Component)]
pub struct Notice {
    remaining: f32,
}

pub fn show_notices(
    mut commands: Commands,
    mut effects: MessageReader<TriggerEffect>,
    art: Res<NoticeArt>,
    sheets: Res<Assets<SpriteSheet>>,
    tables: Res<Assets<TextTable>>,
    levels: Query<&Level>,
    mut images: ResMut<Assets<Image>>,
    shown: Query<Entity, With<Notice>>,
) {
    let messages = levels.iter().find_map(|level| tables.get(level.messages.as_ref()?));
    let strings = tables.get(&art.strings);
    for effect in effects.read() {
        let text = match effect {
            TriggerEffect::Message(id) => messages.and_then(|m| m.get(usize::try_from(*id).ok()?)),
            TriggerEffect::NothingHappened => strings.and_then(|s| s.get(NOTHING_HAPPENS)),
            // Whizzardry's notice for a locked door without its key.
            TriggerEffect::Locked => Some("Your party doesn't have required key."),
            TriggerEffect::Sound { .. } | TriggerEffect::Shake { .. } => None,
        };
        let (Some(text), Some(font)) = (text, sheets.get(&art.font)) else { continue };
        let text = text.trim();
        info!("notice: {text}");
        for old in &shown {
            commands.entity(old).despawn();
        }
        let image = font.text_image(text);
        let size = image.size_f32();
        commands
            .spawn((
                Notice { remaining: NOTICE_SECONDS },
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(40.0),
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_child((
                ImageNode::new(images.add(image)),
                Node { width: Val::Px(size.x), height: Val::Px(size.y), ..default() },
                Pickable::IGNORE,
            ));
    }
}

pub fn fade_notices(
    mut commands: Commands,
    time: Res<Time>,
    mut notices: Query<(Entity, &mut Notice, &Children)>,
    mut images: Query<&mut ImageNode>,
) {
    for (entity, mut notice, children) in &mut notices {
        notice.remaining -= time.delta_secs();
        if notice.remaining <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        for child in children {
            if let Ok(mut image) = images.get_mut(*child) {
                image.color.set_alpha(notice.remaining.min(1.0));
            }
        }
    }
}
