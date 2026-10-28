//! The main screen's keys, from retail's `mgskeyboard.ini`: Backspace
//! cycles the layout (Shift+Backspace backwards), 1-6 select a member,
//! Enter starts a combat round, backslash switches continuous combat, A and
//! D set the selected member to attack or defend, and PgUp, PgDn, Home, End
//! and Del scroll the text box a page, to its top or its end, or empty it.
//! They act only while no other screen is up and no widget owns focus.
//! F6 enters the HUD; Tab navigates its controls and Escape returns to the world.

use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;

use super::{ScreenLayout, SelectedMember};
use crate::game::combat::{Battle, StartRound};
use crate::game::party::Order;
use crate::game::party::roster::PartyRoster;
use crate::game::screen::Screen;
use crate::presentation::notices::{TextLog, VISIBLE_LINES};

#[derive(Component, Clone, Default)]
pub struct MainScreenKeys;

#[derive(InputAction)]
#[action_output(bool)]
struct FocusHud;

#[derive(InputAction)]
#[action_output(bool)]
struct CycleLayout;

#[derive(InputAction)]
#[action_output(bool)]
struct CycleLayoutBack;

/// Select the member in [`MemberKey`]'s slot.
#[derive(InputAction)]
#[action_output(bool)]
struct SelectMember;

/// The party slot a [`SelectMember`] action selects.
#[derive(Component, Clone, Default)]
struct MemberKey(usize);

#[derive(InputAction)]
#[action_output(bool)]
struct BeginRound;

#[derive(InputAction)]
#[action_output(bool)]
struct ToggleContinuous;

/// Set the selected member's order in a fight.
#[derive(InputAction)]
#[action_output(bool)]
struct SetOrder;

#[derive(Component, Clone, Default)]
struct OrderKey(Order);

/// Scroll the text box, or empty it.
#[derive(InputAction)]
#[action_output(bool)]
struct TextBoxKey;

#[derive(Clone, Copy, Default)]
enum TextBoxCommand {
    #[default]
    PageUp,
    PageDown,
    Top,
    End,
    Clear,
}

#[derive(Component, Clone, Default)]
struct TextBoxKeyCommand(TextBoxCommand);

/// The keys, held each frame once focus has been dispatched and before
/// their bindings are read.
pub(super) fn plugin(app: &mut App) {
    app.add_input_context::<MainScreenKeys>()
        .add_systems(Startup, spawn_main_screen_keys)
        .add_systems(
            PreUpdate,
            hold_main_screen_keys
                .after(bevy::input_focus::InputFocusSystems::Dispatch)
                .before(EnhancedInputSystems::Update),
        );
}

fn spawn_main_screen_keys(mut commands: Commands) {
    let members: Vec<_> = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
    ]
    .into_iter()
    .enumerate()
    .map(|(slot, key)| bsn! { Action::<SelectMember>::new() MemberKey({slot}) Bindings [ Binding(key) ] })
    .collect();
    let text_box: Vec<_> = [
        (KeyCode::PageUp, TextBoxCommand::PageUp),
        (KeyCode::PageDown, TextBoxCommand::PageDown),
        (KeyCode::Home, TextBoxCommand::Top),
        (KeyCode::End, TextBoxCommand::End),
        (KeyCode::Delete, TextBoxCommand::Clear),
    ]
    .into_iter()
    .map(|(key, command)| {
        bsn! { Action::<TextBoxKey>::new() TextBoxKeyCommand({command}) Bindings [ Binding(key) ] }
    })
    .collect();
    let shift_backspace = Binding::Keyboard { key: KeyCode::Backspace, mod_keys: ModKeys::SHIFT };
    commands.spawn_scene(bsn! {
        MainScreenKeys
        Actions<MainScreenKeys> [
            Action::<FocusHud>::new() Bindings [ Binding(KeyCode::F6) ]
            --
            Action::<CycleLayoutBack>::new()
            ActionSettings { consume_input: true }
            Bindings [ shift_backspace ]
            --
            Action::<CycleLayout>::new()
            Bindings [ Binding(KeyCode::Backspace) ]
            --
            {members}
            --
            Action::<BeginRound>::new()
            Bindings [ Binding(KeyCode::Enter) ]
            --
            Action::<ToggleContinuous>::new()
            Bindings [ Binding(KeyCode::Backslash) ]
            --
            Action::<SetOrder>::new() OrderKey({Order::Attack}) Bindings [ Binding(KeyCode::KeyA) ]
            --
            Action::<SetOrder>::new() OrderKey({Order::Defend}) Bindings [ Binding(KeyCode::KeyD) ]
            --
            {text_box}
        ]
        on(|_: On<Start<CycleLayout>>, mut layout: ResMut<ScreenLayout>| layout.cycle(false))
        on(|_: On<Start<CycleLayoutBack>>, mut layout: ResMut<ScreenLayout>| layout.cycle(true))
        on(|_: On<Start<FocusHud>>, roots: Query<Entity,With<super::MainScreen>>,
            navigation: bevy::input_focus::tab_navigation::TabNavigation,
            mut focus: ResMut<bevy::input_focus::InputFocus>,
            mut visible: ResMut<bevy::input_focus::InputFocusVisible>| {
            if let Ok(root)=roots.single() && let Ok(target)=navigation.initialize(root,bevy::input_focus::tab_navigation::NavAction::First) {
                focus.set(target,bevy::input_focus::FocusCause::Navigated);visible.0=true;
            }
        })
        on(select_member)
        on(|_: On<Start<BeginRound>>, battle: Option<Single<&Battle>>, mut round: MessageWriter<StartRound>| {
            if battle.is_some() {
                round.write(StartRound);
            }
        })
        on(|_: On<Start<ToggleContinuous>>, mut controls: MessageWriter<crate::game::combat::CombatControl>| {
            controls.write(crate::game::combat::CombatControl::ToggleContinuous);
        })
        on(set_order)
        on(text_box_key)
    });
}

/// Whether the world's shortcut keys may act: the party is in the world,
/// under no question, and no focused control owns the keyboard. Every world
/// shortcut context is held by this one rule.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct WorldKeys<'w, 's> {
    screen: Res<'w, State<Screen>>,
    focus: Res<'w, bevy::input_focus::InputFocus>,
    controls: Query<'w, 's, (), With<bevy::input_focus::tab_navigation::TabIndex>>,
    questions: Query<'w, 's, (), With<crate::game::world::level::TravelQuestion>>,
    conversations: Query<'w, 's, (), With<crate::game::npcs::conversation::Conversation>>,
}

impl WorldKeys<'_, '_> {
    pub(crate) fn active(&self) -> bool {
        *self.screen.get() == Screen::World
            && self.questions.is_empty()
            && self.conversations.is_empty()
            && !self.focus.get().is_some_and(|entity| self.controls.contains(entity))
    }
}

/// The keys act while the party is in the world, under no other screen.
fn hold_main_screen_keys(
    mut commands: Commands,
    world_keys: WorldKeys,
    keys: Query<(Entity, &ContextActivity<MainScreenKeys>)>,
) {
    let active = world_keys.active();
    for (entity, activity) in &keys {
        if **activity != active {
            commands.entity(entity).insert(ContextActivity::<MainScreenKeys>::new(active));
        }
    }
}

/// Retail binds 1-6 to the party's slots.
fn select_member(
    key: On<Start<SelectMember>>,
    slots: Query<&MemberKey>,
    roster: Single<&PartyRoster>,
    mut selected: ResMut<SelectedMember>,
) {
    if let Ok(MemberKey(slot)) = slots.get(key.action)
        && *slot < roster.len()
    {
        selected.0 = roster.get_index(*slot).copied();
    }
}

fn set_order(
    key: On<Start<SetOrder>>,
    orders_by_key: Query<&OrderKey>,
    selected: Res<SelectedMember>,
    mut orders: MessageWriter<crate::game::combat::GiveOrder>,
    roster: Single<&PartyRoster>,
) {
    if let Ok(&OrderKey(order)) = orders_by_key.get(key.action)
        && let Some(member) = selected.member(&roster)
    {
        orders.write(crate::game::combat::GiveOrder { member, order });
    }
}

fn text_box_key(
    key: On<Start<TextBoxKey>>,
    commands: Query<&TextBoxKeyCommand>,
    mut log: ResMut<TextLog>,
) {
    let Ok(TextBoxKeyCommand(command)) = commands.get(key.action) else { return };
    match command {
        TextBoxCommand::PageUp => log.scroll_up(VISIBLE_LINES),
        TextBoxCommand::PageDown => log.scroll_down(VISIBLE_LINES),
        TextBoxCommand::Top => log.scroll_to(0),
        TextBoxCommand::End => log.scroll_to_end(),
        TextBoxCommand::Clear => log.clear(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::main_screen::LayoutMode;

    fn app() -> App {
        let mut app = crate::presentation::test_input::input_app();
        app.init_resource::<ScreenLayout>()
            .init_resource::<SelectedMember>()
            .init_state::<Screen>()
            .add_plugins(plugin);
        app.finish();
        app.cleanup();
        app.update();
        app
    }

    fn tap(app: &mut App, keys: &[KeyCode]) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.iter().for_each(|&key| input.press(key));
        app.update();
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.iter().for_each(|&key| input.release(key));
        app.update();
    }

    #[test]
    fn backspace_cycles_the_layout_and_shift_cycles_it_back() {
        let mut app = app();
        tap(&mut app, &[KeyCode::Backspace]);
        assert_eq!(app.world().resource::<ScreenLayout>().mode, LayoutMode::Full);
        tap(&mut app, &[KeyCode::ShiftLeft, KeyCode::Backspace]);
        assert_eq!(app.world().resource::<ScreenLayout>().mode, LayoutMode::Wide);
    }

    #[test]
    fn number_keys_select_members_only_while_no_other_screen_is_up() {
        let mut app = app();
        let members: Vec<Entity> = (0..3).map(|_| app.world_mut().spawn_empty().id()).collect();
        let party = crate::game::party::roster::test_party(app.world_mut(), &members);
        tap(&mut app, &[KeyCode::Digit3]);
        assert_eq!(app.world().resource::<SelectedMember>().0, Some(members[2]));
        let reversed: Vec<_> = members.iter().rev().copied().collect();
        crate::game::party::roster::test_reorder(app.world_mut(), &reversed);
        let roster = app.world().get::<PartyRoster>(party).unwrap();
        assert_eq!(
            app.world().resource::<SelectedMember>().member(roster),
            Some(members[2]),
            "selection follows the actor after reordering"
        );
        tap(&mut app, &[KeyCode::Digit6]);
        assert_eq!(
            app.world().resource::<SelectedMember>().0,
            Some(members[2]),
            "nobody in slot 6"
        );
        app.world_mut().resource_mut::<NextState<Screen>>().set(Screen::Inventory);
        app.update();
        tap(&mut app, &[KeyCode::Digit1]);
        assert_eq!(
            app.world().resource::<SelectedMember>().0,
            Some(members[2]),
            "the inventory is up"
        );
    }
}
