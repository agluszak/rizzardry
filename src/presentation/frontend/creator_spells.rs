//! The retail creation spell lists, grouped by realm.
use super::{Action, Creation, Page, creator_stats};
use crate::content::characters::{Realm, Skill};
use crate::content::spells::SpellId;
use crate::content::ui::SpriteSheet;
use crate::presentation::retail_controls::set_enabled;
use crate::presentation::screen_art::visible_if;
use crate::presentation::screen_art::{BitmapText, absolute, spawn_picture};
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::{Activate, Button};

#[derive(Component)]
pub(super) struct SpellArt {
    selected: Handle<SpriteSheet>,
}
#[derive(Component)]
pub(super) struct SpellOption(pub(super) SpellId);
#[derive(Component)]
pub(super) struct RealmValue(Realm, bool);
#[derive(Component)]
pub(super) struct RealmHeader(Realm);
#[derive(Component)]
pub(super) enum SpellText {
    Name,
    Cost,
}

pub(super) fn build(commands: &mut Commands, root: Entity, assets: &AssetServer) {
    creator_stats::frame(commands, root, assets, Page::Magic);
    let load = |path: &str| assets.load(crate::content::paths::sheet(path));
    let background: Handle<SpriteSheet> = load("char generation/cg_magic");
    let bottom: Handle<SpriteSheet> = load("char generation/cg_bottombuttons");
    let font: Handle<SpriteSheet> = load("fonts/wiz_text_font");
    let blue: Handle<SpriteSheet> = load("fonts/paletteblue");
    commands.entity(root).insert(SpellArt { selected: blue });
    spawn_picture(commands, root, background, 0, Vec2::new(195., 43.));
    let pieces: Handle<SpriteSheet> = load("char generation/cg_pieces");
    spawn_picture(commands, root, pieces, 5, Vec2::new(143., 371.));
    creator_stats::label(
        commands,
        root,
        font.clone(),
        Vec2::new(8., 375.),
        130.,
        bevy::text::Justify::Center,
        creator_stats::Value::String(0xf4),
    );
    for (i, (realm, frame)) in
        [("fire", 3), ("water", 8), ("air", 8), ("earth", 1), ("mental", 14), ("divine", 11)]
            .into_iter()
            .enumerate()
    {
        let x = 195. + (i % 2) as f32 * 215.;
        let y = 43. + (i / 2) as f32 * 130.;
        crate::presentation::retail_controls::step_scrollbar(
            commands,
            root,
            load("main interface/main_scroll"),
            Vec2::new(x + 197., y + 34.),
            62.,
            0.,
        );
        let realm_id = Realm::try_from(i as u8).unwrap();
        let icon = spawn_picture(
            commands,
            root,
            load(&format!("spell casting/{realm}_realm")),
            frame,
            Vec2::new(x + 15., y + 10.),
        );
        commands.entity(icon).insert((RealmHeader(realm_id), Visibility::Hidden));
        for (at, width, mana, string) in [(x + 41., 60., false, 0xf2), (x + 129., 83., true, 0xf3)]
        {
            commands.spawn((
                RealmHeader(realm_id),
                Visibility::Hidden,
                creator_stats::Value::String(string),
                BitmapText {
                    font: font.clone(),
                    at: Vec2::new(at, y + 15.),
                    width,
                    justify: bevy::text::Justify::Left,
                    ..default()
                },
                ChildOf(root),
            ));
            commands.spawn((
                RealmHeader(realm_id),
                Visibility::Hidden,
                RealmValue(realm_id, mana),
                BitmapText {
                    font: font.clone(),
                    at: Vec2::new(at, y + 15.),
                    width,
                    justify: bevy::text::Justify::Right,
                    ..default()
                },
                ChildOf(root),
            ));
        }
    }
    for id in 1..0x72 {
        commands
            .spawn((
                SpellOption(SpellId(id)),
                Button,
                TabIndex(0),
                absolute(Vec2::ZERO, Vec2::new(154., 13.)),
                Visibility::Hidden,
                Pickable::default(),
                ChildOf(root),
            ))
            .observe(select_spell)
            .observe(super::spell_info::open)
            .with_children(|parent| {
                parent.spawn((
                    SpellText::Name,
                    BitmapText { font: font.clone(), width: 154., ..default() },
                ));
                parent.spawn((
                    SpellText::Cost,
                    BitmapText {
                        font: font.clone(),
                        width: 154.,
                        justify: bevy::text::Justify::Right,
                        ..default()
                    },
                ));
            });
    }
    creator_stats::label(
        commands,
        root,
        font,
        Vec2::new(145., 375.),
        45.,
        bevy::text::Justify::Right,
        creator_stats::Value::Total(Page::Magic),
    );
    creator_stats::button(commands, root, bottom.clone(), Vec2::new(464., 450.), 4, Action::Cancel);
    creator_stats::button(
        commands,
        root,
        bottom.clone(),
        Vec2::new(508., 450.),
        20,
        Action::ResetSpells,
    );
    creator_stats::button(
        commands,
        root,
        bottom.clone(),
        Vec2::new(552., 450.),
        12,
        Action::Page(Page::Identity),
    );
    creator_stats::button(
        commands,
        root,
        bottom,
        Vec2::new(596., 450.),
        8,
        Action::Page(Page::Skills),
    );
}

fn select_spell(
    event: On<Activate>,
    choices: Query<&SpellOption, Without<InteractionDisabled>>,
    content: If<Res<crate::game::data::Catalogs>>,
    mut session: Single<&mut Creation>,
) {
    let (Ok(choice), Some(draft)) = (choices.get(event.entity), session.mode.draft_mut()) else {
        return;
    };
    draft.toggle_spell(&content.spells, choice.0);
}

pub(super) fn refresh(
    session: Option<Single<Ref<Creation>>>,
    art: Option<Single<&SpellArt>>,
    content: If<Res<crate::game::data::Catalogs>>,
    mut rows: Query<(
        Entity,
        &SpellOption,
        &mut Node,
        &mut Visibility,
        &Children,
        Has<InteractionDisabled>,
    )>,
    mut labels: Query<(&mut BitmapText, Option<&RealmValue>, Option<&SpellText>)>,
    mut headers: Query<(&RealmHeader, &mut Visibility), Without<SpellOption>>,
    mut commands: Commands,
) {
    let (Some(session), Some(art)) = (session, art) else { return };
    let Some(draft) = session.mode.draft() else { return };
    let catalog = &content.spells;
    let profile = draft.preview();
    for (mut label, role, _) in &mut labels {
        let Some(role) = role else { continue };
        let skill = Skill::try_from(Skill::FireMagic as u8 + role.0 as u8).unwrap();
        let value = if role.1 {
            let learned = crate::game::magic::LearnedSpells(draft.spells());
            let points = profile.base_stats().max_points(&learned, catalog)[role.0];
            format!("{points}/{points}")
        } else {
            profile.skill_points[skill].to_string()
        };
        if label.value != value {
            label.value = value;
        }
    }
    let mut counts = [0usize; 6];
    // Entity iteration order is not a rule: use the retail spell table order.
    let mut entities: Vec<_> = rows.iter().map(|(entity, option, ..)| (option.0, entity)).collect();
    entities.sort_by_key(|(id, _)| id.0);
    for (id, entity) in entities {
        let Ok((_, _, mut node, mut visibility, children, disabled)) = rows.get_mut(entity) else {
            continue;
        };
        let Some(spell) = catalog.get(id).filter(|_| draft.can_learn_spell(catalog, id)) else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        let realm = spell.realm.unwrap() as usize;
        let index = counts[realm];
        counts[realm] += 1;
        // Retail shows seven rows per realm. Creation has at most seven eligible level-one spells per realm.
        let shown = index < 7;
        visibility.set_if_neq(visible_if(shown));
        let left = px(195. + 13. + (realm % 2) as f32 * 215. + 26.);
        let top = px(43. + 8. + (realm / 2) as f32 * 130. + 26. + index as f32 * 13.);
        if node.left != left {
            node.left = left;
        }
        if node.top != top {
            node.top = top;
        }
        for child in children.iter() {
            if let Ok((mut label, _, Some(role))) = labels.get_mut(child) {
                let value = match role {
                    SpellText::Name => spell.name.clone(),
                    SpellText::Cost => spell.cost.to_string(),
                };
                if label.value != value {
                    label.value = value;
                }
                let palette = draft.knows_spell(id).then(|| art.selected.clone());
                if label.palette != palette {
                    label.palette = palette;
                }
            }
        }
        set_enabled(&mut commands, entity, true, disabled);
    }
    for (header, mut visibility) in &mut headers {
        visibility.set_if_neq(visible_if(counts[header.0 as usize] > 0));
    }
}
