//! The button row (retail `pc.cpp` positions and frames): the button bank,
//! the combat round button and the round's progress along the roof, the
//! previous and next character arrows, and the selected member's caption.
//! Camp, search, the formation panel and the combat orders work; the
//! journal, items, spells and options are grayed until they exist.

use bevy::prelude::*;

use super::{MainScreenArt, Panel, ScreenLayout, SelectedMember, button, layer, loaded};
use crate::content::items::ItemCatalog;
use crate::content::ui::SpriteSheet;
use crate::game::camp::CampRequest;
use crate::game::characters::Character;
use crate::game::combat::{Battle, CombatSettings, StartRound};
use crate::game::data::GameData;
use crate::game::inventory::{Carried, EquipSlot};
use crate::game::party::Order;
use crate::game::party::roster::PartyRoster;
use crate::game::world::search::{Search, ToggleSearch};
use crate::presentation::screen_art::{hover_art, sprite, text};

/// The bank's node, with what it last drew.
#[derive(Component, Clone, Default)]
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
#[derive(Component, Clone, Default)]
pub struct Caption {
    drawn: Option<Vec<String>>,
}

/// The round's progress bar along the roof.
#[derive(Component, Clone, Default)]
pub struct RoundProgress;

/// The button row's own nodes: the bank, the caption, the round's progress
/// bar, and the previous and next character arrows (retail
/// `main_scroll_arrows`).
pub fn row(art: &MainScreenArt, sheets: &Assets<SpriteSheet>) -> impl SceneList {
    let progress = sprite(loaded(sheets, &art.progress), 0, Vec2::new(177.0, 2.0));
    let arrows = loaded(sheets, &art.scroll_arrows);
    let steps: Vec<_> = [(0, 300.0, -1_i32), (4, 323.0, 1)]
        .into_iter()
        .map(|(image, x, step)| {
            bsn! {
                @button(arrows, image, Vec2::new(x, 456.0))
                @hover_art(arrows, image, image + 1)
                on(move |_: On<PointerClick>,
                         roster: Res<PartyRoster>,
                         mut selected: ResMut<SelectedMember>| {
                    let members = roster.0.len() as i32;
                    if members > 0 {
                        let current = selected.member(&roster).and_then(|m| roster.0.get_index_of(&m)).unwrap_or(0);
                        selected.0 = roster.0.get_index((current as i32 + step).rem_euclid(members) as usize).copied();
                    }
                })
            }
        })
        .collect();
    bsn_list! {
        @layer() ButtonBank
        -- @layer() Caption
        -- @{progress} RoundProgress Visibility::Hidden
        -- {steps}
    }
}

/// A working bank button.
#[derive(Component, Clone, Copy, Default, PartialEq, Eq)]
enum Bank {
    #[default]
    Camp,
    Search,
    Formation,
    Order(Order),
    /// The combat round button: left click starts a round, or switches
    /// stance while one plays.
    Round,
}

/// A bank button: where it sits, its frames in its sheet, whether it shows
/// pressed, and what it does, if anything yet.
struct BankButton<'a> {
    sheet: &'a SpriteSheet,
    at: Vec2,
    frames: Frames,
    pressed: bool,
    action: Option<Bank>,
}

/// A button's frames in its sheet.
#[derive(Clone, Copy)]
struct Frames {
    grayed: usize,
    off: usize,
    hovered: usize,
    on: usize,
    on_hovered: usize,
}

pub fn draw_bank(
    mut commands: Commands,
    art: Res<MainScreenArt>,
    sheets: Res<Assets<SpriteSheet>>,
    layout: Res<ScreenLayout>,
    selected: Res<SelectedMember>,
    roster: Res<PartyRoster>,
    (battle, orders, settings, search): (
        Option<Res<Battle>>,
        Query<&Order>,
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
            (
                selected
                    .member(&roster)
                    .and_then(|m| orders.get(m).ok())
                    .copied()
                    .unwrap_or_default(),
                round,
            )
        }),
    };
    if bank.drawn.as_ref() == Some(&view) {
        return;
    }
    let sheet = |handle| loaded(&sheets, handle);
    let (icons, combat_toggle) = (sheet(&art.icons), sheet(&art.combat_toggle));
    let frames = |[grayed, off, hovered, on, on_hovered]: [usize; 5]| Frames {
        grayed,
        off,
        hovered,
        on,
        on_hovered,
    };
    let icon = |x: f32, pressed, action, at: [usize; 5]| BankButton {
        sheet: icons,
        at: Vec2::new(x, 452.0),
        frames: frames(at),
        pressed,
        action,
    };
    let mut buttons = vec![
        BankButton {
            sheet: sheet(&art.options),
            at: Vec2::new(0.0, 450.0),
            frames: frames([3, 0, 1, 2, 2]),
            pressed: false,
            action: None,
        },
        icon(40.0, false, None, [0x2b, 0x28, 0x29, 0x2a, 0x2c]),
        icon(70.0, false, Some(Bank::Camp), [3, 0, 1, 2, 4]),
        icon(100.0, view.searching, Some(Bank::Search), [8, 5, 6, 7, 9]),
        icon(130.0, false, None, [0x17, 0x14, 0x15, 0x16, 0x18]),
        icon(160.0, false, None, [0x1c, 0x19, 0x1a, 0x1b, 0x1d]),
        icon(571.0, view.formation_shown, Some(Bank::Formation), [0x26, 0x23, 0x24, 0x25, 0x27]),
    ];
    let combat = |x: f32, at| BankButton {
        sheet: combat_toggle,
        at: Vec2::new(x, 452.0),
        frames: frames(at),
        pressed: false,
        action: None,
    };
    let mut round = None;
    match view.fight {
        Some((order, face)) => {
            // End Combat (ending a fight by choice is not modelled), Defend
            // and Attack set the selected member's order.
            buttons.push(combat(200.0, [7, 4, 5, 6, 6]));
            let (defend, attack) = (Order::Defend, Order::Attack);
            let defending = order == defend;
            buttons.push(icon(
                232.0,
                defending,
                Some(Bank::Order(defend)),
                [0x12, 0xf, 0x10, 0x11, 0x13],
            ));
            buttons.push(icon(
                262.0,
                !defending,
                Some(Bank::Order(attack)),
                [0xd, 0xa, 0xb, 0xc, 0xe],
            ));
            let face = match face {
                RoundButton::StartRound => &art.attack_confirm,
                RoundButton::GoContinuous => &art.cont_toggle,
                RoundButton::GoPhased => &art.combat_stop,
                RoundButton::StartContinuous => &art.cont_start,
            };
            let face = button(sheet(face), 0, Vec2::new(610.0, 450.0));
            round = Some(bsn! { @{face} Bank::Round on(press) });
        }
        None => buttons.push(combat(199.0, [3, 0, 1, 2, 2])),
    }
    // In order: neighbouring buttons may overlap.
    let mut drawn: Vec<Box<dyn Scene>> = vec![];
    for BankButton { sheet, at, frames, pressed, action } in buttons {
        // Camp and search are grayed in a fight.
        let fighting = view.fight.is_some();
        let action = action.filter(|a| !(fighting && matches!(a, Bank::Camp | Bank::Search)));
        let Some(action) = action else {
            drawn.push(Box::new(sprite(sheet, frames.grayed, at)));
            continue;
        };
        let (image, hover) =
            if pressed { (frames.on, frames.on_hovered) } else { (frames.off, frames.hovered) };
        drawn.push(Box::new(bsn! {
            @button(sheet, image, at)
            @hover_art(sheet, image, hover)
            action
            on(press)
        }));
    }
    commands
        .entity(node)
        .despawn_related::<Children>()
        .queue_spawn_related_scenes::<Children>(bsn_list! { {drawn} -- {round} });
    bank.drawn = Some(view);
}

fn press(
    click: On<PointerClick>,
    buttons: Query<&Bank>,
    mut camp: MessageWriter<CampRequest>,
    mut search: MessageWriter<ToggleSearch>,
    mut layout: ResMut<ScreenLayout>,
    mut orders: Query<&mut Order>,
    roster: Res<PartyRoster>,
    selected: Res<SelectedMember>,
    mut round: MessageWriter<StartRound>,
    mut settings: ResMut<CombatSettings>,
    battle: Option<Res<Battle>>,
) {
    match buttons.get(click.entity) {
        Ok(Bank::Order(order)) => {
            if let Some(member) = selected.member(&roster)
                && let Ok(mut current) = orders.get_mut(member)
            {
                current.set_if_neq(*order);
            }
        }
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
        Query<&Order>,
    ),
    mut captions: Query<(Entity, &mut Caption)>,
) {
    let Ok((node, mut caption)) = captions.single_mut() else { return };
    let catalog = data.and_then(|d| catalogs.get(&d.items));
    let lines = match selected.member(&roster).and_then(|m| members.get(m).ok()) {
        None => vec![],
        Some((character, carried)) => {
            let first = format!("{} - {:?}", character.name, character.profession);
            let second = if battle.is_some() {
                match selected
                    .member(&roster)
                    .and_then(|m| orders.get(m).ok())
                    .copied()
                    .unwrap_or_default()
                {
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
            let second = second.map(|line| {
                if battle.as_ref().is_some_and(|b| b.executing) {
                    format!("Next: {line}")
                } else {
                    line
                }
            });
            std::iter::once(first).chain(second).collect()
        }
    };
    if caption.drawn.as_ref() == Some(&lines) {
        return;
    }
    let font = loaded(&sheets, &art.small_font);
    let texts: Vec<_> = lines
        .iter()
        .zip([454.0, 465.0])
        .map(|(line, y)| text(font, &mut images, line, Vec2::new(343.0, y), 185.0))
        .collect();
    commands
        .entity(node)
        .despawn_related::<Children>()
        .queue_spawn_related_scenes::<Children>(texts);
    caption.drawn = Some(lines);
}
