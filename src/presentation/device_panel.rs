//! The panel for working on a device: a lock's tumblers, a trap's
//! components, or a container's contents (retail's lock and trap panels and
//! `W8TriggerItemPickerDialog`), in native Bevy UI with the retail text font.
//!
//! The panel shows the [`DeviceInteraction`] and submits attempts for the
//! selected member; the game decides what comes of them. It owns only
//! drafts: which trap type the party takes it for, and which stacks, and
//! how much of each, are chosen to take.

use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::{Activate, Button};

use super::main_screen::SelectedMember;
use super::screen_art::BitmapText;
use crate::content::items::ItemCatalog;
use crate::content::level::TRAP_TYPES;
use crate::content::ui::{SpriteSheet, TextTable};
use crate::game::characters::{Character, Vitals};
use crate::game::data::GameData;
use crate::game::inventory::StackId;
use crate::game::party::roster::PartyRoster;
use crate::game::world::containers::{self, Recipient};
use crate::game::world::interaction::{
    self, COMPONENTS, DeviceInteraction, Observation, TRAP_COMPONENTS, Work,
};
use crate::game::world::props::PropMotion;
use crate::game::world::triggers::Container;

/// The trap types' names in the game's string table.
const TRAP_NAMES: usize = 0x542;
const WIDTH: f32 = 380.0;
const BUTTON: Color = Color::srgb(0.22, 0.17, 0.11);
const BUTTON_OFF: Color = Color::srgb(0.1, 0.08, 0.06);
const CHOSEN: Color = Color::srgb(0.4, 0.3, 0.12);

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, load_device_panel_art).add_systems(
        Update,
        (sync_device_panel, sync_loot_rows, draw_device_panel)
            .chain()
            .after(crate::game::GameplaySet::LevelActivation),
    );
}

#[derive(Resource)]
struct DevicePanelArt {
    font: Handle<SpriteSheet>,
    strings: Handle<TextTable>,
}

fn load_device_panel_art(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(DevicePanelArt {
        font: assets.load("retail://data/fonts/wiz_text_font.sti"),
        strings: assets.load("retail://data/strings/stringdata.dat"),
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Lock,
    Trap,
    Loot,
}

fn kind(work: &Work) -> Kind {
    match work {
        Work::Picking(_) => Kind::Lock,
        Work::Disarming(_) => Kind::Trap,
        Work::Looting => Kind::Loot,
    }
}

/// The panel for one interaction, with its drafts.
#[derive(Component, Debug)]
struct DevicePanel {
    device: Entity,
    kind: Kind,
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

/// The container rows, rebuilt when its contents change.
#[derive(Component, Debug, Default)]
struct LootRows(Vec<StackId>);

/// Show the panel while an interaction lasts; a container's opens once its
/// lid has finished moving, as retail's picker does.
fn sync_device_panel(
    mut commands: Commands,
    interaction: Option<Res<DeviceInteraction>>,
    mut panels: Query<(Entity, &DevicePanel, &mut Visibility)>,
    magic_views: Query<&super::main_screen::magic_view::MagicView>,
    motions: Query<&PropMotion>,
    art: Res<DevicePanelArt>,
) {
    let wanted =
        interaction.as_ref().map(|i| (i.device, kind(&i.work))).filter(|(device, kind)| {
            *kind != Kind::Loot || motions.get(*device).is_ok_and(|m| m.frame == m.target)
        });
    for (entity, panel, mut visibility) in &mut panels {
        visibility.set_if_neq(
            if magic_views.iter().any(|view| view.device == Some(panel.device)) {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            },
        );
        if wanted != Some((panel.device, panel.kind)) {
            commands.entity(entity).despawn();
        }
    }
    let Some((device, kind)) = wanted else { return };
    if panels.iter().any(|(_, p, _)| (p.device, p.kind) == (device, kind)) {
        return;
    }
    let font = art.font.clone();
    let root = commands
        .spawn((
            Name::new("Device panel"),
            DevicePanel { device, kind, guess: 0, chosen: vec![] },
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(10),
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
            ChildOf(root),
        ))
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
    match kind {
        Kind::Lock => {
            let count = match interaction.as_ref().map(|i| &i.work) {
                Some(Work::Picking(picking)) => picking.tumblers.len(),
                _ => 0,
            };
            let tumblers = row(&mut commands);
            for index in 0..count {
                button(&mut commands, tumblers, &font, Control::Tumbler(index));
            }
            let actions = row(&mut commands);
            button(&mut commands, actions, &font, Control::Force);
            button(&mut commands, actions, &font, Control::Cast);
            button(&mut commands, actions, &font, Control::UseItem);
            button(&mut commands, actions, &font, Control::Leave);
        }
        Kind::Trap => {
            let guess = row(&mut commands);
            button(&mut commands, guess, &font, Control::PreviousTrap);
            caption(&mut commands, guess, &font, Caption::Guess);
            button(&mut commands, guess, &font, Control::NextTrap);
            let components = row(&mut commands);
            for index in 0..COMPONENTS {
                button(&mut commands, components, &font, Control::Component(index));
            }
            let actions = row(&mut commands);
            button(&mut commands, actions, &font, Control::Inspect);
            button(&mut commands, actions, &font, Control::Cast);
            button(&mut commands, actions, &font, Control::UseItem);
            button(&mut commands, actions, &font, Control::Spring);
            button(&mut commands, actions, &font, Control::Leave);
        }
        Kind::Loot => {
            commands.spawn((
                LootRows::default(),
                Node { flex_direction: FlexDirection::Column, row_gap: px(2), ..default() },
                ChildOf(body),
            ));
            let actions = row(&mut commands);
            button(&mut commands, actions, &font, Control::All);
            button(&mut commands, actions, &font, Control::TakeToParty);
            button(&mut commands, actions, &font, Control::TakeToMember);
            button(&mut commands, actions, &font, Control::Leave);
        }
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

fn button(commands: &mut Commands, parent: Entity, font: &Handle<SpriteSheet>, control: Control) {
    let button = commands
        .spawn((
            Button,
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
}

/// Keep one row per stack in the container, dropping choices of stacks
/// that have gone.
fn sync_loot_rows(
    mut commands: Commands,
    mut panels: Query<&mut DevicePanel>,
    mut lists: Query<(Entity, &mut LootRows, &ChildOf)>,
    parents: Query<&ChildOf>,
    containers: Query<&Container>,
    art: Res<DevicePanelArt>,
) {
    for (list, mut rows, parent) in &mut lists {
        let Ok(root) = parents.get(parent.parent()) else { continue };
        let Ok(mut panel) = panels.get_mut(root.parent()) else { continue };
        let Ok(container) = containers.get(panel.device) else { continue };
        let stacks: Vec<StackId> = container.contents.items.iter().map(|s| s.id).collect();
        if rows.0 == stacks {
            continue;
        }
        panel.chosen.retain(|(id, _)| stacks.contains(id));
        commands.entity(list).despawn_related::<Children>();
        for &id in &stacks {
            let line =
                commands.spawn((Node { column_gap: px(4), ..default() }, ChildOf(list))).id();
            button(&mut commands, line, &art.font, Control::Less(id));
            button(&mut commands, line, &art.font, Control::More(id));
            button(&mut commands, line, &art.font, Control::Row(id));
        }
        rows.0 = stacks;
    }
}

/// Bring every caption and button up to date with the interaction, the
/// selected member and the drafts.
fn draw_device_panel(
    difficulty: Res<crate::game::difficulty::Difficulty>,
    mut commands: Commands,
    interaction: Option<Res<DeviceInteraction>>,
    panels: Query<&DevicePanel>,
    selected: Res<SelectedMember>,
    roster: Single<&PartyRoster>,
    members: Query<(&Character, &Vitals)>,
    containers: Query<&Container>,
    data: Option<Res<GameData>>,
    catalogs: Res<Assets<ItemCatalog>>,
    art: Res<DevicePanelArt>,
    tables: Res<Assets<TextTable>>,
    mut captions: Query<(&Caption, &mut BitmapText)>,
    mut buttons: Query<(Entity, &Control, &mut BackgroundColor, Has<InteractionDisabled>)>,
) {
    let (Some(interaction), Ok(panel)) = (interaction, panels.single()) else { return };
    let member = selected.member(&roster).and_then(|m| Some((m, members.get(m).ok()?)));
    let name = member.map_or(String::new(), |(_, (c, _))| c.name.clone());
    let skill = member.and_then(|(_, (c, v))| interaction::lock_skill(c, v));
    let strings = tables.get(&art.strings);
    let trap_name = |trap: usize| {
        strings.and_then(|s| s.get(TRAP_NAMES + trap)).unwrap_or("Trap").trim().to_owned()
    };
    let catalog = data.as_ref().and_then(|d| catalogs.get(&d.items));
    let stacks = containers.get(interaction.device).map_or(&[][..], |c| &c.contents.items[..]);
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
                        .and_then(|c| c.get(stack.item))
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
    mut panels: Query<&mut DevicePanel>,
    containers: Query<&Container>,
    selected: Res<SelectedMember>,
    roster: Single<&PartyRoster>,
    mut commands: Commands,
) {
    let (Ok(&control), Ok(mut panel)) = (controls.get(click.entity), panels.single_mut()) else {
        return;
    };
    let actor = selected.member(&roster);
    let count = |id: StackId| {
        containers
            .get(panel.device)
            .ok()
            .and_then(|c| c.contents.items.iter().find(|s| s.id == id))
            .map_or(0, |s| s.count())
    };
    let attempt = move |run: fn(&mut World, Entity) -> Result<(), interaction::Unable>| {
        move |world: &mut World| {
            if let Some(actor) = actor
                && let Err(unable) = run(world, actor)
            {
                debug!("device attempt refused: {unable:?}");
            }
        }
    };
    match control {
        Control::Cast | Control::UseItem => {
            if let (Some(actor), Some(art)) = (actor, magic_art.as_deref()) {
                let device = panel.device;
                super::main_screen::magic_view::open(
                    &mut commands,
                    art,
                    actor,
                    control == Control::UseItem,
                    Some(device),
                );
            }
        }
        Control::Tumbler(index) => {
            if let Some(actor) = actor {
                commands.queue(move |world: &mut World| {
                    let _ = interaction::pick_tumbler(world, actor, index);
                });
            }
        }
        Control::Force => commands.queue(attempt(interaction::force_lock)),
        Control::Inspect => commands.queue(attempt(interaction::inspect_trap)),
        Control::Component(index) => {
            if let Some(actor) = actor {
                commands.queue(move |world: &mut World| {
                    let _ = interaction::disarm_component(world, actor, index);
                });
            }
        }
        Control::Spring => commands.queue(move |world: &mut World| {
            let _ = interaction::spring_trap(world, actor);
        }),
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
                .get(panel.device)
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
            commands.queue(move |world: &mut World| {
                if let Err(refusal) = containers::take_from_container(world, &picks, to) {
                    debug!("taking from the container refused: {refusal:?}");
                }
            });
        }
        Control::Leave => commands.queue(interaction::leave),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::characters::Skill;
    use crate::content::level::Lock;
    use crate::game::characters::{Gender, Profession, Race};
    use crate::game::world::interaction::LockNews;

    /// The panel follows the interaction and submits the selected member's
    /// picks; the game, not the panel, decides when the lock opens.
    #[test]
    fn the_lock_panel_picks_for_the_selected_member_and_closes_with_the_lock() {
        let mut app = crate::presentation::test_input::input_app();
        app.add_plugins(plugin)
            .init_asset::<SpriteSheet>()
            .init_asset::<TextTable>()
            .init_asset::<ItemCatalog>()
            .init_resource::<SelectedMember>()
            .init_resource::<crate::game::difficulty::Difficulty>()
            .insert_resource(crate::game::rng::GameRng::seeded(1))
            .add_message::<LockNews>();
        let mut profile = Character::new("A", Race::Human, Gender::Male, Profession::Rogue);
        profile.skill_points[Skill::LocksAndTraps] = 100;
        let member = app.world_mut().spawn((Vitals::new(&profile), profile)).id();
        crate::game::party::roster::test_party(app.world_mut(), &[member]);
        app.world_mut().resource_mut::<SelectedMember>().0 = Some(member);
        let lock = Lock::Pickable { difficulty: 2 };
        let device = app.world_mut().spawn(interaction::Guarded::new(lock)).id();
        app.finish();
        app.cleanup();
        interaction::confront(app.world_mut(), device, lock);
        app.update();
        app.update();
        let tumblers: Vec<(Entity, usize)> = app
            .world_mut()
            .query::<(Entity, &Control)>()
            .iter(app.world())
            .filter_map(|(e, c)| match c {
                Control::Tumbler(i) => Some((e, *i)),
                _ => None,
            })
            .collect();
        assert_eq!(tumblers.len(), 2);
        app.world_mut().trigger(Activate { entity: tumblers[0].0 });
        app.update();
        let Some(Work::Picking(picking)) =
            app.world().get_resource::<DeviceInteraction>().map(|i| &i.work)
        else {
            panic!("still picking")
        };
        assert_eq!(
            picking.tumblers.iter().filter(|t| **t == interaction::Tumbler::Held(member)).count(),
            1
        );
        app.world_mut().trigger(Activate { entity: tumblers[1].0 });
        app.update();
        app.update();
        assert!(!app.world().contains_resource::<DeviceInteraction>());
        assert!(app.world().get::<interaction::Guarded>(device).is_none());
        assert!(app.world_mut().query::<&DevicePanel>().iter(app.world()).next().is_none());
    }
}
