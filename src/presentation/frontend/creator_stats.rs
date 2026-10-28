//! Retail profession and attributes page of character creation.
use super::{Action, Creation, CreatorChoice, Page};
use crate::content::characters::{Attribute, Gender, Profession, Realm, Skill};
use crate::content::spells::SpellCatalog;
use crate::content::ui::{SpriteSheet, TextTable};
use crate::presentation::retail_controls::{ControlSkin, FocusHighlight, set_enabled};
use crate::presentation::screen_art::visible_if;
use crate::presentation::screen_art::{BitmapText, Picture, spawn_picture};
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::Button;
use enumset::EnumSet;

#[derive(Component)]
pub(super) enum Value {
    String(usize),
    Title(Page),
    Help(Page, usize),
    LimitLabel(Page, usize),
    TotalLabel(usize),
    Wrapped(usize, usize, u16),
    Error(usize),
    Profession,
    Race,
    Gender,
    Attribute(Attribute),
    Allocated(Attribute),
    SkillName(Skill),
    Skill(Skill),
    AllocatedSkill(Skill),
    Remaining,
    Limit(Page),
    Total(Page),
    Summary(usize),
    Health,
    Stamina,
    Capacity,
    Armour,
    SpellPoints,
    SkillTotal,
    Resistance(Realm),
    Trait(usize),
    ProfessionSkill(usize),
}
#[derive(Component)]
pub(super) enum Icon {
    Profession,
    Race,
    Gender,
}

#[derive(Component)]
pub(super) struct StatsAssets {
    strings: Handle<TextTable>,
    spells: Handle<SpellCatalog>,
    font: Handle<SpriteSheet>,
    base_icons: Handle<SpriteSheet>,
    profession_icons: Handle<SpriteSheet>,
    race_icons: Handle<SpriteSheet>,
    gender_icons: Handle<SpriteSheet>,
    blue: Handle<SpriteSheet>,
}

#[derive(Component)]
pub(super) struct StatsControl;
#[derive(Component)]
pub(super) struct ChoiceMenu(CreatorChoice);

pub(super) fn label(
    commands: &mut Commands,
    parent: Entity,
    font: Handle<SpriteSheet>,
    at: Vec2,
    width: f32,
    justify: bevy::text::Justify,
    value: Value,
) {
    commands.spawn((value, BitmapText { font, at, width, justify, ..default() }, ChildOf(parent)));
}
pub(super) fn button(
    commands: &mut Commands,
    parent: Entity,
    sheet: Handle<SpriteSheet>,
    at: Vec2,
    normal: usize,
    action: Action,
) {
    commands.spawn((
        Button,
        TabIndex(0),
        FocusHighlight,
        StatsControl,
        ControlSkin::new(normal, normal + 1, normal + 2, normal + 2, normal + 3),
        Picture { sheet, frame: normal, bounds: Some(normal), at },
        Pickable::default(),
        action,
        ChildOf(parent),
    ));
}

/// The creator's shared retail frame; each retained page owns its own controls.
pub(super) fn frame(commands: &mut Commands, parent: Entity, assets: &AssetServer, page: Page) {
    let pieces: Handle<SpriteSheet> = assets.load("retail://data/char generation/cg_pieces.sti");
    let font: Handle<SpriteSheet> = assets.load("retail://data/fonts/wiz_text_font.sti");
    let heading: Handle<SpriteSheet> = assets.load("retail://data/fonts/opt_detail_font.sti");
    for (frame, x, y) in [(0, 195., 0.), (1, 0., 0.), (2, 10., 12.), (4, 0., 450.)] {
        spawn_picture(commands, parent, pieces.clone(), frame, Vec2::new(x, y));
    }
    if page == Page::Identity {
        spawn_picture(commands, parent, pieces.clone(), 3, Vec2::new(5., 165.));
    } else {
        for i in 0..3 {
            label(
                commands,
                parent,
                font.clone(),
                Vec2::new(5., 180. + i as f32 * 14.),
                188.,
                bevy::text::Justify::Center,
                Value::Summary(i),
            );
        }
    }
    label(
        commands,
        parent,
        heading,
        Vec2::new(195., 14.),
        445.,
        bevy::text::Justify::Center,
        Value::Title(page),
    );
    for i in 0..5 {
        label(
            commands,
            parent,
            font.clone(),
            Vec2::new(8., 280. + i as f32 * 11.),
            180.,
            bevy::text::Justify::Center,
            Value::Help(page, i),
        );
    }
    if page == Page::Review {
        for line in 0..3 {
            label(
                commands,
                parent,
                font.clone(),
                Vec2::new(8., 412. + line as f32 * 11.),
                180.,
                bevy::text::Justify::Center,
                Value::Error(line),
            );
        }
    }
    if page != Page::Review && page != Page::Magic {
        for y in [354., 388.] {
            spawn_picture(commands, parent, pieces.clone(), 5, Vec2::new(143., y));
        }
        for line in 0..2 {
            label(
                commands,
                parent,
                font.clone(),
                Vec2::new(8., 354. + line as f32 * 11.),
                130.,
                bevy::text::Justify::Center,
                Value::LimitLabel(page, line),
            );
            label(
                commands,
                parent,
                font.clone(),
                Vec2::new(8., 388. + line as f32 * 11.),
                130.,
                bevy::text::Justify::Center,
                Value::TotalLabel(line),
            );
        }
        label(
            commands,
            parent,
            font.clone(),
            Vec2::new(145., 358.),
            45.,
            bevy::text::Justify::Right,
            Value::Limit(page),
        );
        label(
            commands,
            parent,
            font,
            Vec2::new(145., 392.),
            45.,
            bevy::text::Justify::Right,
            Value::Total(page),
        );
    }
}

pub(super) fn build(commands: &mut Commands, parent: Entity, assets: &AssetServer) {
    let load = |path: &str| assets.load(crate::content::paths::sheet(path));
    let stats = load("char generation/cg_profession");
    let controls = load("char generation/cg_buttons");
    let bottom = load("char generation/cg_bottombuttons");
    let font = load("fonts/wiz_text_font");
    let heading = load("fonts/opt_detail_font");
    let base_icons = load("char generation/cg_icons_base");
    let profession_icons = load("char generation/cg_icons_profession");
    let race_icons = load("char generation/cg_icons_race");
    let gender_icons = load("char generation/cg_icons_gender");
    commands.entity(parent).insert(StatsAssets {
        strings: assets.load(crate::content::paths::STRINGS),
        spells: assets.load(crate::content::paths::SPELLS),
        font: font.clone(),
        base_icons: base_icons.clone(),
        profession_icons: profession_icons.clone(),
        race_icons: race_icons.clone(),
        gender_icons: gender_icons.clone(),
        blue: load("fonts/paletteblue"),
    });
    frame(commands, parent, assets, Page::Identity);
    spawn_picture(commands, parent, stats.clone(), 0, Vec2::new(195., 43.));
    use bevy::text::Justify::{Center, Left, Right};
    for (id, x, y, width, justify) in [
        (0x96, 424., 57., 195., Center),
        (0x87, 473., 169., 143., Left),
        (0xb1, 221., 215., 202., Center),
        (0xa6, 438., 215., 180., Center),
        (0xa7, 443., 229., 110., Left),
        (0xa9, 443., 243., 110., Left),
        (0xed, 443., 257., 110., Left),
        (0xad, 443., 271., 110., Left),
        (0xaf, 443., 285., 110., Left),
        (0xb3, 438., 331., 180., Center),
        (0xb9, 221., 363., 202., Center),
        (0x87, 487., 420., 125., Left),
    ] {
        label(commands, parent, font.clone(), Vec2::new(x, y), width, justify, Value::String(id));
    }
    for (field, x, y, width) in [
        (Value::Remaining, 445., 169., 22.),
        (Value::Health, 559., 229., 39.),
        (Value::Stamina, 559., 243., 39.),
        (Value::SpellPoints, 559., 257., 39.),
        (Value::Armour, 559., 271., 39.),
        (Value::Capacity, 559., 285., 39.),
        (Value::SkillTotal, 454., 420., 22.),
    ] {
        label(commands, parent, font.clone(), Vec2::new(x, y), width, Right, field);
    }
    for i in 0..7 {
        label(
            commands,
            parent,
            font.clone(),
            Vec2::new(224., 230. + i as f32 * 14.),
            195.,
            Left,
            Value::Trait(i),
        );
    }
    for i in 0..5 {
        label(
            commands,
            parent,
            font.clone(),
            Vec2::new(441., 345. + i as f32 * 14.),
            170.,
            Left,
            Value::ProfessionSkill(i),
        );
    }
    for (i, attribute) in EnumSet::<Attribute>::all().iter().enumerate() {
        let y = 70. + 14. * i as f32;
        label(
            commands,
            parent,
            font.clone(),
            Vec2::new(425., y),
            120.,
            Left,
            Value::String(0x274 + i),
        );
        label(
            commands,
            parent,
            font.clone(),
            Vec2::new(546., y),
            21.,
            Right,
            Value::Attribute(attribute),
        );
        label(
            commands,
            parent,
            font.clone(),
            Vec2::new(586., y),
            14.,
            Right,
            Value::Allocated(attribute),
        );
        button(
            commands,
            parent,
            controls.clone(),
            Vec2::new(570., y),
            25,
            Action::Attribute(attribute, false),
        );
        button(
            commands,
            parent,
            controls.clone(),
            Vec2::new(604., y),
            30,
            Action::Attribute(attribute, true),
        );
    }
    for (i, (name, icon, left, right)) in [
        (Value::Profession, Icon::Profession, Action::Profession(-1), Action::Profession(1)),
        (Value::Race, Icon::Race, Action::Race(-1), Action::Race(1)),
        (Value::Gender, Icon::Gender, Action::Gender(-1), Action::Gender(1)),
    ]
    .into_iter()
    .enumerate()
    {
        let y = 57. + 51. * i as f32;
        spawn_picture(commands, parent, stats.clone(), 1, Vec2::new(244., y));
        let e = spawn_picture(
            commands,
            parent,
            base_icons.clone(),
            [2, 0, 4][i],
            Vec2::new(246., y + 2.),
        );
        commands.entity(e).insert(icon);
        label(commands, parent, heading.clone(), Vec2::new(270., y + 5.), 95., Left, name);
        button(commands, parent, controls.clone(), Vec2::new(218., y), 0, left);
        button(commands, parent, controls.clone(), Vec2::new(376., y), 5, right);
        let choice = [CreatorChoice::Profession, CreatorChoice::Race, CreatorChoice::Gender][i];
        commands.spawn((
            Button,
            TabIndex(0),
            crate::presentation::retail_controls::FocusRing,
            StatsControl,
            Action::ToggleChoice(choice),
            super::absolute(Vec2::new(244., y), Vec2::new(126., 24.)),
            Pickable::default(),
            ChildOf(parent),
        ));
    }
    for (i, realm) in ["fire", "water", "air", "earth", "mental", "divine"].into_iter().enumerate()
    {
        let x = 230. + (i / 2) as f32 * 63.;
        let y = 384. + (i % 2) as f32 * 26.;
        let frame = [3, 8, 8, 1, 14, 11][i];
        spawn_picture(
            commands,
            parent,
            load(&format!("spell casting/{realm}_realm")),
            frame,
            Vec2::new(x, y),
        );
        label(
            commands,
            parent,
            font.clone(),
            Vec2::new(x + 22., y + 2.),
            33.,
            Center,
            Value::Resistance(
                [Realm::Fire, Realm::Water, Realm::Air, Realm::Earth, Realm::Mental, Realm::Divine]
                    [i],
            ),
        );
    }
    button(commands, parent, bottom.clone(), Vec2::new(464., 450.), 4, Action::Cancel);
    button(commands, parent, bottom.clone(), Vec2::new(508., 450.), 20, Action::ResetAttributes);
    spawn_picture(commands, parent, bottom.clone(), 15, Vec2::new(552., 450.));
    button(commands, parent, bottom, Vec2::new(596., 450.), 8, Action::Page(Page::Skills));
    for (choice, y) in [
        (CreatorChoice::Profession, 57.),
        (CreatorChoice::Race, 108.),
        (CreatorChoice::Gender, 159.),
    ] {
        let menu = commands
            .spawn((
                ChoiceMenu(choice),
                super::absolute(Vec2::new(375., y), Vec2::new(126., 330.)),
                Visibility::Hidden,
                ZIndex(10),
                Pickable::IGNORE,
                ChildOf(parent),
            ))
            .id();
        let entries: Vec<(Action, usize, usize, Handle<SpriteSheet>)> = match choice {
            CreatorChoice::Profession => EnumSet::<Profession>::all()
                .iter()
                .map(|profession| {
                    let index = profession as usize;
                    (
                        Action::ChooseProfession(profession),
                        crate::retail::strings::PROFESSION_NAMES + index,
                        [10, 24, 8, 2, 16, 20, 12, 28, 6, 0, 18, 22, 4, 14, 26][index],
                        profession_icons.clone(),
                    )
                })
                .collect(),
            CreatorChoice::Race => super::PLAYABLE_RACES
                .into_iter()
                .map(|race| {
                    let index = race as usize;
                    (
                        Action::ChooseRace(race),
                        crate::retail::strings::RACE_NAMES + index,
                        [8, 4, 12, 16, 14, 20, 0, 10, 2, 18, 6][index],
                        race_icons.clone(),
                    )
                })
                .collect(),
            CreatorChoice::Gender => [Gender::Male, Gender::Female]
                .into_iter()
                .map(|gender| {
                    (
                        Action::ChooseGender(gender),
                        crate::retail::strings::GENDER_NAMES + gender as usize,
                        2 * gender as usize,
                        gender_icons.clone(),
                    )
                })
                .collect(),
        };
        for (index, (action, id, frame, sheet)) in entries.into_iter().enumerate() {
            let row = commands
                .spawn((
                    Button,
                    TabIndex(0),
                    FocusHighlight,
                    StatsControl,
                    action,
                    ControlSkin::new(3, 2, 2, 2, 3),
                    Picture {
                        sheet: stats.clone(),
                        frame: 3,
                        bounds: Some(3),
                        at: Vec2::new(0., index as f32 * 22.),
                    },
                    Pickable::default(),
                    ChildOf(menu),
                ))
                .id();
            spawn_picture(commands, row, sheet, frame, Vec2::new(2., 1.));
            label(commands, row, heading.clone(), Vec2::new(26., 4.), 98., Left, Value::String(id));
        }
    }
}

pub(super) fn refresh(
    session: Option<Single<Ref<Creation>>>,
    art: Option<Single<&StatsAssets>>,
    tables: Res<Assets<TextTable>>,
    spells: Res<Assets<SpellCatalog>>,
    sheets: Res<Assets<SpriteSheet>>,
    mut labels: Query<(&Value, &mut BitmapText)>,
    mut icons: Query<(&Icon, &mut Picture)>,
    controls: Query<(Entity, &Action, Has<InteractionDisabled>), With<StatsControl>>,
    mut menus: Query<(&ChoiceMenu, &mut Node, &mut Visibility)>,
    mut commands: Commands,
) {
    let (Some(session), Some(art)) = (session, art) else { return };
    let Some(draft) = session.mode.draft() else { return };
    let profile = draft.preview();
    let strings = tables.get(&art.strings);
    let string = |id| strings.and_then(|t| t.get(id)).unwrap_or("");
    let wrap = |id, width| {
        sheets
            .get(&art.font)
            .map(|font| {
                crate::presentation::notices::wrap(string(id), width, |s| font.text_width(s))
            })
            .unwrap_or_default()
    };
    let resistances = crate::game::characters::resistance::resistances(
        &profile,
        &crate::game::inventory::Carried::default(),
        &crate::content::items::ItemCatalog::default(),
    );
    let help = [0xe5, 0xe8, 0xe9, 0xea].map(|id| {
        sheets
            .get(&art.font)
            .map(|f| crate::presentation::notices::wrap(string(id), 180, |s| f.text_width(s)))
            .unwrap_or_default()
    });
    for (field, mut label) in &mut labels {
        let value = match *field {
            Value::String(0) => "-".to_owned(),
            Value::String(id) => string(id).to_owned(),
            Value::Title(page) => string(match page {
                Page::Identity => 0xcf,
                Page::Magic => 0xd1,
                Page::Skills => 0xd0,
                Page::Review => 0xd2,
                Page::Party => 0xcf,
            })
            .to_owned(),
            Value::Help(page, i) => help[match page {
                Page::Identity => 0,
                Page::Magic => 3,
                Page::Skills => 1,
                _ => 2,
            }]
            .get(i)
            .cloned()
            .unwrap_or_default(),
            Value::LimitLabel(page, line) => {
                wrap(if page == Page::Skills { 0xe3 } else { 0xe2 }, 130)
                    .get(line)
                    .cloned()
                    .unwrap_or_default()
            }
            Value::TotalLabel(line) => wrap(0xe4, 130).get(line).cloned().unwrap_or_default(),
            Value::Error(line) => sheets
                .get(&art.font)
                .map(|font| {
                    crate::presentation::notices::wrap(&session.message, 180, |s| {
                        font.text_width(s)
                    })
                })
                .and_then(|lines| lines.get(line).cloned())
                .unwrap_or_default(),
            Value::Wrapped(id, line, width) => {
                wrap(id, u32::from(width)).get(line).cloned().unwrap_or_default()
            }
            Value::Profession => string(if session.choices.profession {
                crate::retail::strings::PROFESSION_NAMES + draft.profession() as usize
            } else {
                0x8f
            })
            .to_owned(),
            Value::Race => string(if session.choices.race {
                crate::retail::strings::RACE_NAMES + draft.race() as usize
            } else {
                0x90
            })
            .to_owned(),
            Value::Gender => string(if session.choices.gender {
                crate::retail::strings::GENDER_NAMES + draft.gender() as usize
            } else {
                0x91
            })
            .to_owned(),
            Value::Attribute(a) => {
                if session.choices.race {
                    profile.attribute(a).to_string()
                } else {
                    String::new()
                }
            }
            Value::Allocated(a) => {
                if session.choices.race {
                    draft.allocated_attribute(a).to_string()
                } else {
                    String::new()
                }
            }
            Value::SkillName(skill) => {
                string(crate::retail::strings::SKILL_NAMES + skill as usize).to_owned()
            }
            Value::Skill(skill) => profile.skill_points[skill].to_string(),
            Value::AllocatedSkill(skill) => draft.allocated_skill(skill).to_string(),
            Value::Remaining => {
                if session.choices.profession && session.choices.race {
                    draft.attribute_remaining().to_string()
                } else {
                    String::new()
                }
            }
            Value::Limit(Page::Skills) => "5".into(),
            Value::Limit(_) => {
                if session.choices.profession && session.choices.race {
                    draft.attribute_step_limit().to_string()
                } else {
                    draft.spell_points_total().to_string()
                }
            }
            Value::Total(Page::Magic) => {
                format!("{}/{}", draft.spell_remaining(), draft.spell_points_total())
            }
            Value::Total(Page::Skills) => format!("{}/15", draft.skill_remaining()),
            Value::Total(_) if session.choices.profession && session.choices.race => format!(
                "{}/{}",
                draft.attribute_remaining(),
                draft.attribute_remaining()
                    + EnumSet::<Attribute>::all()
                        .iter()
                        .map(|a| draft.allocated_attribute(a))
                        .sum::<u8>()
            ),
            Value::Total(_) => "0/0".into(),
            Value::Summary(i) => match i {
                0 => format!(
                    "{} {}",
                    string(crate::retail::strings::GENDER_NAMES + draft.gender() as usize),
                    string(crate::retail::strings::RACE_NAMES + draft.race() as usize)
                ),
                1 => string(crate::retail::strings::PROFESSION_NAMES + draft.profession() as usize)
                    .to_owned(),
                _ => format!("{} 1 (Novice)", string(0x6b9)),
            },
            Value::Health => {
                if session.choices.race && session.choices.profession {
                    profile.max_hp().to_string()
                } else if session.choices.race {
                    "0".into()
                } else {
                    String::new()
                }
            }
            Value::Stamina => {
                if session.choices.race {
                    profile.max_stamina().to_string()
                } else {
                    String::new()
                }
            }
            Value::Capacity => {
                if session.choices.race {
                    (profile.capacity() / 10).to_string()
                } else {
                    String::new()
                }
            }
            Value::Armour => {
                if session.choices.race {
                    crate::game::combat::attack::armour(
                        &profile,
                        &crate::game::inventory::Carried::default(),
                        &crate::content::items::ItemCatalog::default(),
                        crate::game::characters::Load::None,
                        false,
                    )
                    .average()
                    .to_string()
                } else {
                    String::new()
                }
            }
            Value::SpellPoints => {
                if session.choices.race {
                    spells
                        .get(&art.spells)
                        .map(|catalog| {
                            EnumSet::<Realm>::all()
                                .iter()
                                .map(|realm| {
                                    crate::game::magic::max_points(
                                        &profile,
                                        &crate::game::magic::LearnedSpells(draft.spells()),
                                        catalog,
                                    )[realm]
                                })
                                .sum::<u32>()
                        })
                        .unwrap_or(0)
                        .to_string()
                } else {
                    String::new()
                }
            }
            Value::SkillTotal => {
                if session.choices.profession {
                    "15".into()
                } else {
                    String::new()
                }
            }
            Value::Resistance(realm) => {
                let amount = if session.choices.race || session.choices.profession {
                    resistances[realm].total - 25
                } else {
                    0
                };
                if amount == 0 { "-".into() } else { format!("{amount:+}") }
            }
            Value::Trait(i) => {
                let mut lines = Vec::new();
                if session.choices.profession {
                    lines.push(format!(
                        "{} {}",
                        string(
                            crate::retail::strings::SKILL_NAMES + profile.bonus_skill() as usize
                        ),
                        string(0xb2)
                    ));
                }
                let mut abilities = enumset::EnumSet::empty();
                if session.choices.profession {
                    abilities |= profile.profession_traits();
                }
                if session.choices.race {
                    abilities |= profile.racial_traits();
                }
                lines.extend(abilities.iter().map(|ability| {
                    string(crate::retail::strings::ABILITY_NAMES + ability as usize).to_owned()
                }));
                lines.get(i).cloned().unwrap_or_default()
            }
            Value::ProfessionSkill(i) => {
                if !session.choices.profession {
                    String::new()
                } else {
                    let skill = if i == 0 {
                        Some(profile.bonus_skill())
                    } else {
                        profile.profession_skills().get(i - 1).copied()
                    };
                    skill
                        .map(|s| {
                            string(crate::retail::strings::SKILL_NAMES + s as usize).to_owned()
                        })
                        .unwrap_or_default()
                }
            }
        };
        if label.value != value {
            label.value = value;
        }
        if let Value::SkillName(skill) = *field {
            let palette = (skill == profile.bonus_skill()).then(|| art.blue.clone());
            if label.palette != palette {
                label.palette = palette;
            }
        }
        if matches!(*field, Value::ProfessionSkill(0)) {
            let palette = Some(art.blue.clone());
            if label.palette != palette {
                label.palette = palette;
            }
        }
    }
    for (icon, mut picture) in &mut icons {
        let (sheet, frame) = match icon {
            Icon::Profession => {
                if session.choices.profession {
                    (
                        &art.profession_icons,
                        [10, 24, 8, 2, 16, 20, 12, 28, 6, 0, 18, 22, 4, 14, 26]
                            [draft.profession() as usize],
                    )
                } else {
                    (&art.base_icons, 2)
                }
            }
            Icon::Race => {
                if session.choices.race {
                    (
                        &art.race_icons,
                        [8, 4, 12, 16, 14, 20, 0, 10, 2, 18, 6][draft.race() as usize],
                    )
                } else {
                    (&art.base_icons, 0)
                }
            }
            Icon::Gender => {
                if session.choices.gender {
                    (&art.gender_icons, if draft.gender() == Gender::Male { 0 } else { 2 })
                } else {
                    (&art.base_icons, 4)
                }
            }
        };
        if picture.sheet != *sheet {
            picture.sheet = sheet.clone();
        }
        if picture.frame != frame {
            picture.frame = frame;
        }
    }
    for (entity, action, disabled) in &controls {
        let enabled = match *action {
            Action::Attribute(attribute, increase) => {
                session.choices.complete() && draft.can_adjust_attribute(attribute, increase)
            }
            Action::Page(Page::Skills) if session.mode.page() == Page::Magic => {
                draft.spell_remaining() < draft.spell_points_total()
            }
            Action::Page(Page::Skills) => {
                session.choices.complete()
                    && !EnumSet::<Attribute>::all()
                        .iter()
                        .any(|attribute| draft.can_adjust_attribute(attribute, true))
            }
            Action::ResetAttributes => EnumSet::<Attribute>::all()
                .iter()
                .any(|attribute| draft.allocated_attribute(attribute) > 0),
            Action::Skill(skill, increase) => draft.can_adjust_skill(skill, increase),
            Action::ResetSkills => {
                EnumSet::<Skill>::all().iter().any(|skill| draft.allocated_skill(skill) > 0)
            }
            Action::Page(Page::Review) => {
                !EnumSet::<Skill>::all().iter().any(|skill| draft.can_adjust_skill(skill, true))
            }
            Action::Finish => !draft.name.trim().is_empty() && !draft.nickname.trim().is_empty(),
            Action::ChooseProfession(profession) => {
                let gender = if profession == Profession::Valkyrie && !session.choices.gender {
                    Gender::Female
                } else {
                    draft.gender()
                };
                super::CharacterDraft::budget(draft.race(), gender, profession).is_ok()
            }
            Action::ChooseRace(race) => {
                super::CharacterDraft::budget(race, draft.gender(), draft.profession()).is_ok()
            }
            Action::ChooseGender(gender) => {
                super::CharacterDraft::budget(draft.race(), gender, draft.profession()).is_ok()
            }
            _ => true,
        };
        set_enabled(&mut commands, entity, enabled, disabled);
    }
    for (menu, mut node, mut visibility) in &mut menus {
        let show = session.choices.open == Some(menu.0);
        let display = if show { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        visibility.set_if_neq(visible_if(show));
    }
}
