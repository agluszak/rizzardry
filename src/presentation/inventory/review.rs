//! Retail character statistics and skills review pages.
use super::super::screen_art::{BitmapText, Picture, PictureCrop};
use super::{InventoryArt, InventoryScreen, absolute};
use crate::content::characters::{Attribute, Skill};
use crate::content::items::{EquipClass, ItemCatalog};
use crate::content::ui::{SpriteSheet, TextTable};
use crate::game::characters::Character;
use crate::game::combat::attack::{character_initiative, primary_hand};
use crate::game::data::GameData;
use crate::game::inventory::{Carried, EquipSlot};
use crate::game::party::roster::PartyRoster;
use crate::presentation::character_view::{CharacterView, CharacterViewItem};
use crate::presentation::retail_controls::ControlSkin;
use crate::presentation::screen_art::spawn_picture as picture;
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::{InteractionDisabled, Selected};
use bevy::ui_widgets::{
    Activate, Button as UiButton, SelectedTab, Tab, TabList, tablist_self_update,
};
use bevy::ui_widgets::{SetSliderValue, SliderRange, SliderValue, SliderValueChange};
use enumset::EnumSet;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Items,
    Statistics,
    Skills,
    Spells,
}
#[derive(Component)]
pub(super) struct PageButton(pub(super) Page);
#[derive(Component)]
pub(super) struct PageTabs;

#[derive(Component)]
pub(super) enum Field {
    String(usize),
    InfoString(Option<usize>, Option<usize>),
    Attribute(Attribute),
    Skill(Skill),
    SkillName(Skill),
    Bonus,
    Effect(usize),
    HandName(bool),
    CombatValue(usize),
}
#[derive(Component, Default)]
pub(super) struct StatsInfo {
    armour: bool,
    filter: EffectFilter,
    items: bool,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum EffectFilter {
    #[default]
    All,
    Beneficial,
    Detrimental,
}
#[derive(Component)]
pub(super) struct EffectButton(usize);
#[derive(Component)]
pub(super) struct StatsScroll;
#[derive(Component)]
pub(super) struct InfoToggle;
#[derive(Component)]
pub(super) struct InfoBackground;
#[derive(Component)]
pub(super) struct SkillRow(Skill);
#[derive(Component)]
pub(super) enum Bar {
    Attribute(Attribute),
    SkillBase(Skill),
    SkillBonus(Skill),
}

fn label(
    commands: &mut Commands,
    parent: Entity,
    font: Handle<SpriteSheet>,
    at: Vec2,
    width: f32,
    field: Field,
) {
    let label = crate::presentation::screen_art::spawn_label(
        commands,
        parent,
        BitmapText {
            font,
            at,
            width,
            justify: if width > 0.0 {
                bevy::text::Justify::Center
            } else {
                bevy::text::Justify::Left
            },
            ..default()
        },
    );
    commands.entity(label).insert(field);
}
fn bar(
    commands: &mut Commands,
    parent: Entity,
    sheet: Handle<SpriteSheet>,
    at: Vec2,
    kind: Bar,
    frame: usize,
) {
    commands.spawn((
        kind,
        Picture { sheet, frame, at, bounds: None },
        PictureCrop::horizontal(0.0, 0.0),
        ChildOf(parent),
    ));
}
fn panel(commands: &mut Commands, root: Entity, page: Page) -> Entity {
    commands
        .spawn((
            page,
            Node {
                display: Display::None,
                ..absolute(Vec2::new(0.0, 165.0), Vec2::new(640.0, 285.0))
            },
            ChildOf(root),
        ))
        .id()
}

pub(super) fn build(commands: &mut Commands, root: Entity, art: &InventoryArt) -> Entity {
    // These actions require their gameplay operations; show retail's disabled
    // art until those operations are available.
    for (i, base) in [0, 10, 5, 15, 20, 30, 35, 25].into_iter().enumerate() {
        picture(
            commands,
            root,
            art.item_buttons.clone(),
            base + 3,
            Vec2::new(i as f32 * 44.0, 450.0),
        );
    }
    let list = commands
        .spawn((
            PageTabs,
            TabList::default(),
            absolute(Vec2::new(420.0, 450.0), Vec2::new(176.0, 30.0)),
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .observe(tablist_self_update)
        .id();
    let mut initial = Entity::PLACEHOLDER;
    for (i, (page, base)) in [
        (Some(Page::Items), 0),
        (Some(Page::Spells), 10),
        (Some(Page::Skills), 15),
        (Some(Page::Statistics), 5),
    ]
    .into_iter()
    .enumerate()
    {
        let at = Vec2::new(i as f32 * 44.0, 0.0);
        let mut button = commands.spawn((
            Tab,
            ControlSkin::new(base, base + 1, base + 4, base + 2, base + 3),
            Hovered::default(),
            Picture { sheet: art.page_buttons.clone(), frame: base, bounds: Some(base), at },
            absolute(at, Vec2::new(44.0, 30.0)),
            Pickable::default(),
            crate::presentation::retail_controls::FocusRing,
            ChildOf(list),
        ));
        if let Some(page) = page {
            button.insert(PageButton(page));
            if page == Page::Items {
                initial = button.id();
            }
        } else {
            button.insert((
                InteractionDisabled,
                Picture {
                    sheet: art.page_buttons.clone(),
                    frame: base + 3,
                    bounds: Some(base),
                    at,
                },
            ));
        }
    }
    commands.entity(list).insert(SelectedTab(Some(initial)));
    let stats = panel(commands, root, Page::Statistics);
    commands.entity(stats).insert(StatsInfo::default());
    picture(commands, stats, art.stats_page.clone(), 1, Vec2::ZERO);
    let background =
        picture(commands, stats, art.combat_panel.clone(), 0, Vec2::new(310.0, -165.0));
    commands.entity(background).insert(InfoBackground);
    let at = Vec2::new(318.0, -26.0);
    commands
        .spawn((
            UiButton,
            TabIndex(0),
            Hovered::default(),
            InfoToggle,
            ControlSkin::new(15, 17, 16, 19, 15),
            Picture { sheet: art.combat_buttons.clone(), frame: 15, bounds: Some(15), at },
            absolute(at, Vec2::splat(20.0)),
            Pickable::default(),
            crate::presentation::retail_controls::FocusRing,
            ChildOf(stats),
        ))
        .observe(crate::presentation::retail_controls::primary_press)
        .observe(toggle_info);
    label(
        commands,
        stats,
        art.font.clone(),
        Vec2::new(347.0, -155.0),
        285.0,
        Field::InfoString(Some(0x8b0), Some(0x8be)),
    );
    label(
        commands,
        stats,
        art.font.clone(),
        Vec2::new(350.0, -117.0),
        0.0,
        Field::InfoString(Some(0x8b1), None),
    );
    label(commands, stats, art.font.clone(), Vec2::new(430.0, -117.0), 32.0, Field::CombatValue(8));
    for (x, left) in [(470.0, true), (552.0, false)] {
        label(commands, stats, art.font.clone(), Vec2::new(x, -131.0), 80.0, Field::HandName(left));
    }
    for (i, id) in [0x8b6, 0x8bd, 0x8bc, 0x8ba, 0x8bb, 0x8b7, 0x8b8, 0x8b9].into_iter().enumerate()
    {
        let y = -117.0 + i as f32 * 14.0;
        label(
            commands,
            stats,
            art.font.clone(),
            Vec2::new(504.0, y),
            94.0,
            Field::InfoString(Some(id), None),
        );
        label(commands, stats, art.font.clone(), Vec2::new(470.0, y), 32.0, Field::CombatValue(i));
    }
    for (i, id) in (1052..1064).enumerate() {
        let x = if i < 6 { 350.0 } else { 495.0 };
        let y = -131.0 + (i % 6) as f32 * 14.0;
        label(
            commands,
            stats,
            art.font.clone(),
            Vec2::new(x, y),
            0.0,
            Field::InfoString(None, Some(id)),
        );
    }
    label(
        commands,
        stats,
        art.font.clone(),
        Vec2::new(474.0, -33.0),
        0.0,
        Field::InfoString(None, Some(0x8bf)),
    );
    label(commands, stats, art.font.clone(), Vec2::new(350.0, -33.0), 0.0, Field::String(0x935));
    label(commands, stats, art.font.clone(), Vec2::new(350.0, -19.0), 0.0, Field::String(0x936));

    label(commands, stats, art.font.clone(), Vec2::new(10.0, 10.0), 287.0, Field::String(0x937));
    label(commands, stats, art.font.clone(), Vec2::new(10.0, 141.0), 287.0, Field::String(0x938));
    label(commands, stats, art.font.clone(), Vec2::new(315.0, 10.0), 315.0, Field::String(0x939));
    label(commands, stats, art.font.clone(), Vec2::new(16.0, 157.0), 0.0, Field::Bonus);
    const ATTRIBUTE_NAMES: [usize; 7] = [0x924, 0x925, 0x926, 0x927, 0x928, 0x92a, 0x929];
    for (i, attribute) in EnumSet::<Attribute>::all().iter().enumerate() {
        let y = 25.0 + i as f32 * 14.0;
        label(
            commands,
            stats,
            art.font.clone(),
            Vec2::new(10.0, y),
            123.0,
            Field::String(ATTRIBUTE_NAMES[i]),
        );
        label(
            commands,
            stats,
            art.font.clone(),
            Vec2::new(264.0, y),
            33.0,
            Field::Attribute(attribute),
        );
        bar(
            commands,
            stats,
            art.slider.clone(),
            Vec2::new(136.0, y + 1.0),
            Bar::Attribute(attribute),
            0,
        );
    }
    for (i, y) in [25.0, 49.0, 78.0].into_iter().enumerate() {
        let at = Vec2::new(316.0, y);
        commands
            .spawn((
                UiButton,
                TabIndex(0),
                Hovered::default(),
                EffectButton(i),
                ControlSkin::new(
                    i * 5,
                    i * 5 + 2,
                    if i == 2 { 15 } else { i * 5 + 1 },
                    if i == 2 { 17 } else { i * 5 + 4 },
                    i * 5 + 3,
                ),
                Picture {
                    sheet: art.effect_filters.clone(),
                    frame: i * 5,
                    bounds: Some(i * 5),
                    at,
                },
                absolute(at, Vec2::splat(24.0)),
                Pickable::default(),
                crate::presentation::retail_controls::FocusRing,
                ChildOf(stats),
            ))
            .observe(crate::presentation::retail_controls::primary_press)
            .observe(select_filter);
    }
    let track = crate::presentation::retail_controls::step_scrollbar(
        commands,
        stats,
        art.scroll.clone(),
        Vec2::new(612.0, 25.0),
        215.0,
        0.0,
    );
    commands.entity(track).insert(StatsScroll);
    commands
        .spawn((
            crate::presentation::retail_controls::ScrollWheel(track),
            absolute(Vec2::new(349.0, 25.0), Vec2::new(259.0, 247.0)),
            Pickable::default(),
            ChildOf(stats),
        ))
        .observe(crate::presentation::retail_controls::scroll_wheel);
    for i in 0..17 {
        label(
            commands,
            stats,
            art.font.clone(),
            Vec2::new(350.0, 26.0 + i as f32 * 14.0),
            0.0,
            Field::Effect(i),
        );
    }

    let skills = panel(commands, root, Page::Skills);
    picture(commands, skills, art.skills_page.clone(), 2, Vec2::ZERO);
    let expert = picture(commands, skills, art.skills_page.clone(), 0, Vec2::new(310.0, -165.0));
    commands.entity(expert).insert(ExpertBackdrop);
    for (i, (x, y, frame)) in
        [(352.0, 8.0, 2), (352.0, 204.0, 3), (34.0, 9.0, 0), (34.0, 134.0, 1), (352.0, -132.0, 4)]
            .into_iter()
            .enumerate()
    {
        let entity = commands
            .spawn((
                Picture {
                    sheet: art.skills_icons.clone(),
                    frame,
                    at: Vec2::new(x - 22.0, y - 3.0),
                    bounds: None,
                },
                CategoryIcon(i),
                ChildOf(skills),
            ))
            .id();
        let _ = entity;
    }
    for skill in EnumSet::<Skill>::all() {
        let row = commands
            .spawn((SkillRow(skill), absolute(Vec2::ZERO, Vec2::new(273.0, 14.0)), ChildOf(skills)))
            .id();
        picture(commands, row, art.skills_page.clone(), 3, Vec2::ZERO);
        commands
            .spawn((
                super::info::InfoTarget { subject: super::info::Subject::Skill(skill), root },
                absolute(Vec2::ZERO, Vec2::new(109.0, 14.0)),
                Pickable::default(),
                ChildOf(row),
            ))
            .observe(super::info::open_info);
        label(commands, row, art.font.clone(), Vec2::new(2.0, 1.0), 107.0, Field::SkillName(skill));
        label(commands, row, art.font.clone(), Vec2::new(238.0, 1.0), 36.0, Field::Skill(skill));
        bar(commands, row, art.slider.clone(), Vec2::new(111.0, 2.0), Bar::SkillBase(skill), 0);
        bar(commands, row, art.slider.clone(), Vec2::new(111.0, 2.0), Bar::SkillBonus(skill), 1);
    }
    initial
}
#[derive(Component)]
pub(super) struct CategoryIcon(usize);
#[derive(Component)]
pub(super) struct ExpertBackdrop;

fn select_filter(
    click: On<Activate>,
    buttons: Query<&EffectButton>,
    mut info: Single<&mut StatsInfo>,
    slider: Single<Entity, With<StatsScroll>>,
    mut commands: Commands,
) {
    if let Ok(button) = buttons.get(click.entity) {
        match button.0 {
            0 => {
                info.filter = if info.filter == EffectFilter::Beneficial {
                    EffectFilter::All
                } else {
                    EffectFilter::Beneficial
                }
            }
            1 => {
                info.filter = if info.filter == EffectFilter::Detrimental {
                    EffectFilter::All
                } else {
                    EffectFilter::Detrimental
                }
            }
            _ => info.items = !info.items,
        }
        commands
            .trigger(SetSliderValue { entity: *slider, change: SliderValueChange::Absolute(0.0) });
    }
}
fn toggle_info(_: On<Activate>, mut info: Single<&mut StatsInfo>) {
    info.armour = !info.armour;
}

fn skill_category(skill: Skill) -> usize {
    use Skill::*;
    match skill {
        Sword | Axe | Polearm | MaceAndFlail | Dagger | StaffAndWand | Shield | ModernWeapon
        | Bow | ThrowingAndSling | MartialArts => 0,
        LocksAndTraps | Stealth | Music | Pickpocket | Scouting => 1,
        CloseCombat | RangedCombat | DualWeapons | CriticalStrike | Artifacts | Mythology
        | Communication | Engineering => 2,
        Wizardry | Divinity | Alchemy | Psionics | FireMagic | WaterMagic | AirMagic
        | EarthMagic | MentalMagic | DivineMagic => 3,
        PowerStrike | PowerCast | IronWill | IronSkin | Reflextion | SnakeSpeed | EagleEye => 4,
    }
}
fn words(value: impl std::fmt::Debug) -> String {
    let value = format!("{value:?}");
    let mut result = String::new();
    for (i, c) in value.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            result.push(' ');
        }
        result.push(c);
    }
    result
}
pub(super) fn show_pages(
    selected: Option<Single<&SelectedTab, With<PageTabs>>>,
    buttons: Query<&PageButton>,
    mut pages: Query<(&Page, &mut Node, &mut Visibility)>,
) {
    let Some(selected) = selected else { return };
    let current = selected.0.and_then(|entity| buttons.get(entity).ok()).map(|button| button.0);
    for (page, mut node, mut visibility) in &mut pages {
        let shown = Some(*page) == current;
        let display = if shown { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        visibility.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
}

pub(super) fn refresh(
    screen: Option<Single<Ref<InventoryScreen>>>,
    characters: Query<CharacterView>,
    mut removed_carried: RemovedComponents<Carried>,
    roster: Option<Res<PartyRoster>>,
    profiles: Query<Ref<Character>>,
    info: Option<Single<Ref<StatsInfo>>>,
    slider: Option<Single<(Entity, Ref<SliderValue>, &SliderRange), With<StatsScroll>>>,
    mut commands: Commands,
    data: Option<Res<GameData>>,
    catalogs: Option<Res<Assets<ItemCatalog>>>,
    art: Option<Res<InventoryArt>>,
    tables: Option<Res<Assets<TextTable>>>,
    mut fields: Query<(Ref<Field>, &mut BitmapText)>,
    mut info_background: Query<&mut Picture, With<InfoBackground>>,
) {
    let removed: Vec<_> = removed_carried.read().collect();
    let Some(screen) = screen else { return };
    let Ok(view) = characters.get(screen.member) else { return };
    let actor_changed = view.is_changed() || removed.contains(&screen.member);
    let CharacterViewItem { profile, vitals, carried } = view;
    let armour_view = info.as_ref().is_some_and(|info| info.armour);
    let effect_filter = info.as_ref().map_or(EffectFilter::All, |info| info.filter);
    let show_items = info.as_ref().is_some_and(|info| info.items);
    let info_changed = info.as_ref().is_some_and(|info| info.is_changed());
    let catalog = data
        .as_ref()
        .and_then(|data| catalogs.as_ref().and_then(|catalogs| catalogs.get(&data.items)));
    let hand = carried
        .as_ref()
        .zip(catalog)
        .map(|(carried, catalog)| primary_hand(&profile, carried, catalog));
    let unidentified = carried
        .as_ref()
        .and_then(|c| c.worn(EquipSlot::Primary))
        .is_some_and(|item| !item.identified);
    let table =
        art.as_ref().and_then(|art| tables.as_ref().and_then(|tables| tables.get(&art.strings)));
    let table_changed = tables.as_ref().is_some_and(|tables| tables.is_changed());
    let party_changed = roster.as_ref().is_some_and(|roster| {
        roster.is_changed()
            || profiles.iter_many(&roster.0).matched().any(|profile| profile.is_changed())
    });
    if !screen.is_changed()
        && !actor_changed
        && !info_changed
        && !slider.as_ref().is_some_and(|slider| slider.1.is_changed())
        && !table_changed
        && !party_changed
        && !catalogs.as_ref().is_some_and(|catalogs| catalogs.is_changed())
        && !fields.iter().any(|(field, _)| field.is_added())
    {
        return;
    }
    let string = |id| table.and_then(|table| table.get(id)).unwrap_or("");
    let mut effects = Vec::new();
    if show_items {
        if let (Some(carried), Some(catalog)) = (carried.as_ref(), catalog) {
            for slot in EnumSet::<EquipSlot>::all() {
                if matches!(slot, EquipSlot::AlternatePrimary | EquipSlot::AlternateSecondary) {
                    continue;
                }
                let Some(item) = carried.worn(slot).filter(|item| item.identified) else {
                    continue;
                };
                let Some(definition) = catalog.get(item.item) else { continue };
                let armour = if definition.armor_class_bonus < 0
                    || (!matches!(
                        slot,
                        EquipSlot::Head
                            | EquipSlot::Torso
                            | EquipSlot::Hands
                            | EquipSlot::Legs
                            | EquipSlot::Feet
                    ) && definition.class != EquipClass::Shield)
                {
                    definition.armor_class_bonus
                } else {
                    0
                };
                let cursed = definition.cursed && !item.curse_released;
                let beneficial = definition.hit_bonus > 0
                    || armour > 0
                    || definition.resistance_bonus.values().any(|&value| value > 0);
                let detrimental = definition.hit_bonus < 0
                    || armour < 0
                    || cursed
                    || definition.resistance_bonus.values().any(|&value| value < 0);
                if !(beneficial || detrimental)
                    || (effect_filter == EffectFilter::Beneficial && !beneficial)
                    || (effect_filter == EffectFilter::Detrimental && !detrimental)
                {
                    continue;
                }
                effects.push((item.display_name(definition, false), true));
                if definition.hit_bonus != 0 {
                    effects.push((format!("{} {:+}", string(0x8b7), definition.hit_bonus), false));
                }
                if armour != 0 {
                    effects.push((format!("{} {:+}", string(0x8df), armour), false));
                }
                for (realm, &bonus) in definition.resistance_bonus.iter() {
                    if bonus != 0 {
                        effects.push((
                            format!(
                                "{} {} {:+}%",
                                string(0x30b + realm as usize),
                                string(0x8e0),
                                bonus
                            ),
                            false,
                        ));
                    }
                }
                if cursed {
                    effects.push((string(0x8e1).to_owned(), false));
                }
                effects.push((String::new(), false));
            }
        }
    } else if effect_filter != EffectFilter::Beneficial {
        if vitals.is_dead() {
            effects.push((format!("{} {}", string(0x8d1), string(0x390)).trim().to_owned(), true));
            effects.push((string(0x8d2).to_owned(), false));
            effects.push((String::new(), false));
        } else {
            let mut conditions: Vec<_> = vitals.conditions.iter().collect();
            conditions.sort_unstable_by_key(|(condition, _)| std::cmp::Reverse(*condition));
            for &(condition, lasting) in &conditions {
                let ordinal = *condition as usize + 1;
                let name_id = match ordinal {
                    16 => 0x38c,
                    17 => 0x388,
                    _ => 0x348 + ordinal * 4,
                };
                let name = string(name_id);
                let name = if name.is_empty() { words(condition) } else { name.to_owned() };
                effects.push((format!("{} {}", string(0x8d1), name).trim().to_owned(), true));
                if matches!(lasting, crate::game::characters::Lasting::Indefinite) {
                    effects.push((string(0x8d2).to_owned(), false));
                }
                effects.push((String::new(), false));
            }
        }
    }
    let maximum = effects.len().saturating_sub(17);
    let offset = slider.as_deref().map_or(0, |(_, value, _)| (value.0 as usize).min(maximum));
    if let Some((entity, value, range)) = slider.as_deref() {
        crate::presentation::retail_controls::set_scroll_maximum(
            &mut commands,
            *entity,
            value,
            range,
            maximum as f32,
        );
    }
    for (field, mut text) in &mut fields {
        if !field.is_added()
            && !screen.is_changed()
            && !profile.is_changed()
            && !vitals.is_changed()
            && !table_changed
            && !party_changed
            && !info_changed
            && !slider.as_ref().is_some_and(|slider| slider.1.is_changed())
            && !catalogs.as_ref().is_some_and(|catalogs| catalogs.is_changed())
        {
            continue;
        }
        let palette = match *field {
            Field::Effect(index)
                if effects.get(index + offset).is_some_and(|(_, heading)| *heading) =>
            {
                art.as_ref().map(|art| art.green.clone())
            }
            Field::SkillName(skill) => {
                let active = profile.skill_available(skill);
                let level = profile.skill(skill);
                let best = level > 0
                    && roster.as_ref().is_some_and(|roster| {
                        profiles
                            .iter_many(&roster.0)
                            .matched()
                            .all(|other| other.skill(skill) <= level)
                    });
                art.as_ref().and_then(|art| match (best, active) {
                    (true, true) => Some(art.yellow.clone()),
                    (true, false) => Some(art.beige.clone()),
                    (false, false) => Some(art.gray.clone()),
                    (false, true) => None,
                })
            }
            _ => None,
        };
        if text.palette != palette {
            text.palette = palette;
        }
        let value = match *field {
            Field::String(id) => string(id).to_owned(),
            Field::InfoString(combat, armour) => {
                (if armour_view { armour } else { combat }).map_or("", string).to_owned()
            }
            Field::Attribute(attribute) => profile.attribute(attribute).to_string(),
            Field::Skill(skill) => profile.skill(skill).to_string(),
            Field::SkillName(skill) => string(0x2e2 + skill as usize).to_owned(),
            Field::Bonus => {
                format!("{} {}", string(0x2e2 + profile.bonus_skill() as usize), string(0x8c5))
            }
            Field::Effect(index) => {
                effects.get(index + offset).map(|(value, _)| value.clone()).unwrap_or_default()
            }
            Field::HandName(_) if armour_view => String::new(),
            Field::HandName(left) => {
                let empty = carried.as_ref().is_none_or(|c| {
                    c.worn(EquipSlot::Primary).is_none() && c.worn(EquipSlot::Secondary).is_none()
                });
                string(match (empty, left) {
                    (true, true) => 0x8b5,
                    (true, false) => 0x8b4,
                    (false, true) => 0x8b2,
                    (false, false) => 0x8b3,
                })
                .to_owned()
            }
            Field::CombatValue(_) if armour_view => String::new(),
            Field::CombatValue(index) => {
                if let Some(hand) = &hand {
                    if unidentified && index != 8 {
                        "?".to_owned()
                    } else {
                        match index {
                            1 => {
                                let min = i32::from(hand.dice.base) + i32::from(hand.dice.count);
                                let max = i32::from(hand.dice.base)
                                    + i32::from(hand.dice.count) * i32::from(hand.dice.sides);
                                let scale = 100 + hand.damage_percent;
                                format!(
                                    "{}-{}",
                                    ((min * scale + 50) / 100).max(1),
                                    ((max * scale + 50) / 100).max(1)
                                )
                            }
                            3 => hand.attacks.to_string(),
                            4 => hand.swings.to_string(),
                            5 => format!("{:+}", hand.to_hit),
                            7 => format!("{:+}%", hand.damage_percent),
                            8 => character_initiative(&profile, hand).to_string(),
                            _ => String::new(),
                        }
                    }
                } else {
                    String::new()
                }
            }
        };
        if text.value != value {
            text.value = value;
        }
    }
    for mut picture in &mut info_background {
        let frame = usize::from(armour_view);
        if picture.frame != frame {
            picture.frame = frame;
        }
    }
}

pub(super) fn layout_skills(
    screen: Option<Single<Ref<InventoryScreen>>>,
    characters: Query<Ref<Character>>,
    mut rows: Query<
        (Entity, Ref<SkillRow>, &mut Node, &mut Visibility),
        (Without<Bar>, Without<CategoryIcon>),
    >,
    mut bars: Query<(&Bar, &mut PictureCrop), Without<SkillRow>>,
    mut backdrop: Query<&mut Picture, With<ExpertBackdrop>>,
    mut icons: Query<(&CategoryIcon, &mut Visibility), Without<SkillRow>>,
) {
    let Some(screen) = screen else { return };
    let Ok(profile) = characters.get(screen.member) else { return };
    if !screen.is_changed()
        && !profile.is_changed()
        && !rows.iter().any(|(_, row, _, _)| row.is_added())
    {
        return;
    }
    let mut ordered: Vec<_> =
        rows.iter().map(|(entity, row, _, _)| (row.0 as usize, entity)).collect();
    ordered.sort_unstable_by_key(|(skill, _)| *skill);
    let origins = [(352.0, 8.0), (352.0, 204.0), (34.0, 9.0), (34.0, 134.0), (352.0, -132.0)];
    let mut counts = [0usize; 5];
    for (_, entity) in ordered {
        let Ok((_, row, mut node, mut visibility)) = rows.get_mut(entity) else { continue };
        let category = skill_category(row.0);
        let shown = profile.skill_available(row.0)
            || profile.skill_points[row.0] > 0
            || profile.skill(row.0) > 0;
        let display = if shown { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        visibility.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
        if shown {
            let (x, y) = origins[category];
            let top = px(y + counts[category] as f32 * 14.0);
            if node.left != px(x) {
                node.left = px(x);
            }
            if node.top != top {
                node.top = top;
            }
            counts[category] += 1;
        }
    }
    for mut picture in &mut backdrop {
        {
            let frame = usize::from(counts[4] > 0);
            if picture.frame != frame {
                picture.frame = frame;
            }
        }
    }
    for (icon, mut visibility) in &mut icons {
        visibility.set_if_neq(if icon.0 != 4 || counts[4] > 0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    for (bar, mut crop) in &mut bars {
        let (start, end) = match *bar {
            Bar::Attribute(attribute) => (0.0, profile.attribute(attribute).min(125) as f32),
            Bar::SkillBase(skill) => (
                0.0,
                u32::from(profile.skill_points[skill]).min(profile.skill(skill)).min(125) as f32,
            ),
            Bar::SkillBonus(skill) => (
                u32::from(profile.skill_points[skill]).min(profile.skill(skill)).min(125) as f32,
                profile.skill(skill).min(125) as f32,
            ),
        };
        crop.set_if_neq(PictureCrop::horizontal(start, end));
    }
}

pub(super) fn style_info_button(
    info: Option<Single<&StatsInfo>>,
    mut buttons: Query<(&mut ControlSkin, &mut Picture), With<InfoToggle>>,
) {
    let Some(info) = info else { return };
    let base = if info.armour { 10 } else { 15 };
    for (mut skin, mut picture) in &mut buttons {
        skin.set_if_neq(ControlSkin::new(base, base + 2, base + 1, base + 4, base));
        if picture.bounds != Some(base) {
            picture.bounds = Some(base);
        }
    }
}

pub(super) fn style_effect_buttons(
    info: Option<Single<&StatsInfo>>,
    mut commands: Commands,
    buttons: Query<(Entity, &EffectButton, Has<Selected>)>,
) {
    let Some(info) = info else { return };
    for (entity, button, selected) in &buttons {
        let active = match button.0 {
            0 => info.filter == EffectFilter::Beneficial,
            1 => info.filter == EffectFilter::Detrimental,
            _ => info.items,
        };
        if active != selected {
            if active {
                commands.entity(entity).insert(Selected);
            } else {
                commands.entity(entity).remove::<Selected>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::characters::Vitals;
    use crate::game::characters::{Condition, Gender, Lasting, Profession, Race};

    #[test]
    fn review_tracks_character_and_condition_changes() {
        let mut app = App::new();
        // As InventoryPlugin registers them: unordered.
        app.add_plugins(MinimalPlugins).insert_resource(InventoryArt::default()).add_systems(
            Update,
            (show_pages, refresh, layout_skills, style_info_button, style_effect_buttons),
        );
        let profile = Character::new("A", Race::Human, Gender::Male, Profession::Fighter);
        let actor = app.world_mut().spawn((Vitals::new(&profile), profile)).id();
        let root = app.world_mut().spawn(InventoryScreen { member: actor }).id();
        app.world_mut().commands().queue(move |world: &mut World| {
            let mut commands = world.commands();
            build(&mut commands, root, &InventoryArt::default());
        });
        app.update();
        let text = |world: &mut World, value: &str| {
            world.query::<&BitmapText>().iter(world).any(|text| text.value == value)
        };
        assert!(text(app.world_mut(), "55"));
        app.world_mut().get_mut::<Character>(actor).unwrap().attributes[Attribute::Strength] = 70;
        app.world_mut()
            .get_mut::<Vitals>(actor)
            .unwrap()
            .inflict(Condition::Blind, Lasting::Rounds(1));
        app.update();
        assert!(text(app.world_mut(), "70"));
        assert!(text(app.world_mut(), "Blind"));
        let beneficial = app
            .world_mut()
            .query::<(Entity, &EffectButton)>()
            .iter(app.world())
            .find_map(|(entity, button)| (button.0 == 0).then_some(entity))
            .unwrap();
        app.world_mut().trigger(Activate { entity: beneficial });
        app.update();
        assert!(!text(app.world_mut(), "Blind"));
        assert!(app.world().get::<Vitals>(actor).unwrap().has(Condition::Blind));
        app.world_mut().trigger(Activate { entity: beneficial });
        app.update();
        assert!(text(app.world_mut(), "Blind"));
        app.world_mut().get_mut::<Vitals>(actor).unwrap().age_conditions(1);
        app.update();
        assert!(!text(app.world_mut(), "Blind"));
    }
}
