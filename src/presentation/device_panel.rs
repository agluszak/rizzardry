//! The panel for working on a device: a lock's tumblers, a trap's
//! components, or a container's contents (retail's lock and trap panels and
//! `W8TriggerItemPickerDialog`), in native Bevy UI with the retail text font.
//!
//! The panel shows one [`DeviceInteraction`] attempt and lives exactly as
//! long ([`PanelOf`]); everything it submits names that attempt, so a click
//! left over from an attempt that has ended changes nothing. It submits for
//! the selected member, and the game decides what comes of it. It owns only
//! drafts: which trap type the party takes it for, and which stacks, and how
//! much of each, are chosen to take.
//!
//! Input: the panel takes keyboard focus when it opens. Tab and Shift+Tab move
//! among its enabled controls, Enter or Space activates one, and Escape leaves
//! (closes a container). The party stays put while it is up, so world
//! movement keys do nothing; portraits stay clickable to choose who works,
//! as retail allows.

use super::main_screen::magic_view::AttemptCasting;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
use bevy::input_focus::{FocusedInput, InputFocus};
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::{Activate, Button};

use super::main_screen::SelectedMember;
use super::retail_controls::FocusRing;
use super::screen_art::{BitmapText, visible_if};
use crate::content::level::TRAP_TYPES;
use crate::content::ui::SpriteSheet;
use crate::game::characters::{Character, Vitals};
use crate::game::inventory::StackId;
use crate::game::party::roster::PartyRoster;
use crate::game::world::containers::{Container, Recipient};
use crate::game::world::interaction::{
    self, AttemptOn, COMPONENTS, DeviceInteraction, DeviceRequest, Observation, TRAP_COMPONENTS,
    Work,
};
use crate::game::world::props::PropMotion;

const WIDTH: f32 = 380.0;
const BUTTON: Color = Color::srgb(0.22, 0.17, 0.11);
const BUTTON_OFF: Color = Color::srgb(0.1, 0.08, 0.06);
const CHOSEN: Color = Color::srgb(0.4, 0.3, 0.12);

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, load_device_panel_art).add_observer(open_panel).add_systems(
        Update,
        (show_panel, sync_loot_rows, draw_device_panel)
            .chain()
            .after(crate::game::GameplaySet::LevelActivation),
    );
}

#[derive(Resource)]
struct DevicePanelArt {
    font: Handle<SpriteSheet>,
}

fn load_device_panel_art(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(DevicePanelArt {
        font: assets.load("retail://data/fonts/wiz_text_font.sti"),
    });
}

/// The attempt a panel shows.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = AttemptPanel)]
pub struct PanelOf(pub Entity);

/// An attempt's panel, despawned with it.
#[derive(Component, Debug)]
#[relationship_target(relationship = PanelOf, linked_spawn)]
pub struct AttemptPanel(Entity);

/// A panel's drafts.
#[derive(Component, Debug, Default)]
struct DevicePanel {
    /// The trap type the party takes the trap for (retail's list of trap
    /// names): it picks which components can be worked on.
    guess: usize,
    /// Stacks chosen to take, and how much of each.
    chosen: Vec<(StackId, u16)>,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Tumbler(usize),
    Force,
    Cast,
    UseItem,
    PreviousTrap,
    NextTrap,
    Component(usize),
    Inspect,
    Spring,
    Row(StackId),
    Less(StackId),
    More(StackId),
    All,
    TakeToParty,
    TakeToMember,
    Leave,
}

/// A line of text the panel keeps current.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum Caption {
    Title,
    Status,
    /// The trap type the party takes it for.
    Guess,
    Control(Control),
}

/// The list of a container's stacks, on the panel it belongs to.
#[derive(Component, Debug)]
struct LootRows {
    panel: Entity,
}

/// One stack's row, kept while the stack stays in the container.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct LootRow(StackId);

/// Build an attempt's panel when the attempt begins.
fn open_panel(
    add: On<Add<DeviceInteraction>>,
    attempts: Query<&DeviceInteraction>,
    art: Option<Res<DevicePanelArt>>,
    focus: Option<ResMut<InputFocus>>,
    mut commands: Commands,
) {
    let attempt = add.entity;
    let (Ok(interaction), Some(art)) = (attempts.get(attempt), art) else { return };
    let font = art.font.clone();
    let root = commands
        .spawn((
            Name::new("Device panel"),
            PanelOf(attempt),
            DevicePanel::default(),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(10),
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .id();
    let body = commands
        .spawn((
            Node {
                width: px(WIDTH),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(6),
                padding: UiRect::all(px(8)),
                border: UiRect::all(px(1)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.04, 0.03, 0.92)),
            BorderColor::all(Color::srgb(0.55, 0.45, 0.3)),
            TabGroup::modal(),
            ChildOf(root),
        ))
        .observe(
            move |mut key: On<FocusedInput<KeyboardInput>>,
                  mut requests: MessageWriter<DeviceRequest>| {
                if key.input.state.is_pressed() && key.input.logical_key == Key::Escape {
                    key.propagate(false);
                    requests.write(DeviceRequest::Leave { on: attempt });
                }
            },
        )
        .id();
    caption(&mut commands, body, &font, Caption::Title);
    caption(&mut commands, body, &font, Caption::Status);
    let row = |commands: &mut Commands| {
        commands
            .spawn((
                Node {
                    flex_wrap: FlexWrap::Wrap,
                    justify_content: JustifyContent::Center,
                    column_gap: px(4),
                    row_gap: px(4),
                    ..default()
                },
                ChildOf(body),
            ))
            .id()
    };
    let first = match &interaction.work {
        Work::Picking(picking) => {
            let tumblers = row(&mut commands);
            let first: Vec<_> = (0..picking.tumblers.len())
                .map(|index| button(&mut commands, tumblers, &font, Control::Tumbler(index)))
                .collect();
            let actions = row(&mut commands);
            let force = button(&mut commands, actions, &font, Control::Force);
            button(&mut commands, actions, &font, Control::Cast);
            button(&mut commands, actions, &font, Control::UseItem);
            button(&mut commands, actions, &font, Control::Leave);
            first.first().copied().unwrap_or(force)
        }
        Work::Disarming(_) => {
            let guess = row(&mut commands);
            button(&mut commands, guess, &font, Control::PreviousTrap);
            caption(&mut commands, guess, &font, Caption::Guess);
            button(&mut commands, guess, &font, Control::NextTrap);
            let components = row(&mut commands);
            for index in 0..COMPONENTS {
                button(&mut commands, components, &font, Control::Component(index));
            }
            let actions = row(&mut commands);
            let inspect = button(&mut commands, actions, &font, Control::Inspect);
            button(&mut commands, actions, &font, Control::Cast);
            button(&mut commands, actions, &font, Control::UseItem);
            button(&mut commands, actions, &font, Control::Spring);
            button(&mut commands, actions, &font, Control::Leave);
            inspect
        }
        Work::Looting => {
            commands.spawn((
                LootRows { panel: root },
                Node { flex_direction: FlexDirection::Column, row_gap: px(2), ..default() },
                ChildOf(body),
            ));
            let actions = row(&mut commands);
            let all = button(&mut commands, actions, &font, Control::All);
            button(&mut commands, actions, &font, Control::TakeToParty);
            button(&mut commands, actions, &font, Control::TakeToMember);
            button(&mut commands, actions, &font, Control::Leave);
            all
        }
    };
    if let Some(mut focus) = focus {
        focus.set(first, bevy::input_focus::FocusCause::Navigated);
    }
}

/// A container's panel shows once its lid has stopped moving, as retail's
/// picker does; a panel hides while a spell or item is being chosen for its
/// attempt.
fn show_panel(
    mut panels: Query<(&PanelOf, &mut Visibility)>,
    attempts: Query<(&DeviceInteraction, &AttemptOn, Has<AttemptCasting>)>,
    motions: Query<&PropMotion>,
) {
    for (panel, mut visibility) in &mut panels {
        let Ok((interaction, on, casting)) = attempts.get(panel.0) else { continue };
        let settled = !matches!(interaction.work, Work::Looting)
            || motions.get(on.0).is_ok_and(|m| m.frame == m.target);
        visibility.set_if_neq(visible_if(settled && !casting));
    }
}

fn label(font: &Handle<SpriteSheet>) -> impl Bundle {
    (
        BitmapText { font: font.clone(), ..default() },
        Node { position_type: PositionType::Relative, ..default() },
    )
}

fn caption(commands: &mut Commands, parent: Entity, font: &Handle<SpriteSheet>, role: Caption) {
    commands.spawn((label(font), role, ChildOf(parent)));
}

/// A text button composed like the other retail controls: native button,
/// tab stop and focus ring.
fn button(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<SpriteSheet>,
    control: Control,
) -> Entity {
    let button = commands
        .spawn((
            Button,
            TabIndex(0),
            FocusRing,
            control,
            Node {
                padding: UiRect::axes(px(5), px(2)),
                border: UiRect::all(px(1)),
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BUTTON),
            BorderColor::all(Color::srgb(0.45, 0.36, 0.22)),
            ChildOf(parent),
        ))
        .observe(press)
        .id();
    commands.spawn((label(font), Caption::Control(control), ChildOf(button)));
    button
}

/// Keep one row per stack in the container, in its order: rows of stacks
/// that stay are kept as they are (with their focus and hover), rows of
/// stacks that went are removed, new stacks get new rows. Choices of
/// stacks that went are dropped, and focus on a removed row moves to the
/// row that took its place, or to the panel's first action.
fn sync_loot_rows(
    mut commands: Commands,
    mut panels: Query<(&PanelOf, &mut DevicePanel)>,
    attempts: Query<&AttemptOn>,
    lists: Query<(Entity, &LootRows, Option<&Children>)>,
    rows: Query<&LootRow>,
    controls: Query<(Entity, &Control)>,
    children: Query<&Children>,
    containers: Query<Ref<Container>>,
    mut focus: Option<ResMut<InputFocus>>,
    art: Res<DevicePanelArt>,
) {
    for (list, rows_of, existing) in &lists {
        let Ok((panel_of, mut panel)) = panels.get_mut(rows_of.panel) else { continue };
        let Ok(on) = attempts.get(panel_of.0) else { continue };
        let Ok(container) = containers.get(on.0) else { continue };
        let existing: Vec<(Entity, StackId)> = existing
            .map(|c| c.iter().filter_map(|r| rows.get(r).ok().map(|row| (r, row.0))).collect())
            .unwrap_or_default();
        let stacks: Vec<StackId> = container.contents.items.iter().map(|s| s.id).collect();
        if existing.iter().map(|(_, id)| *id).eq(stacks.iter().copied()) {
            continue;
        }
        if panel.chosen.iter().any(|(id, _)| !stacks.contains(id)) {
            panel.chosen.retain(|(id, _)| stacks.contains(id));
        }
        let focused = focus.as_ref().and_then(|f| f.get());
        let mut lost_focus_at = None;
        for (index, &(row, id)) in existing.iter().enumerate() {
            if !stacks.contains(&id) {
                let holds_focus = focused
                    .is_some_and(|f| f == row || children.iter_descendants(row).any(|d| d == f));
                if holds_focus {
                    lost_focus_at = Some(index);
                }
                commands.entity(row).despawn();
            }
        }
        let mut ordered = Vec::with_capacity(stacks.len());
        for &id in &stacks {
            match existing.iter().find(|(_, existing)| *existing == id) {
                Some(&(row, _)) => ordered.push(row),
                None => {
                    let row =
                        commands.spawn((LootRow(id), Node { column_gap: px(4), ..default() })).id();
                    button(&mut commands, row, &art.font, Control::Less(id));
                    button(&mut commands, row, &art.font, Control::More(id));
                    button(&mut commands, row, &art.font, Control::Row(id));
                    ordered.push(row);
                }
            }
        }
        commands.entity(list).replace_children(&ordered);
        if let (Some(at), Some(focus)) = (lost_focus_at, focus.as_mut()) {
            // The stack that now stands where the focused one stood, if any.
            let survivor = stacks
                .get(at.min(stacks.len().saturating_sub(1)))
                .filter(|_| !stacks.is_empty())
                .and_then(|&id| {
                    controls.iter().find(|(_, c)| **c == Control::Row(id)).map(|(e, _)| e)
                });
            let fallback = controls.iter().find(|(_, c)| **c == Control::All).map(|(e, _)| e);
            if let Some(target) = survivor.or(fallback) {
                focus.set(target, bevy::input_focus::FocusCause::Navigated);
            } else {
                focus.clear();
            }
        }
    }
}

/// Bring every caption and button up to date with the interaction, the
/// selected member and the drafts.
fn draw_device_panel(
    difficulty: Res<crate::game::difficulty::Difficulty>,
    mut commands: Commands,
    attempts: Query<(&DeviceInteraction, &AttemptOn)>,
    panels: Query<(&PanelOf, &DevicePanel)>,
    selected: Res<SelectedMember>,
    roster: Single<&PartyRoster>,
    members: Query<(&Character, &Vitals)>,
    containers: Query<&Container>,
    content: If<Res<crate::game::data::Catalogs>>,
    strings: crate::presentation::screen_art::GameStrings,
    mut captions: Query<(&Caption, &mut BitmapText)>,
    mut buttons: Query<(Entity, &Control, &mut BackgroundColor, Has<InteractionDisabled>)>,
) {
    let Ok((panel_of, panel)) = panels.single() else { return };
    let Ok((interaction, on)) = attempts.get(panel_of.0) else { return };
    let member = selected.member(&roster).and_then(|m| Some((m, members.get(m).ok()?)));
    let name = member.map_or(String::new(), |(_, (c, _))| c.name.clone());
    let skill = member.and_then(|(_, (c, v))| interaction::lock_skill(c, v));
    let trap_name = |trap: usize| {
        let name = strings.get(crate::content::ui::TRAP_NAMES + trap);
        if name.is_empty() { "Trap" } else { name }.to_owned()
    };
    let catalog = &content.items;
    let stacks = containers.get(on.0).map_or(&[][..], |c| &c.contents.items[..]);
    let percent = |value: Option<i64>| value.map_or("-".to_owned(), |v| format!("{v}%"));

    let text = |role: Caption| -> String {
        match (role, &interaction.work) {
            (Caption::Title, Work::Picking(picking)) => {
                format!("Locked: {} tumblers", picking.tumblers.len())
            }
            (Caption::Title, Work::Disarming(_)) => "Trapped".to_owned(),
            (Caption::Title, Work::Looting) => "Contents".to_owned(),
            (Caption::Status, Work::Picking(picking)) => {
                let force = member.and_then(|(_, (c, v))| {
                    interaction::force_chance(c, v, picking.tumblers.len(), *difficulty)
                });
                format!(
                    "{name}: Locks & Traps {}, Force {}",
                    percent(skill.map(i64::from)),
                    percent(force.map(i64::from))
                )
            }
            (Caption::Status, Work::Disarming(disarming)) => format!(
                "{name}: Inspect {} / Disarm {}",
                percent(
                    skill.map(|s| i64::from(disarming.inspection_chance(Some(s), *difficulty)))
                ),
                percent(skill.map(|s| i64::from(disarming.disarm_chance(Some(s), *difficulty))))
            ),
            (Caption::Status, Work::Looting) => name.clone(),
            (Caption::Guess, _) => trap_name(panel.guess),
            (Caption::Control(control), work) => match (control, work) {
                (Control::Tumbler(index), Work::Picking(picking)) => {
                    match picking.tumblers.get(index) {
                        Some(pin) if pin.raised() => format!("{} up", index + 1),
                        _ => format!("{}", index + 1),
                    }
                }
                (Control::Component(index), Work::Disarming(disarming)) => {
                    let mark = if disarming.disarmed[index] {
                        "ok"
                    } else {
                        match member.and_then(|(m, _)| disarming.observed(m)).map(|s| s[index]) {
                            Some(Observation::Present) => "!",
                            Some(Observation::Absent) => "-",
                            Some(Observation::Unknown) | None => "?",
                        }
                    };
                    format!("{} {mark}", index + 1)
                }
                (Control::Row(id), Work::Looting) => {
                    let Some(stack) = stacks.iter().find(|s| s.id == id) else {
                        return String::new();
                    };
                    let item = catalog
                        .get(stack.item)
                        .map_or(String::new(), |d| stack.display_name(d, false));
                    match panel.chosen.iter().find(|(c, _)| *c == id) {
                        Some((_, amount)) if stack.count() > 1 => {
                            format!("[{amount}/{}] {item}", stack.count())
                        }
                        Some(_) => format!("[x] {item}"),
                        None if stack.count() > 1 => format!("[ ] {item} ({})", stack.count()),
                        None => format!("[ ] {item}"),
                    }
                }
                (Control::Less(_), _) => "-".to_owned(),
                (Control::More(_), _) => "+".to_owned(),
                (Control::Force, _) => "Force".to_owned(),
                (Control::PreviousTrap, _) => "<".to_owned(),
                (Control::NextTrap, _) => ">".to_owned(),
                (Control::Inspect, _) => "Inspect".to_owned(),
                (Control::Cast, _) => "Spell".to_owned(),
                (Control::UseItem, _) => "Use item".to_owned(),
                (Control::Spring, _) => "Open".to_owned(),
                (Control::All, _) => "All".to_owned(),
                (Control::TakeToParty, _) => "Take".to_owned(),
                (Control::TakeToMember, _) => format!("Give {name}"),
                (Control::Leave, Work::Looting) => "Close".to_owned(),
                (Control::Leave, _) => "Leave".to_owned(),
                _ => String::new(),
            },
        }
    };
    for (role, mut label) in &mut captions {
        let value = text(*role);
        if label.value != value {
            label.value = value;
        }
    }
    let enabled = |control: Control| -> bool {
        match (control, &interaction.work) {
            (Control::Tumbler(index), Work::Picking(picking)) => {
                skill.is_some() && picking.tumblers.get(index).is_some_and(|pin| !pin.raised())
            }
            (Control::Force, Work::Picking(picking)) => member.is_some_and(|(_, (c, v))| {
                interaction::force_chance(c, v, picking.tumblers.len(), *difficulty).is_some()
            }),
            (Control::Component(index), Work::Disarming(disarming)) => {
                skill.is_some() && TRAP_COMPONENTS[panel.guess][index] && !disarming.disarmed[index]
            }
            (Control::Inspect, _) => skill.is_some(),
            (Control::Cast | Control::UseItem, _) => {
                member.is_some_and(|(_, (_, vitals))| vitals.can_act())
            }
            (Control::Less(id) | Control::More(id), _) => {
                stacks.iter().find(|s| s.id == id).is_some_and(|s| s.count() > 1)
            }
            (Control::TakeToParty | Control::TakeToMember, _) => !panel.chosen.is_empty(),
            _ => true,
        }
    };
    for (entity, control, mut background, disabled) in &mut buttons {
        let on = enabled(*control);
        if on == disabled {
            if on {
                commands.entity(entity).remove::<InteractionDisabled>();
            } else {
                commands.entity(entity).insert(InteractionDisabled);
            }
        }
        let chosen =
            matches!(control, Control::Row(id) if panel.chosen.iter().any(|(c, _)| c == id));
        let colour = if chosen {
            CHOSEN
        } else if on {
            BUTTON
        } else {
            BUTTON_OFF
        };
        background.set_if_neq(BackgroundColor(colour));
    }
}

fn press(
    click: On<Activate>,
    magic_art: Option<Res<super::main_screen::magic_view::MagicViewArt>>,
    controls: Query<&Control, Without<InteractionDisabled>>,
    mut panels: Query<(&PanelOf, &mut DevicePanel)>,
    attempts: Query<&AttemptOn>,
    containers: Query<&Container>,
    selected: Res<SelectedMember>,
    roster: Single<&PartyRoster>,
    mut commands: Commands,
    mut requests: MessageWriter<DeviceRequest>,
) {
    let (Ok(&control), Ok((&PanelOf(on), mut panel))) =
        (controls.get(click.entity), panels.single_mut())
    else {
        return;
    };
    let Ok(&AttemptOn(device)) = attempts.get(on) else { return };
    let actor = selected.member(&roster);
    let count = |id: StackId| {
        containers
            .get(device)
            .ok()
            .and_then(|c| c.contents.items.iter().find(|s| s.id == id))
            .map_or(0, |s| s.count())
    };
    match control {
        Control::Cast | Control::UseItem => {
            if let (Some(actor), Some(art)) = (actor, magic_art.as_deref()) {
                super::main_screen::magic_view::open(
                    &mut commands,
                    art,
                    actor,
                    control == Control::UseItem,
                    Some(super::main_screen::magic_view::ViewTarget::Device(on)),
                );
            }
        }
        Control::Tumbler(tumbler) => {
            if let Some(actor) = actor {
                requests.write(DeviceRequest::Pick { on, actor, tumbler });
            }
        }
        Control::Force => {
            if let Some(actor) = actor {
                requests.write(DeviceRequest::Force { on, actor });
            }
        }
        Control::Inspect => {
            if let Some(actor) = actor {
                requests.write(DeviceRequest::Inspect { on, actor });
            }
        }
        Control::Component(component) => {
            if let Some(actor) = actor {
                requests.write(DeviceRequest::Disarm { on, actor, component });
            }
        }
        Control::Spring => {
            requests.write(DeviceRequest::Spring { on, actor });
        }
        Control::PreviousTrap => panel.guess = (panel.guess + TRAP_TYPES - 1) % TRAP_TYPES,
        Control::NextTrap => panel.guess = (panel.guess + 1) % TRAP_TYPES,
        Control::Row(id) => {
            if let Some(at) = panel.chosen.iter().position(|(c, _)| *c == id) {
                panel.chosen.remove(at);
            } else {
                let all = count(id);
                panel.chosen.push((id, all));
            }
        }
        Control::Less(id) | Control::More(id) => {
            let all = count(id);
            let step = |amount: u16| match control {
                Control::Less(_) => amount.saturating_sub(1).max(1),
                _ => (amount + 1).min(all),
            };
            match panel.chosen.iter_mut().find(|(c, _)| *c == id) {
                Some((_, amount)) => *amount = step(*amount),
                None => panel.chosen.push((
                    id,
                    if matches!(control, Control::Less(_)) {
                        all.saturating_sub(1).max(1)
                    } else {
                        1
                    },
                )),
            }
        }
        Control::All => {
            let ids: Vec<(StackId, u16)> = containers
                .get(device)
                .map(|c| c.contents.items.iter().map(|s| (s.id, s.count())).collect())
                .unwrap_or_default();
            panel.chosen = if panel.chosen.len() == ids.len() { vec![] } else { ids };
        }
        Control::TakeToParty | Control::TakeToMember => {
            let to = match control {
                Control::TakeToParty => Recipient::Party,
                _ => match actor {
                    Some(actor) => Recipient::Member(actor),
                    None => return,
                },
            };
            let picks = std::mem::take(&mut panel.chosen);
            requests.write(DeviceRequest::Take { on, picks, to });
        }
        Control::Leave => {
            requests.write(DeviceRequest::Leave { on });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::Skill;
    use crate::content::items::{ItemDefinition, ItemId, QuantityKind};
    use crate::content::level::Lock;
    use crate::content::ui::TextTable;
    use crate::game::characters::{Gender, Profession, Race};
    use crate::game::inventory::{ItemIds, ItemStack};
    use crate::game::world::interaction::{LockNews, Unable};

    fn app() -> (App, Entity) {
        let mut app = crate::presentation::test_input::input_app();
        app.add_plugins((
            plugin,
            crate::game::world::interaction::plugin,
            bevy::input_focus::InputDispatchPlugin,
            bevy::input_focus::tab_navigation::TabNavigationPlugin,
        ))
        .add_observer(crate::game::world::interaction::end_attempts)
        .init_asset::<SpriteSheet>()
        .init_asset::<TextTable>()
        .init_resource::<SelectedMember>()
        .init_resource::<ItemIds>()
        .init_resource::<crate::game::difficulty::Difficulty>()
        .insert_resource(crate::game::rng::GameRng::seeded(1))
        .add_message::<LockNews>();
        let mut profile = Character::new("A", Race::Human, Gender::Male, Profession::Rogue);
        profile.skill_points[Skill::LocksAndTraps] = 100;
        let member = app.world_mut().spawn((Vitals::new(&profile), profile)).id();
        crate::game::party::roster::test_party(app.world_mut(), &[member]);
        app.world_mut().resource_mut::<SelectedMember>().0 = Some(member);
        app.init_resource::<crate::game::time::GameClock>();
        app.finish();
        app.cleanup();
        // Startup loads the panel's art.
        app.update();
        (app, member)
    }

    fn controls(app: &mut App, wanted: impl Fn(Control) -> bool) -> Vec<Entity> {
        app.world_mut()
            .query::<(Entity, &Control)>()
            .iter(app.world())
            .filter(|(_, c)| wanted(**c))
            .map(|(e, _)| e)
            .collect()
    }

    fn work(app: &App) -> Option<&Work> {
        let (on, _) = interaction::current(app.world())?;
        app.world().get::<DeviceInteraction>(on).map(|i| &i.work)
    }

    fn device(app: &mut App, lock: Lock) -> Entity {
        app.world_mut()
            .spawn((
                crate::game::world::triggers::Device { definition: 0, recipients: vec![] },
                interaction::Guarded::new(lock),
            ))
            .id()
    }

    /// The panel follows the attempt and submits the selected member's
    /// picks; the game, not the panel, decides when the lock opens.
    #[test]
    fn the_lock_panel_picks_for_the_selected_member_and_closes_with_the_lock() {
        let (mut app, member) = app();
        let lock = Lock::Pickable { difficulty: 2 };
        let device = device(&mut app, lock);
        interaction::confront(app.world_mut(), device, lock);
        app.update();
        app.update();
        let tumblers = controls(&mut app, |c| matches!(c, Control::Tumbler(_)));
        assert_eq!(tumblers.len(), 2);
        // The panel takes focus so that its keys work at once.
        let focus = app.world().resource::<InputFocus>().get();
        assert!(focus.is_some_and(|f| tumblers.contains(&f)));
        app.world_mut().trigger(Activate { entity: tumblers[0] });
        app.update();
        let Some(Work::Picking(picking)) = work(&app) else { panic!("still picking") };
        assert_eq!(
            picking.tumblers.iter().filter(|t| **t == interaction::Tumbler::Held(member)).count(),
            1
        );
        app.world_mut().trigger(Activate { entity: tumblers[1] });
        app.update();
        app.update();
        assert!(interaction::current(app.world()).is_none());
        assert!(app.world().get::<interaction::Guarded>(device).is_none());
        assert!(app.world_mut().query::<&DevicePanel>().iter(app.world()).next().is_none());
    }

    /// Escape on the focused panel leaves the attempt, as its Leave button does.
    #[test]
    fn escape_leaves_the_attempt_the_panel_shows() {
        let (mut app, _) = app();
        let window = app.world_mut().spawn((Window::default(), bevy::window::PrimaryWindow)).id();
        let lock = Lock::Pickable { difficulty: 2 };
        let device = device(&mut app, lock);
        interaction::confront(app.world_mut(), device, lock);
        app.update();
        crate::presentation::test_input::tap(&mut app, window, KeyCode::Escape);
        app.update();
        assert!(interaction::current(app.world()).is_none());
        assert!(app.world().get::<interaction::Guarded>(device).is_some());
        assert!(app.world_mut().query::<&DevicePanel>().iter(app.world()).next().is_none());
    }

    /// A request names its attempt: one left over from an attempt that has
    /// ended does nothing to the attempt that replaced it.
    #[test]
    fn a_stale_request_cannot_reach_a_replacement_attempt() {
        let (mut app, member) = app();
        let lock = Lock::Pickable { difficulty: 2 };
        let device = device(&mut app, lock);
        interaction::confront(app.world_mut(), device, lock);
        let (first, _) = interaction::current(app.world()).unwrap();
        interaction::leave(app.world_mut(), first);
        interaction::confront(app.world_mut(), device, lock);
        let (second, _) = interaction::current(app.world()).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            interaction::pick_tumbler(app.world_mut(), first, member, 0),
            Err(Unable::NotNow)
        );
        let Some(Work::Picking(picking)) = work(&app) else { panic!("picking") };
        assert!(picking.tumblers.iter().all(|t| !t.raised()));
        // Leaving the ended attempt again leaves the new one alone.
        interaction::leave(app.world_mut(), first);
        assert_eq!(interaction::current(app.world()).map(|(a, _)| a), Some(second));
    }

    /// The attempt and its panel go with their device, and a fight ends
    /// them; nothing polls for either.
    #[test]
    fn an_attempt_and_its_panel_end_with_the_device_or_a_fight() {
        let (mut app, _) = app();
        let lock = Lock::Trap { difficulty: 1, kind: Some(0) };
        let device = device(&mut app, lock);
        interaction::confront(app.world_mut(), device, lock);
        app.update();
        assert!(app.world_mut().query::<&DevicePanel>().iter(app.world()).next().is_some());
        app.world_mut().despawn(device);
        assert!(interaction::current(app.world()).is_none());
        app.update();
        assert!(app.world_mut().query::<&DevicePanel>().iter(app.world()).next().is_none());
        let device = self::device(&mut app, lock);
        interaction::confront(app.world_mut(), device, lock);
        app.update();
        app.world_mut().spawn(crate::game::combat::Battle::default());
        app.update();
        assert!(interaction::current(app.world()).is_none());
        assert!(app.world_mut().query::<&DevicePanel>().iter(app.world()).next().is_none());
        assert!(app.world().get::<interaction::Guarded>(device).is_some());
    }

    /// Taking one stack keeps the other rows as they were; focus on a row
    /// that went moves to the row that took its place.
    #[test]
    fn container_rows_outlive_the_stacks_taken_around_them() {
        let (mut app, _) = app();
        let mut arrows = ItemDefinition { id: ItemId(0), name: "arrow".into(), ..default() };
        arrows.quantity = QuantityKind::Stack;
        arrows.stack_limit = 99;
        let stacks: Vec<ItemStack> = {
            let mut ids = app.world_mut().resource_mut::<ItemIds>();
            (1..=3).map(|n| ItemStack::new(&arrows, &mut ids, n, true)).collect()
        };
        let ids: Vec<StackId> = stacks.iter().map(|s| s.id).collect();
        let chest = app
            .world_mut()
            .spawn((
                crate::game::world::triggers::Device { definition: 0, recipients: vec![] },
                Container::holding(stacks),
            ))
            .id();
        interaction::begin(app.world_mut(), chest, Work::Looting);
        app.update();
        app.update();
        let rows = |app: &mut App| {
            app.world_mut()
                .query::<(Entity, &LootRow)>()
                .iter(app.world())
                .map(|(e, r)| (r.0, e))
                .collect::<std::collections::HashMap<_, _>>()
        };
        let before = rows(&mut app);
        assert_eq!(before.len(), 3);
        let middle = controls(&mut app, |c| c == Control::Row(ids[1]))[0];
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(middle, bevy::input_focus::FocusCause::Navigated);
        app.world_mut().get_mut::<Container>(chest).unwrap().contents.items.remove(1);
        app.update();
        let after = rows(&mut app);
        assert_eq!(after.len(), 2);
        assert_eq!(after[&ids[0]], before[&ids[0]]);
        assert_eq!(after[&ids[2]], before[&ids[2]]);
        assert!(app.world().get_entity(before[&ids[1]]).is_err());
        let third = controls(&mut app, |c| c == Control::Row(ids[2]))[0];
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(third));
        // Rows stay in the container's order.
        let list = app
            .world_mut()
            .query_filtered::<&Children, With<LootRows>>()
            .single(app.world())
            .unwrap();
        let order: Vec<_> =
            list.iter().filter_map(|e| app.world().get::<LootRow>(e)).map(|r| r.0).collect();
        assert_eq!(order, [ids[0], ids[2]]);
    }
}
