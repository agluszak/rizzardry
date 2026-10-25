//! The button row (retail `pc.cpp` positions and frames): the button bank,
//! the combat round button and the round's progress along the roof, the
//! previous and next character arrows, and the selected member's caption.
//! Camp, search, the formation panel and the combat orders work; the
//! journal, items, spells and options are grayed until they exist.

use bevy::prelude::*;

use super::{MainScreenArt, Panel, ScreenLayout, SelectedMember, hover_art, layer, loaded, put};
use crate::content::items::ItemCatalog;
use crate::content::ui::SpriteSheet;
use crate::game::camp::CampRequest;
use crate::game::characters::Character;
use crate::game::combat::{Battle, CombatSettings, Order, PartyOrders, StartRound};
use crate::game::data::GameData;
use crate::game::inventory::{Carried, EquipSlot};
use crate::game::party::roster::PartyRoster;
use crate::game::world::search::{Search, ToggleSearch};
use crate::presentation::screen_art::{sprite, text};

/// The bank's node, with what it last drew.
#[derive(Component, Default)]
pub struct ButtonBank {
    drawn: Option<BankView>,
}

#[derive(Clone, Debug, PartialEq)]
struct BankView {
    searching: bool,
    formation_shown: bool,
    /// In a fight: the selected member's order, and the round button.
    fight: Option<(Order, RoundButton)>,
}

/// The round button's face (retail `UpdateCombatStanceButtons`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum RoundButton {
    /// A phased fight waits for its round.
    StartRound,
    /// A phased round plays.
    GoContinuous,
    /// A continuous round plays.
    GoPhased,
    /// Continuous, between rounds.
    StartContinuous,
}

/// The selected member's caption, with the lines it last showed.
#[derive(Component, Default)]
pub struct Caption {
    drawn: Option<Vec<String>>,
}

/// The round's progress bar along the roof.
#[derive(Component)]
pub struct RoundProgress;

pub fn spawn(
    commands: &mut Commands,
    frame: Entity,
    art: &MainScreenArt,
    sheets: &Assets<SpriteSheet>,
) {
    layer(commands, frame, ButtonBank::default());
    layer(commands, frame, Caption::default());
    if let Some(mut bar) =
        put(commands, frame, sprite(loaded(sheets, &art.progress), 0, Vec2::new(177.0, 2.0)))
    {
        bar.insert((RoundProgress, Visibility::Hidden));
    }
    // Previous and next character (retail `main_scroll_arrows`).
    let arrows = loaded(sheets, &art.scroll_arrows);
    for (image, x, step) in [(0, 300.0, -1), (4, 323.0, 1)] {
        if let Some(mut arrow) = put(commands, frame, sprite(arrows, image, Vec2::new(x, 456.0))) {
            arrow
                .insert((Step(step), hover_art(arrows, image, image + 1), Pickable::default()))
                .observe(step_selection);
        }
    }
}

/// A working bank button.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Bank {
    Camp,
    Search,
    Formation,
    Order(Order),
    /// The combat round button: left click starts a round, or switches
    /// stance while one plays.
    Round,
}

pub fn draw_bank(
    mut commands: Commands,
    art: Res<MainScreenArt>,
    sheets: Res<Assets<SpriteSheet>>,
    layout: Res<ScreenLayout>,
    selected: Res<SelectedMember>,
    (battle, orders, settings, search): (
        Option<Res<Battle>>,
        Res<PartyOrders>,
        Res<CombatSettings>,
        Res<Search>,
    ),
    mut banks: Query<(Entity, &mut ButtonBank)>,
) {
    let Ok((node, mut bank)) = banks.single_mut() else { return };
    let view = BankView {
        searching: search.on,
        formation_shown: layout.shows(Panel::Formation),
        fight: battle.map(|battle| {
            let round = match (settings.continuous, battle.progress().is_some()) {
                (false, false) => RoundButton::StartRound,
                (false, true) => RoundButton::GoContinuous,
                (true, true) => RoundButton::GoPhased,
                (true, false) => RoundButton::StartContinuous,
            };
            (orders.of(selected.0), round)
        }),
    };
    if bank.drawn.as_ref() == Some(&view) {
        return;
    }
    commands.entity(node).despawn_related::<Children>();
    let sheet = |handle| loaded(&sheets, handle);
    let (icons, combat_toggle) = (sheet(&art.icons), sheet(&art.combat_toggle));
    let fighting = view.fight.is_some();
    // Each button's frames: grayed, off, hovered, on, on and hovered.
    let mut buttons: Vec<(&SpriteSheet, f32, f32, [usize; 5], bool, Option<Bank>)> = vec![
        (sheet(&art.options), 0.0, 450.0, [3, 0, 1, 2, 2], false, None),
        (icons, 40.0, 452.0, [0x2b, 0x28, 0x29, 0x2a, 0x2c], false, None),
        (icons, 70.0, 452.0, [3, 0, 1, 2, 4], false, Some(Bank::Camp)),
        (icons, 100.0, 452.0, [8, 5, 6, 7, 9], view.searching, Some(Bank::Search)),
        (icons, 130.0, 452.0, [0x17, 0x14, 0x15, 0x16, 0x18], false, None),
        (icons, 160.0, 452.0, [0x1c, 0x19, 0x1a, 0x1b, 0x1d], false, None),
        (
            icons,
            571.0,
            452.0,
            [0x26, 0x23, 0x24, 0x25, 0x27],
            view.formation_shown,
            Some(Bank::Formation),
        ),
    ];
    match view.fight {
        Some((order, round)) => {
            // End Combat (ending a fight by choice is not modelled), Defend
            // and Attack set the selected member's order.
            buttons.push((combat_toggle, 200.0, 452.0, [7, 4, 5, 6, 6], false, None));
            buttons.push((
                icons,
                232.0,
                452.0,
                [0x12, 0xf, 0x10, 0x11, 0x13],
                order == Order::Defend,
                Some(Bank::Order(Order::Defend)),
            ));
            buttons.push((
                icons,
                262.0,
                452.0,
                [0xd, 0xa, 0xb, 0xc, 0xe],
                order == Order::Attack,
                Some(Bank::Order(Order::Attack)),
            ));
            let face = match round {
                RoundButton::StartRound => &art.attack_confirm,
                RoundButton::GoContinuous => &art.cont_toggle,
                RoundButton::GoPhased => &art.combat_stop,
                RoundButton::StartContinuous => &art.cont_start,
            };
            if let Some(mut button) =
                put(&mut commands, node, sprite(sheet(face), 0, Vec2::new(610.0, 450.0)))
            {
                button.insert((Bank::Round, Pickable::default())).observe(press);
            }
        }
        None => buttons.push((combat_toggle, 199.0, 452.0, [3, 0, 1, 2, 2], false, None)),
    }
    for (sheet, x, y, [grayed, off, hover, on, on_hover], pressed, action) in buttons {
        // Camp and search are grayed in a fight.
        let usable =
            action.is_some() && !(fighting && matches!(action, Some(Bank::Camp | Bank::Search)));
        let (image, hover) = match (usable, pressed) {
            (false, _) => (grayed, grayed),
            (true, false) => (off, hover),
            (true, true) => (on, on_hover),
        };
        if let Some(mut button) = put(&mut commands, node, sprite(sheet, image, Vec2::new(x, y)))
            && usable
            && let Some(action) = action
        {
            button
                .insert((action, hover_art(sheet, image, hover), Pickable::default()))
                .observe(press);
        }
    }
    bank.drawn = Some(view);
}

fn press(
    click: On<PointerClick>,
    buttons: Query<&Bank>,
    mut camp: MessageWriter<CampRequest>,
    mut search: MessageWriter<ToggleSearch>,
    mut layout: ResMut<ScreenLayout>,
    mut orders: ResMut<PartyOrders>,
    selected: Res<SelectedMember>,
    mut round: MessageWriter<StartRound>,
    mut settings: ResMut<CombatSettings>,
    battle: Option<Res<Battle>>,
) {
    match buttons.get(click.entity) {
        Ok(Bank::Order(order)) => orders.set(selected.0, *order),
        Ok(Bank::Round) => {
            if battle.is_some_and(|b| b.executing) {
                settings.continuous = !settings.continuous;
            } else {
                round.write(StartRound);
            }
        }
        Ok(Bank::Camp) => {
            camp.write(CampRequest);
        }
        Ok(Bank::Search) => {
            search.write(ToggleSearch);
        }
        // The Party Formation button opens and closes the formation panel.
        Ok(Bank::Formation) => layout.toggle(Panel::Formation),
        Err(_) => {}
    }
}

/// Show the round's progress along the roof while one plays.
pub fn show_round_progress(
    battle: Option<Res<Battle>>,
    mut bars: Query<(&mut Node, &mut Visibility), With<RoundProgress>>,
) {
    let progress = battle.and_then(|b| b.progress());
    for (mut node, mut visibility) in &mut bars {
        match progress {
            Some(progress) => {
                node.width = Val::Px(286.0 * progress);
                node.overflow = Overflow::clip();
                visibility.set_if_neq(Visibility::Inherited);
            }
            None => {
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
    }
}

/// The selected member's caption: "Name - Profession" (retail
/// `DrawSubMenuCharacterAction`), then their order in a fight ("Attack -
/// weapon", "Unarmed" for an empty hand), else their worst condition.
pub fn draw_caption(
    mut commands: Commands,
    art: Res<MainScreenArt>,
    sheets: Res<Assets<SpriteSheet>>,
    mut images: ResMut<Assets<Image>>,
    selected: Res<SelectedMember>,
    roster: Res<PartyRoster>,
    members: Query<(&Character, Option<&Carried>)>,
    (data, catalogs, battle, orders): (
        Option<Res<GameData>>,
        Res<Assets<ItemCatalog>>,
        Option<Res<Battle>>,
        Res<PartyOrders>,
    ),
    mut captions: Query<(Entity, &mut Caption)>,
) {
    let Ok((node, mut caption)) = captions.single_mut() else { return };
    let catalog = data.and_then(|d| catalogs.get(&d.items));
    let lines = match roster.0.get(selected.0).and_then(|&m| members.get(m).ok()) {
        None => vec![],
        Some((character, carried)) => {
            let first = format!("{} - {:?}", character.name, character.profession);
            let second = if battle.is_some() {
                match orders.of(selected.0) {
                    Order::Defend => Some("Defend".to_owned()),
                    Order::Attack => {
                        let weapon = carried
                            .and_then(|c| c.worn(EquipSlot::Primary))
                            .and_then(|i| catalog?.get(i.item))
                            .map_or("Unarmed", |d| d.name.as_str());
                        Some(format!("Attack - {weapon}"))
                    }
                }
            } else {
                super::party::condition_icon(character).map(|mut c| {
                    c[..1].make_ascii_uppercase();
                    c
                })
            };
            std::iter::once(first).chain(second).collect()
        }
    };
    if caption.drawn.as_ref() == Some(&lines) {
        return;
    }
    commands.entity(node).despawn_related::<Children>();
    let font = loaded(&sheets, &art.small_font);
    for (line, y) in lines.iter().zip([454.0, 465.0]) {
        commands.spawn((text(font, &mut images, line, Vec2::new(343.0, y), 185.0), ChildOf(node)));
    }
    caption.drawn = Some(lines);
}

/// The bottom row's previous and next character buttons.
#[derive(Component, Clone, Copy)]
struct Step(i32);

fn step_selection(
    click: On<PointerClick>,
    steps: Query<&Step>,
    roster: Res<PartyRoster>,
    mut selected: ResMut<SelectedMember>,
) {
    if let (Ok(step), n @ 1..) = (steps.get(click.entity), roster.0.len() as i32) {
        selected.0 = (selected.0 as i32 + step.0).rem_euclid(n) as usize;
    }
}

/// Combat keys from retail's `mgskeyboard.ini`: Enter starts the round,
/// backslash switches continuous combat, A and D set the selected member to
/// attack or defend.
pub fn combat_keys(
    keys: Res<ButtonInput<KeyCode>>,
    battle: Option<Res<Battle>>,
    selected: Res<SelectedMember>,
    mut orders: ResMut<PartyOrders>,
    mut settings: ResMut<CombatSettings>,
    mut round: MessageWriter<StartRound>,
) {
    if keys.just_pressed(KeyCode::Backslash) {
        settings.continuous = !settings.continuous;
    }
    if battle.is_none() {
        return;
    }
    if keys.just_pressed(KeyCode::Enter) {
        round.write(StartRound);
    }
    if keys.just_pressed(KeyCode::KeyA) {
        orders.set(selected.0, Order::Attack);
    }
    if keys.just_pressed(KeyCode::KeyD) {
        orders.set(selected.0, Order::Defend);
    }
}
