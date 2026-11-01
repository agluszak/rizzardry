//! Confirm consequential choices in the character creator.
use super::{Creation, Page, Session};
use crate::content::ui::SpriteSheet;
use crate::presentation::retail_controls::ControlSkin;
use crate::presentation::screen_art::{BitmapText, Picture, spawn_dialog_frame};
use bevy::input_focus::{
    FocusCause, InputFocus,
    tab_navigation::{TabGroup, TabIndex},
};
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button, ModalDialog};
use bevy_enhanced_input::prelude::*;

enum Purpose {
    Delete { candidate: u32, name: String },
    KeepSpells,
}

#[derive(Component)]
pub(super) struct ConfirmDialog {
    purpose: Purpose,
}
#[derive(Component)]
pub(super) struct ConfirmationButton(usize);
/// The dialog's panel, with the font of its question.
#[derive(Component)]
pub(super) struct Panel(Handle<SpriteSheet>);
/// On a panel whose question has been laid out.
#[derive(Component)]
pub(super) struct Measured;
#[derive(Event)]
pub(super) struct Answer {
    dialog: Entity,
    accepted: bool,
}
#[derive(InputAction)]
#[action_output(bool)]
struct Accept;
#[derive(InputAction)]
#[action_output(bool)]
struct Cancel;

pub(super) fn open(
    commands: &mut Commands,
    assets: &AssetServer,
    candidate: u32,
    name: &str,
    focus: &mut InputFocus,
) {
    open_dialog(commands, assets, Purpose::Delete { candidate, name: name.to_owned() }, focus);
}

pub(super) fn confirm_remaining_spells(
    commands: &mut Commands,
    assets: &AssetServer,
    focus: &mut InputFocus,
) {
    open_dialog(commands, assets, Purpose::KeepSpells, focus);
}

fn open_dialog(
    commands: &mut Commands,
    assets: &AssetServer,
    purpose: Purpose,
    focus: &mut InputFocus,
) {
    let restore = crate::presentation::retail_controls::RestoreFocus(focus.get());
    let dialog = commands.spawn_scene(bsn! {
        Node { width: percent(100), height: percent(100), justify_content: JustifyContent::Center, align_items: AlignItems::Center }
        GlobalZIndex(100)
        Pickable::default()
        TabGroup::modal()
        ModalDialog
        ContextPriority::<ConfirmDialog>::new(10)
        ContextActivity::<ConfirmDialog>::INACTIVE
        Actions<ConfirmDialog> [
            Action::<Accept>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [ Binding(KeyCode::Enter) ]
            -- Action::<Cancel>::new() ActionSettings { consume_input: true, require_reset: true } Bindings [ Binding(KeyCode::Escape) ]
        ]
        on(|event: On<Start<Accept>>, mut commands: Commands| { commands.trigger(Answer { dialog: event.context, accepted: true }); })
        on(|event: On<Start<Cancel>>, mut commands: Commands| { commands.trigger(Answer { dialog: event.context, accepted: false }); })
    }).insert((ConfirmDialog { purpose }, restore, DespawnOnExit(Session::Creation))).id();
    let panel = commands
        .spawn((
            Panel(assets.load("retail://data/fonts/wiz_text_font.sti")),
            Node { width: px(210), height: px(100), overflow: Overflow::clip(), ..default() },
            Pickable::IGNORE,
            ChildOf(dialog),
        ))
        .id();
    spawn_dialog_frame(commands, assets, panel, true);
    let sheet: Handle<SpriteSheet> = assets.load("retail://data/dialogs/dialogconfirmation.sti");
    for (x, frame, accepted) in [(68., 0, true), (117., 4, false)] {
        let button = commands
            .spawn((
                Button,
                TabIndex(0),
                ConfirmationButton(frame),
                ControlSkin::new(frame, frame + 1, frame + 2, frame + 2, frame + 3),
                Picture { sheet: sheet.clone(), frame, at: Vec2::new(x, 61.), bounds: Some(frame) },
                Pickable::default(),
                ChildOf(panel),
            ))
            .observe(move |_: On<Activate>, mut commands: Commands| {
                commands.trigger(Answer { dialog, accepted });
            })
            .id();
        if accepted {
            focus.set(button, FocusCause::Navigated);
        }
    }
}

pub(super) fn resolve(
    answer: On<Answer>,
    dialogs: Query<(&ConfirmDialog, &ContextActivity<ConfirmDialog>)>,
    mut sessions: Query<&mut Creation>,
    mut commands: Commands,
) {
    let Ok((dialog, active)) = dialogs.get(answer.dialog) else { return };
    if !**active {
        return;
    }
    if answer.accepted
        && let Ok(mut session) = sessions.single_mut()
    {
        match &dialog.purpose {
            Purpose::Delete { candidate, .. } => {
                if let Some(index) = session.candidates.iter().position(|(id, _)| id == candidate) {
                    session.candidates.remove(index);
                    session.party.retain(|id| id != candidate);
                    session.selected = session.candidates.get(index.max(1) - 1).map(|(id, _)| *id);
                }
            }
            Purpose::KeepSpells => {
                if session.mode.page() == Page::Magic
                    && session
                        .mode
                        .draft()
                        .is_some_and(|draft| draft.spell_remaining() < draft.spell_points_total())
                {
                    session.mode.set_page(Page::Skills);
                }
            }
        }
    }
    commands.entity(answer.dialog).despawn();
}

/// Word the question once its font and strings are there, each line centred
/// on its own, and size the panel around it.
pub(super) fn show_text(
    mut commands: Commands,
    game_strings: crate::presentation::screen_art::GameStrings,
    sheets: Res<Assets<SpriteSheet>>,
    dialogs: Query<&ConfirmDialog>,
    mut panels: Query<
        (Entity, &Panel, &ChildOf, &mut Node),
        (Without<Measured>, Without<ConfirmationButton>),
    >,
    mut buttons: Query<(&ConfirmationButton, &ChildOf, &mut Picture), Without<Panel>>,
) {
    for (entity, Panel(font), dialog, mut node) in &mut panels {
        let (Some(table), Some(sheet)) = (game_strings.table(), sheets.get(font)) else { continue };
        let message = match &dialogs.get(dialog.parent()).unwrap().purpose {
            Purpose::Delete { name, .. } => format!(
                "{} {} {}?",
                table.get(0x6d4).unwrap_or("Erase"),
                name,
                table.get(0x6d5).unwrap_or("Permanently")
            ),
            Purpose::KeepSpells => table
                .get(0xc4)
                .unwrap_or("You still have some spell picks. Save them and proceed?")
                .to_owned(),
        };
        let lines = crate::presentation::notices::wrap(&message, 220, |l| sheet.text_width(l));
        let width = lines.iter().map(|line| sheet.text_width(line)).max().unwrap_or(0) as f32;
        let width = (width * 1.25).max(170.).floor();
        let line_height = sheet.line_height();
        let text_height = line_height * lines.len() as f32;
        let height = (text_height + 63.).max(text_height * 7.);
        let top = ((height - 33. - text_height) / 2.).floor();
        for (line, value) in lines.into_iter().enumerate() {
            let at = Vec2::new(0., top + line as f32 * line_height);
            let text = BitmapText { value, ..BitmapText::centred(font.clone(), at, width) };
            commands.spawn((text, ChildOf(entity)));
        }
        node.width = px(width);
        node.height = px(height);
        for (button, panel, mut picture) in &mut buttons {
            if panel.parent() == entity {
                let left = if button.0 == 0 {
                    ((width - 75.) / 2.).floor()
                } else {
                    ((width + 25.) / 2.).floor()
                };
                picture.at = Vec2::new(left, height - 24. - 15.);
            }
        }
        commands.entity(entity).insert(Measured);
    }
}
