//! The button row (retail `pc.cpp` positions and frames): the button bank,
//! the combat round button and the round's progress along the roof, the
//! previous and next character arrows, and the selected member's caption.
//! Journal and options open full screens through the shared gameplay gate.

use crate::presentation::retail_controls::{ControlSkin, set_enabled};
use bevy::prelude::*;
use bevy::ui::Selected;
use bevy::ui_widgets::Activate;

use super::{MainScreenArt, Panel, ScreenLayout, SelectedMember, layer};
use crate::content::ui::SpriteSheet;
use crate::game::camp::CampRequest;
use crate::game::characters::{Character, Vitals};
use crate::game::combat::{Battle, CombatSettings, StartRound};
use crate::game::inventory::{Carried, EquipSlot};
use crate::game::party::Order;
use crate::game::party::roster::PartyRoster;
use crate::game::world::search::{Search, ToggleSearch};
use crate::presentation::screen_art::visible_if;
use crate::presentation::screen_art::{BitmapText, Picture, PictureCrop, label, picture};
use bevy::picking::hover::Hovered;

/// Native nodes retain the buttons' input and display lifetimes.
#[derive(Component, Clone, Default)]
pub struct ButtonBank;

#[derive(Component, Clone, Default)]
pub struct CaptionLine(usize);

#[derive(Component, Clone, Default)]
pub struct RoundProgress;

#[derive(Component, Clone, Copy, Default, PartialEq, Eq)]
enum Bank {
    #[default]
    Camp,
    Journal,
    Options,
    Search,
    Formation,
    Order(Order),
    Round,
    EndCombat,
    /// Use Item / Cast Spell: open the magic menu.
    Magic {
        items: bool,
    },
    /// Party Move: no move, walk, run, in turn.
    PartyMove,
}

#[derive(Component)]
#[require(Hovered)]
pub struct BankFace {
    kind: Option<Bank>,
    sheet: Handle<SpriteSheet>,
}

fn bank_button(
    sheet: Handle<SpriteSheet>,
    at: Vec2,
    frames: [usize; 5],
    kind: Option<Bank>,
) -> impl Scene {
    let [disabled, normal, hovered, pressed, pressed_hovered] = frames;
    let skin = ControlSkin::new(normal, hovered, pressed, pressed_hovered, disabled);
    bsn! {
        @crate::presentation::retail_controls::image_button(sheet.clone(),at,skin)
        template(move |_| Ok(BankFace {kind,sheet:sheet.clone()}))
        on(press)
    }
}

pub fn row(art: &MainScreenArt, _sheets: &Assets<SpriteSheet>) -> impl SceneList {
    let progress = picture(art.progress.clone(), 0, Vec2::new(177.0, 2.0));
    let steps:Vec<_>=[(0,300.0,-1_i32),(4,323.0,1)].into_iter().map(|(frame,x,step)|bsn!{
        @crate::presentation::retail_controls::image_button(art.scroll_arrows.clone(),Vec2::new(x,456.0),ControlSkin::new(frame,frame+1,frame+2,frame+2,frame+3))
        on(move |_:On<Activate>,roster:Single<&PartyRoster>,mut selected:ResMut<SelectedMember>|{
            let members=roster.len() as i32;
            if members>0{let current=selected.member(&roster).and_then(|m|roster.get_index_of(&m)).unwrap_or(0);
                selected.0=roster.get_index((current as i32+step).rem_euclid(members) as usize).copied();}
        })
    }).collect();
    let icon = |x, frames, kind| bank_button(art.icons.clone(), Vec2::new(x, 452.0), frames, kind);
    let buttons = bsn_list! {
        @bank_button(art.options.clone(),Vec2::new(0.0,450.0),[3,0,1,2,2],Some(Bank::Options))
        -- @icon(40.0,[0x2b,0x28,0x29,0x2a,0x2c],Some(Bank::Journal))
        -- @icon(70.0,[3,0,1,2,4],Some(Bank::Camp))
        -- @icon(100.0,[8,5,6,7,9],Some(Bank::Search))
        -- @icon(130.0,[0x17,0x14,0x15,0x16,0x18],Some(Bank::Magic { items: true }))
        -- @icon(160.0,[0x1c,0x19,0x1a,0x1b,0x1d],Some(Bank::Magic { items: false }))
        -- @icon(541.0,[0x21,0x1e,0x1f,0x20,0x22],Some(Bank::PartyMove))
        -- @icon(571.0,[0x26,0x23,0x24,0x25,0x27],Some(Bank::Formation))
        -- @bank_button(art.combat_toggle.clone(),Vec2::new(199.0,452.0),[3,0,1,2,2],Some(Bank::EndCombat))
        -- @icon(232.0,[0x12,0xf,0x10,0x11,0x13],Some(Bank::Order(Order::Defend)))
        -- @icon(262.0,[0xd,0xa,0xb,0xc,0xe],Some(Bank::Order(Order::Attack)))
        -- @bank_button(art.attack_confirm.clone(),Vec2::new(610.0,450.0),[0;5],Some(Bank::Round))
    };
    bsn_list! {
        @layer() ButtonBank Children [{buttons}]
        -- @label(art.small_font.clone(),Vec2::new(343.0,454.0),185.0) CaptionLine(0)
        -- @label(art.small_font.clone(),Vec2::new(343.0,465.0),185.0) CaptionLine(1)
        -- @{progress} RoundProgress template(|_| Ok(PictureCrop::horizontal(0.0, 0.0))) Visibility::Hidden
        -- {steps}
    }
}

pub fn draw_bank(
    art: Res<MainScreenArt>,
    layout: Res<ScreenLayout>,
    selected: Res<SelectedMember>,
    roster: Single<&PartyRoster>,
    battle: Option<Single<&Battle>>,
    orders: Query<&Order>,
    settings: Res<CombatSettings>,
    search: Res<Search>,
    mut commands: Commands,
    mut buttons: Query<(
        Entity,
        &BankFace,
        &ControlSkin,
        Has<Selected>,
        Has<bevy::ui::InteractionDisabled>,
        &mut Picture,
        &mut Visibility,
    )>,
) {
    let fighting = battle.is_some();
    let order =
        selected.member(&roster).and_then(|m| orders.get(m).ok()).copied().unwrap_or_default();
    for (entity, face, skin, selected, disabled, mut picture, mut visible) in &mut buttons {
        let combat_only = matches!(face.kind, Some(Bank::Order(_) | Bank::Round | Bank::PartyMove));
        visible.set_if_neq(visible_if(!combat_only || fighting));
        let enabled = face.kind.is_some_and(|kind| {
            !matches!(kind, Bank::EndCombat)
                && !(fighting && matches!(kind, Bank::Camp | Bank::Search))
                // Choices are made between rounds.
                && !(matches!(kind, Bank::Magic { .. } | Bank::PartyMove) && battle.as_ref().is_some_and(|b| b.executing()))
                && !(combat_only && !fighting)
        });
        set_enabled(&mut commands, entity, enabled, disabled);
        let selected_state = match face.kind {
            Some(Bank::Search) => search.on(),
            Some(Bank::Formation) => layout.shows(Panel::Formation),
            Some(Bank::Order(wanted)) => wanted == order,
            Some(Bank::PartyMove) => battle.as_ref().is_some_and(|b| b.party_pace.is_some()),
            _ => false,
        };
        if selected_state != selected {
            if selected_state {
                commands.entity(entity).insert(Selected);
            } else {
                commands.entity(entity).remove::<Selected>();
            }
        }
        let sheet = match face.kind {
            Some(Bank::Round) => {
                match (settings.continuous, battle.as_ref().is_some_and(|b| b.executing())) {
                    (false, false) => &art.attack_confirm,
                    (false, true) => &art.cont_toggle,
                    (true, true) => &art.combat_stop,
                    (true, false) => &art.cont_start,
                }
            }
            Some(Bank::EndCombat) => {
                let end_skin = ControlSkin::new(0, 0, 0, 0, if fighting { 7 } else { 3 });
                if *skin != end_skin {
                    commands.entity(entity).insert(end_skin);
                }
                if picture.at.x != if fighting { 200.0 } else { 199.0 } {
                    picture.at.x = if fighting { 200.0 } else { 199.0 };
                }
                &face.sheet
            }
            _ => &face.sheet,
        };
        let at = picture.at;
        let bounds = if !enabled {
            if matches!(face.kind, Some(Bank::EndCombat)) && fighting { 7 } else { skin.disabled }
        } else if selected_state {
            skin.pressed
        } else {
            skin.normal
        };
        picture.set_if_neq(Picture {
            sheet: sheet.clone(),
            frame: picture.frame,
            at,
            bounds: Some(bounds),
        });
    }
}

fn press(
    click: On<Activate>,
    buttons: Query<&BankFace, Without<bevy::ui::InteractionDisabled>>,
    mut camp: MessageWriter<CampRequest>,
    mut journal: MessageWriter<crate::presentation::journal::OpenJournal>,
    mut options: MessageWriter<crate::presentation::options::OpenOptions>,
    mut search: MessageWriter<ToggleSearch>,
    mut layout: ResMut<ScreenLayout>,
    mut orders: MessageWriter<crate::game::combat::GiveOrder>,
    roster: Single<&PartyRoster>,
    selected: Res<SelectedMember>,
    mut round: MessageWriter<StartRound>,
    mut settings: ResMut<CombatSettings>,
    battle: Option<Single<&Battle>>,
    mut commands: Commands,
    views: Query<Entity, With<super::magic_view::MagicView>>,
    magic_art: Res<super::magic_view::MagicViewArt>,
    mut party_moves: MessageWriter<crate::game::combat::CyclePartyMove>,
) {
    match buttons.get(click.entity).ok().and_then(|face| face.kind) {
        Some(Bank::Order(order)) if battle.is_some() => {
            if let Some(member) = selected.member(&roster) {
                orders.write(crate::game::combat::GiveOrder { member, order });
            }
        }
        Some(Bank::Round) if battle.is_some() => {
            if battle.is_some_and(|b| b.executing()) {
                settings.continuous = !settings.continuous;
            } else {
                round.write(StartRound);
            }
        }
        Some(Bank::Journal) => {
            journal.write(crate::presentation::journal::OpenJournal);
        }
        Some(Bank::Options) => {
            options.write(crate::presentation::options::OpenOptions);
        }
        Some(Bank::Camp) => {
            camp.write(CampRequest);
        }
        Some(Bank::Search) => {
            search.write(ToggleSearch);
        }
        // The Party Formation button opens and closes the formation panel.
        Some(Bank::Formation) => layout.toggle(Panel::Formation),
        Some(Bank::PartyMove) if battle.is_some() => {
            party_moves.write(crate::game::combat::CyclePartyMove);
        }
        Some(Bank::Magic { items }) => {
            // The same button closes its view (retail toggles it).
            let open = !views.is_empty();
            for view in &views {
                commands.entity(view).despawn();
            }
            if !open && let Some(member) = selected.member(&roster) {
                super::magic_view::open(&mut commands, &magic_art, member, items, None);
            }
        }
        _ => {}
    }
}

/// Show the round's progress along the roof while one plays.
pub fn show_round_progress(
    battle: Option<Single<&Battle>>,
    mut bars: Query<(&mut PictureCrop, &mut Visibility), With<RoundProgress>>,
) {
    let progress = battle.and_then(|b| b.progress());
    for (mut crop, mut visibility) in &mut bars {
        match progress {
            Some(progress) => {
                crop.set_if_neq(PictureCrop::horizontal(0.0, 286.0 * progress));
                visibility.set_if_neq(Visibility::Inherited);
            }
            None => {
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
    }
}

/// Captions are two stable text controls, independent of button input.
pub fn draw_caption(
    pool: Single<&crate::game::inventory::PartyInventory>,
    selected: Res<SelectedMember>,
    roster: Single<&PartyRoster>,
    members: Query<(&Character, &Vitals, Option<&Carried>)>,
    content: crate::game::data::Content,
    battle: Option<Single<&Battle>>,
    orders: Query<&Order>,
    mut captions: Query<(&CaptionLine, &mut BitmapText)>,
    strings: crate::presentation::screen_art::GameStrings,
) {
    let spells = content.spells();
    let catalog = content.items();
    let (first, second) = match selected.member(&roster).and_then(|m| members.get(m).ok()) {
        None => (String::new(), String::new()),
        Some((profile, vitals, carried)) => {
            let first = format!(
                "{} - {}",
                profile.name,
                strings.get(crate::content::ui::PROFESSION_NAMES + profile.profession as usize)
            );
            let mut second = if battle.is_some() {
                match selected
                    .member(&roster)
                    .and_then(|m| orders.get(m).ok())
                    .copied()
                    .unwrap_or_default()
                {
                    Order::Defend => "Defend".into(),
                    Order::Attack => {
                        let weapon = carried
                            .and_then(|c| c.worn(EquipSlot::Primary))
                            .and_then(|i| catalog?.get(i.item))
                            .map_or("Unarmed", |d| d.name.as_str());
                        format!("Attack - {weapon}")
                    }
                    Order::Magic(choice) => match choice.source {
                        crate::game::magic::casting::Source::Spell { spell, power } => {
                            let name = spells.and_then(|s| s.get(spell)).map_or("", |s| &s.name);
                            format!("Cast {name} ({power})")
                        }
                        crate::game::magic::casting::Source::PoolItem(id) => {
                            let name = pool
                                .pool
                                .stacks
                                .iter()
                                .find(|s| s.id == id)
                                .and_then(|s| catalog?.get(s.item))
                                .map_or("", |d| d.name.as_str());
                            format!("Use {name} from party pool")
                        }
                        crate::game::magic::casting::Source::Item(id) => {
                            let name = carried
                                .and_then(|c| c.find_stack(id))
                                .and_then(|i| catalog?.get(i.item))
                                .map_or("", |d| d.name.as_str());
                            format!("Use {name}")
                        }
                    },
                }
            } else {
                crate::presentation::screen_art::condition_icon(vitals)
                    .map(|mut c| {
                        c[..1].make_ascii_uppercase();
                        c
                    })
                    .unwrap_or_default()
            };
            if battle.as_ref().is_some_and(|b| b.executing()) && !second.is_empty() {
                second = format!("Next: {second}");
            }
            (first, second)
        }
    };
    for (line, mut label) in &mut captions {
        let value = if line.0 == 0 { &first } else { &second };
        if label.value != *value {
            label.value = value.clone();
        }
    }
}
