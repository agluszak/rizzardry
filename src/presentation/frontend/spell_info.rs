//! Retail spell information, shared by the creator's spell choices.
use super::creator_spells::{SpellArt, SpellOption};
use crate::content::spells::{PowerChoice, SpellId, SpellText, Spellbook};
use crate::content::ui::SpriteSheet;
use crate::presentation::retail_controls::{ControlSkin, FocusRing, ScrollWheel, step_scrollbar};
use crate::presentation::screen_art::{BitmapText, Picture, absolute, spawn_picture};
use bevy::input::keyboard::KeyboardInput;
use bevy::input_focus::{
    FocusCause, FocusedInput, InputFocus,
    tab_navigation::{TabGroup, TabIndex},
};
use bevy::picking::{
    events::{PointerClick, PointerRelease},
    pointer::PointerButton,
};
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button, ModalDialog, SliderValue};

#[derive(Component)]
pub(super) struct SpellInfo {
    id: SpellId,
    effects: Handle<SpellText>,
    descriptions: Handle<SpellText>,
    font: Handle<SpriteSheet>,
}
#[derive(Component)]
pub(super) struct Scroll(Entity);
#[derive(Component)]
pub(super) struct Lines;
#[derive(Component)]
pub(super) struct RealmAnimation {
    timer: Timer,
    frames: usize,
}

pub(super) fn open(
    click: On<PointerClick>,
    options: Query<&SpellOption>,
    page: Single<(Entity, &SpellArt)>,
    existing: Query<(), With<SpellInfo>>,
    assets: Res<AssetServer>,
    mut commands: Commands,
    mut focus: ResMut<InputFocus>,
) {
    if click.button != PointerButton::Secondary || !existing.is_empty() {
        return;
    }
    let Ok(option) = options.get(click.entity) else { return };
    let root = commands
        .spawn((
            SpellInfo {
                id: option.0,
                effects: assets.load(crate::content::paths::SPELL_EFFECTS),
                descriptions: assets.load(crate::content::paths::SPELL_DESCRIPTIONS),
                font: assets.load("retail://data/fonts/wiz_text_font.sti"),
            },
            crate::presentation::retail_controls::RestoreFocus(focus.get()),
            absolute(Vec2::ZERO, Vec2::new(640., 480.)),
            GlobalZIndex(100),
            ModalDialog,
            TabGroup::modal(),
            Pickable::default(),
            ChildOf(page.0),
        ))
        .observe(close_right)
        .observe(close_key)
        .observe(crate::presentation::retail_controls::scroll_wheel)
        .id();
    let background: Handle<SpriteSheet> = assets.load("retail://data/dialogs/popup_spellinfo.sti");
    spawn_picture(&mut commands, root, background, 0, Vec2::new(156., 90.));
    let close = commands
        .spawn((
            Button,
            TabIndex(0),
            FocusRing,
            ControlSkin::new(0, 1, 4, 4, 0),
            Picture {
                sheet: assets.load("retail://data/dialogs/popup_confirmationbuttons.sti"),
                frame: 0,
                at: Vec2::new(438., 350.),
                bounds: Some(0),
            },
            Pickable::default(),
            ChildOf(root),
        ))
        .observe(move |_: On<Activate>, mut commands: Commands| {
            commands.entity(root).try_despawn();
        })
        .id();
    focus.set(close, FocusCause::Navigated);
}
fn close_right(
    event: On<PointerRelease>,
    dialog: Single<Entity, With<SpellInfo>>,
    mut commands: Commands,
) {
    if event.button == PointerButton::Secondary {
        commands.entity(*dialog).despawn();
    }
}
fn close_key(
    event: On<FocusedInput<KeyboardInput>>,
    infos: Query<&SpellInfo>,
    parents: Query<&ChildOf>,
    mut commands: Commands,
) {
    if !event.input.state.is_pressed() || event.input.key_code != KeyCode::Escape {
        return;
    }
    let mut entity = event.focused_entity;
    loop {
        if infos.contains(entity) {
            commands.entity(entity).despawn();
            break;
        }
        let Ok(parent) = parents.get(entity) else { break };
        entity = parent.parent();
    }
}
fn label(
    commands: &mut Commands,
    root: Entity,
    font: &Handle<SpriteSheet>,
    value: String,
    at: Vec2,
    width: f32,
    center: bool,
) {
    commands.spawn((
        BitmapText {
            font: font.clone(),
            value,
            at,
            width,
            justify: if center { bevy::text::Justify::Center } else { bevy::text::Justify::Left },
            ..default()
        },
        ChildOf(root),
    ));
}

pub(super) fn populate(
    // Built once its contents have loaded, which gives it its scrollbar.
    dialogs: Query<(Entity, &SpellInfo), Without<Scroll>>,
    assets: Res<AssetServer>,
    content: If<Res<crate::game::data::Catalogs>>,
    game_strings: crate::presentation::screen_art::GameStrings,
    spell_texts: Res<Assets<SpellText>>,
    sheets: Res<Assets<SpriteSheet>>,
    mut commands: Commands,
) {
    for (root, info) in &dialogs {
        let spell = &content.spells[info.id];
        let (Some(strings), Some(effects), Some(descriptions), Some(font)) = (
            game_strings.table(),
            spell_texts.get(&info.effects),
            spell_texts.get(&info.descriptions),
            sheets.get(&info.font),
        ) else {
            continue;
        };
        let string = |id| strings.get(id).unwrap_or("");
        label(
            &mut commands,
            root,
            &info.font,
            spell.name.clone(),
            Vec2::new(193., 108.),
            231.,
            true,
        );
        label(
            &mut commands,
            root,
            &info.font,
            string(0x118).to_owned(),
            Vec2::new(193., 126.),
            83.,
            true,
        );
        label(
            &mut commands,
            root,
            &info.font,
            spell.level.to_string(),
            Vec2::new(280., 126.),
            22.,
            true,
        );
        let cost_label = if matches!(spell.power, PowerChoice::Fixed) { 0x11a } else { 0x119 };
        label(
            &mut commands,
            root,
            &info.font,
            string(cost_label).to_owned(),
            Vec2::new(193., 141.),
            123.,
            true,
        );
        label(
            &mut commands,
            root,
            &info.font,
            format!("{} {}", spell.cost, string(0x11b)),
            Vec2::new(320., 141.),
            37.,
            true,
        );
        if let Some(realm) = spell.realm {
            let name = ["fire", "water", "air", "earth", "mental", "divine"][realm as usize];
            let icon = spawn_picture(
                &mut commands,
                root,
                assets.load(format!("retail://data/spell casting/{name}_realm.sti")),
                0,
                Vec2::new(170., 104.),
            );
            commands.entity(icon).insert(RealmAnimation {
                timer: Timer::from_seconds(0.05, TimerMode::Repeating),
                frames: [22, 18, 14, 21, 16, 24][realm as usize],
            });
        }
        let books =
            [Spellbook::Wizardry, Spellbook::Divinity, Spellbook::Alchemy, Spellbook::Psionics];
        let names = books
            .into_iter()
            .enumerate()
            .filter(|(_, book)| spell.books.contains(*book))
            .map(|(i, _)| string(791 + i))
            .collect::<Vec<_>>()
            .join(", ");
        let mut entries = vec![(0x11c, names)];
        {
            let usage = spell.usable;
            entries.push((0x11d, string([795, 797, 796, 796, 796][usage as usize]).to_owned()));
        }
        {
            let target = spell.targeting;
            entries.push((
                0x11e,
                string([798, 799, 800, 801, 802, 803, 804, 805, 806, 807, 807][target as usize])
                    .to_owned(),
            ));
        }
        {
            let range = spell.reach;
            entries.push((0x11f, string(1307 + range as usize).to_owned()));
        }
        if let Some(dice) = spell.dice.filter(|_| spell.show_effect_dice) {
            let min = dice.base + dice.count;
            let max = dice.base + dice.count * dice.sides;
            let value =
                if dice.count == 0 { dice.base.to_string() } else { format!("{min}-{max}") };
            entries.push((0x120, format!("{value} {}", string(0x121))));
        }
        // Retail expresses duration in rounds, minutes, hours, or as permanent.
        if spell.duration_base > 0 || spell.duration_per_power > 0 {
            let unit = |value: i32| match value {
                9999 => (0, 0x129),
                ..30 => (value, 0x123),
                30..120 => (value / 6, 0x125),
                _ => (value / 30, 0x127),
            };
            let (per, per_unit) = unit(spell.duration_per_power);
            let (base, base_unit) = unit(spell.duration_base);
            let unit_id = if spell.duration_base > 0 { base_unit } else { per_unit };
            if spell.duration_base == 0 || spell.duration_per_power == 0 || per_unit == base_unit {
                let singular = (spell.duration_per_power == 1 && base == 0)
                    || (spell.duration_per_power == 0 && base == 1);
                let label = if singular && matches!(unit_id, 0x123 | 0x125 | 0x127) {
                    unit_id + 1
                } else {
                    unit_id
                };
                let mut value = String::new();
                if spell.duration_per_power > 0 && per > 0 {
                    value.push_str(&format!("{per} "));
                }
                if spell.duration_per_power > 0 && spell.duration_base > 0 {
                    value.push_str("+ ");
                }
                if spell.duration_base > 0 && base > 0 {
                    value.push_str(&format!("{base} "));
                }
                value.push_str(string(label));
                if base > 0 {
                    value.push_str(string(0x12a));
                }
                entries.push((0x122, value));
            }
        }
        for (id, table) in [(0x12b, effects), (0x12c, descriptions)] {
            if let Some(Some(value)) = table.0.get(info.id.0 as usize)
                && !value.trim().is_empty()
            {
                entries.push((id, value.trim().to_owned()));
            }
        }
        let mut lines = Vec::new();
        for (id, value) in entries {
            let prefix = format!("{}: ", string(id));
            let wrapped =
                crate::presentation::notices::wrap(&format!("{prefix}{value}"), 270, |s| {
                    font.text_width(s)
                });
            let count = wrapped.len();
            for (index, line) in wrapped.into_iter().enumerate() {
                let (heading, body) = if index == 0 && line.starts_with(&prefix) {
                    (Some(prefix.clone()), line[prefix.len()..].to_owned())
                } else {
                    (None, line)
                };
                lines.push((heading, body, index + 1 == count));
            }
        }
        let line_height = font.line_height();
        let viewport = commands
            .spawn((
                Node {
                    overflow: Overflow::clip(),
                    ..absolute(Vec2::new(173., 157.), Vec2::new(270., 185.))
                },
                Pickable::IGNORE,
                ChildOf(root),
            ))
            .id();
        let content = commands
            .spawn((
                Lines,
                absolute(Vec2::ZERO, Vec2::new(270., lines.len() as f32 * (line_height + 1.))),
                Pickable::IGNORE,
                ChildOf(viewport),
            ))
            .id();
        let bronze = assets.load("retail://data/fonts/palettebronze.sti");
        let mut y = 0.;
        for (heading, line, end_entry) in &lines {
            let x = if let Some(heading) = heading {
                commands.spawn((
                    BitmapText {
                        font: info.font.clone(),
                        palette: Some(bronze.clone()),
                        value: heading.clone(),
                        at: Vec2::new(0., y),
                        ..default()
                    },
                    ChildOf(content),
                ));
                font.text_width(heading) as f32
            } else {
                0.
            };
            label(
                &mut commands,
                content,
                &info.font,
                line.clone(),
                Vec2::new(x, y),
                270. - x,
                false,
            );
            y += line_height + if *end_entry { 1. } else { 0. };
        }
        let max = lines.len() - lines.len().min((185. / line_height).floor() as usize);
        spawn_picture(
            &mut commands,
            root,
            assets.load("retail://data/dialogs/popup_spellinfo.sti"),
            1,
            Vec2::new(455., 157.),
        );
        let track = step_scrollbar(
            &mut commands,
            root,
            assets.load("retail://data/main interface/main_scroll.sti"),
            Vec2::new(455., 157.),
            153.,
            max as f32,
        );
        commands.entity(root).insert(Scroll(track)).insert(ScrollWheel(track));
    }
}
pub(super) fn scroll(
    dialogs: Query<&Scroll>,
    sliders: Query<&SliderValue>,
    mut lines: Query<&mut Node, With<Lines>>,
    sheets: Res<Assets<SpriteSheet>>,
    infos: Query<&SpellInfo>,
) {
    let (Ok(dialog), Ok(info)) = (dialogs.single(), infos.single()) else { return };
    let (Ok(value), Some(font)) = (sliders.get(dialog.0), sheets.get(&info.font)) else { return };
    let height = font.line_height();
    for mut node in &mut lines {
        let top = px(-value.0 * height);
        if node.top != top {
            node.top = top;
        }
    }
}

pub(super) fn animate(
    time: Res<Time<Real>>,
    mut icons: Query<(&mut RealmAnimation, &mut Picture)>,
) {
    for (mut animation, mut picture) in &mut icons {
        let steps = animation.timer.tick(time.delta()).times_finished_this_tick() as usize;
        if steps > 0 {
            picture.frame = (picture.frame + steps) % animation.frames;
        }
    }
}
