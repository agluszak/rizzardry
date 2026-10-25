//! One-line notices in the retail text font: trigger messages ("Access
//! Denied."), and "Nothing happens." when using something does nothing.

use std::collections::VecDeque;

use bevy::prelude::*;

use crate::content::ui::{SpriteSheet, TextTable};
use crate::game::world::level::Level;
use crate::game::world::triggers::TriggerEffect;

/// Retail's "Nothing happens." in the game's string table.
const NOTHING_HAPPENS: usize = 0x964;
/// How long a notice stays, fading over its last second.
const NOTICE_SECONDS: f32 = 4.0;
/// How long a notice stays when others wait.
const HURRIED_SECONDS: f32 = 1.5;

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

/// Notices waiting their turn; retail shows them one after another.
#[derive(Resource, Debug, Default)]
pub struct NoticeQueue(VecDeque<String>);

/// "Party takes %s", and the two halves of the gold notice.
const PARTY_TAKES: usize = 0x1e9;
const GOLD_BEFORE: usize = 0x15f;
const GOLD_AFTER: usize = 0x160;

/// Turn trigger effects into notice texts.
pub fn queue_notices(
    mut effects: MessageReader<TriggerEffect>,
    art: Res<NoticeArt>,
    tables: Res<Assets<TextTable>>,
    levels: Query<&Level>,
    mut queue: ResMut<NoticeQueue>,
) {
    let messages = levels.iter().find_map(|level| tables.get(level.messages.as_ref()?));
    let strings = tables.get(&art.strings);
    let string = |index| strings.and_then(|s| s.get(index)).unwrap_or_default().trim().to_owned();
    for effect in effects.read() {
        let text = match effect {
            TriggerEffect::Message(id) => {
                messages.and_then(|m| m.get(usize::try_from(*id).ok()?)).map(str::to_owned)
            }
            TriggerEffect::NothingHappened => Some(string(NOTHING_HAPPENS)),
            // Whizzardry's notice for a locked door without its key.
            TriggerEffect::Locked => Some("Your party doesn't have required key.".to_owned()),
            TriggerEffect::PartyTakes(item) => Some(string(PARTY_TAKES).replace("%s", item)),
            TriggerEffect::Gold(gold) => {
                Some(format!("{} {gold} {}.", string(GOLD_BEFORE), string(GOLD_AFTER)))
            }
            TriggerEffect::Sound { .. } | TriggerEffect::Shake { .. } => None,
        };
        if let Some(text) = text.filter(|t| !t.trim().is_empty()) {
            info!("notice: {}", text.trim());
            queue.0.push_back(text.trim().to_owned());
        }
    }
}

/// Show the next notice once the last is gone; waiting ones hurry it.
pub fn show_notices(
    mut commands: Commands,
    mut queue: ResMut<NoticeQueue>,
    art: Res<NoticeArt>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    mut shown: Query<&mut Notice>,
) {
    if let Ok(mut current) = shown.single_mut() {
        if !queue.0.is_empty() {
            current.remaining = current.remaining.min(HURRIED_SECONDS);
        }
        return;
    }
    let Some(font) = sheets.get(&art.font) else { return };
    let Some(text) = queue.0.pop_front() else { return };
    let image = font.text_image(&text);
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
