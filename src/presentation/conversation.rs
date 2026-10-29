//! Native dialogue widgets. All selected rows and quotes are drafts; gameplay
//! resolves item/stock identities again when a confirmation executes.
mod quantity;
use super::main_screen::SelectedMember;
use super::retail_controls::{ControlSkin, FocusRing};
use super::screen_art::{BitmapInk, BitmapText, BitmapWrap, Picture, absolute};
use crate::content::npcs::TopicCategory;
use crate::content::{items::EquipClass, npcs::NpcCatalog, ui::SpriteSheet};
use crate::game::npcs::{
    Npc, NpcAssets,
    conversation::*,
    trade::{self, TradeQuote, TradeSource},
};
use crate::game::{
    inventory::{Carried, ItemOwner, ItemRef, PartyInventory},
    party::roster::PartyRoster,
};
use bevy::ui::{InteractionDisabled, ScrollPosition};
use bevy::{
    input::keyboard::{Key, KeyboardInput},
    input_focus::{
        FocusCause, FocusedInput, InputFocus,
        tab_navigation::{TabGroup, TabIndex},
    },
    prelude::*,
    text::EditableText,
    ui_widgets::{Activate, Button, ScrollArea, TextInput},
};
#[derive(Component)]
#[relationship(relationship_target = ConversationViews)]
struct ConversationView(pub Entity);
#[derive(Component)]
#[relationship_target(relationship = ConversationView, linked_spawn)]
struct ConversationViews(Vec<Entity>);
#[derive(Component, Default)]
struct DialogueState {
    selected: Option<TradeQuote>,
    error: Option<String>,
    mode: Mode,
    pool: bool,
    category: Option<TopicCategory>,
    alphabetical: bool,
    item_group: Option<ItemGroup>,
    usability: Option<bool>,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Talk,
    Buy,
    Sell,
    /// Hand the NPC an item from a member's pack or the party's items.
    Give,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ItemGroup {
    Hand,
    Body,
    Accessory,
    Other,
}
impl ItemGroup {
    fn of(class: EquipClass) -> Self {
        match class {
            EquipClass::ShortWeapon
            | EquipClass::ExtendedWeapon
            | EquipClass::ThrownWeapon
            | EquipClass::RangedWeapon
            | EquipClass::Ammunition
            | EquipClass::Shield
            | EquipClass::Dagger
            | EquipClass::LongSword
            | EquipClass::Bipennis
            | EquipClass::BattleAxe
            | EquipClass::Flail
            | EquipClass::Mace => Self::Hand,
            EquipClass::Head
            | EquipClass::Torso
            | EquipClass::Hands
            | EquipClass::Legs
            | EquipClass::Feet => Self::Body,
            EquipClass::Misc | EquipClass::Cloak => Self::Accessory,
            _ => Self::Other,
        }
    }
}
#[derive(Component, Clone, Copy)]
enum ItemFilter {
    Group(ItemGroup),
    Usable(bool),
}
#[derive(Component)]
struct ModePanel(bool);
/// The topic menu of an NPC not yet friendly, shown instead of the talk panel.
#[derive(Component)]
struct MenuPanel;
#[derive(Component)]
struct ApproachButton(Approach);
#[derive(Component)]
struct ModeFocus(bool);
#[derive(Component)]
struct BubbleEdge;
#[derive(Component)]
struct TopicChrome;
#[derive(Component)]
enum Caption {
    Name,
    Gold,
    Quote,
    TradeAction,
}
#[derive(Component)]
struct TopicList;
#[derive(Component)]
struct TopicRow(String);
#[derive(Component)]
struct QuoteBubble {
    line: Option<(Entity, Entity)>,
}
#[derive(Component)]
struct ItemPreview;
#[derive(Component)]
struct Confirm;
#[derive(Component)]
struct QuantityControl;
#[derive(Component)]
struct TradePrice;
#[derive(Component)]
struct TradeCount;
#[derive(Component)]
struct ModeButton(Mode);
#[derive(Component)]
struct PoolButton(bool);
#[derive(Component)]
struct CategoryButton(Option<TopicCategory>);
#[derive(Component)]
struct TradeAvailable;

#[derive(Component)]
struct RadioMark;
#[derive(Component)]
struct SortToggle;
/// A text button's authored chrome is separate from its hit rectangle.
#[derive(Component)]
struct WellChrome {
    size: Vec2,
    hover: usize,
    pressed: usize,
    pressed_hover: usize,
}

#[derive(Component)]
struct Summary;
#[derive(Component)]
struct TranscriptRow(usize);
#[derive(Component)]
struct ScrollToNewest;
#[derive(Component)]
struct Offer;
#[derive(Component, PartialEq)]
struct Choices {
    font: Handle<SpriteSheet>,
}
#[derive(Component, Default)]
struct ChoiceOverlay {
    previous_focus: Option<Entity>,
}
#[derive(Component)]
struct ChoiceCaption(bool);
#[derive(Clone, PartialEq)]
enum Answer {
    Reply(String),
    Price(bool),
}
#[derive(Component, PartialEq)]
struct Choice {
    session: Entity,
    index: usize,
    revision: u64,
    answer: Answer,
}
#[derive(Component)]
pub(crate) struct TopicInput;
#[derive(Component)]
struct SubmitTopic;
#[derive(Component)]
struct TradeList;
#[derive(Component, Clone, Copy, PartialEq)]
struct TradeRow {
    session: Entity,
    source: TradeSource,
    quantity: u32,
}
#[derive(Component)]
struct PlaybackStarted;

pub struct ConversationViewPlugin;
impl Plugin for ConversationViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(activate_choice)
            .add_observer(activate_trade)
            .add_observer(activate_topic)
            .add_systems(
                Update,
                (
                    close_views,
                    create_views,
                    (
                        project_text,
                        project_modes,
                        sync_choices,
                        sync_trade_rows,
                        sync_topic_rows,
                        sync_transcript,
                    ),
                    (sync_bubble_geometry, sync_topic_geometry, route_mode_focus, paint_wells),
                )
                    .chain()
                    .after(crate::game::npcs::conversation::update_conversation),
            )
            .add_systems(
                Update,
                (play_lines, watch_playback)
                    .chain()
                    .after(crate::game::npcs::conversation::update_conversation),
            )
            .add_systems(
                PostUpdate,
                (
                    submit_topic.after(bevy::text::EditableTextSystems),
                    colour_selections,
                    paint_topic_text.before(super::screen_art::render_text),
                    scroll_to_newest.after(bevy::ui::UiSystems::Layout),
                    quantity::sync.after(bevy::text::EditableTextSystems),
                ),
            );
    }
}
fn bitmap(world: &mut World, parent: Entity, value: String, width: f32) -> Entity {
    let font = world
        .resource::<AssetServer>()
        .load::<SpriteSheet>("retail://data/fonts/wiz_text_font.sti");
    world
        .spawn((
            BitmapText { font, value, width, ..default() },
            Node { position_type: PositionType::Relative, flex_shrink: 0.0, ..default() },
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}
fn label(world: &mut World, parent: Entity, text: &str, at: Vec2, width: f32) -> Entity {
    let e = bitmap(world, parent, text.into(), width);
    world.get_mut::<BitmapText>(e).unwrap().at = at;
    world.get_mut::<Node>(e).unwrap().position_type = PositionType::Absolute;
    e
}
fn picture(world: &mut World, parent: Entity, path: &str, frame: usize, at: Vec2) -> Entity {
    let sheet =
        world.resource::<AssetServer>().load(format!("retail://data/npc interaction/{path}.sti"));
    world.spawn((Picture { sheet, frame, at, bounds: None }, ChildOf(parent))).id()
}
fn panel(world: &mut World, parent: Entity, at: Vec2, size: Vec2, frame: usize) -> Entity {
    let e = world.spawn((absolute(at, size), BackgroundColor(Color::BLACK), ChildOf(parent))).id();
    picture(world, e, "npc_bottompanel", frame, Vec2::ZERO);
    e
}
fn button(world: &mut World, parent: Entity, text: &str) -> Entity {
    let e = world
        .spawn((
            Button,
            FocusRing,
            TabIndex(0),
            Node { min_height: px(12), flex_shrink: 0.0, ..default() },
            ChildOf(parent),
        ))
        .id();
    bitmap(world, e, text.into(), 0.0);
    e
}
fn action(world: &mut World, parent: Entity, text: &str, at: Vec2, size: Vec2) -> Entity {
    let e = button(world, parent, text);
    let mut node = absolute(at, size);
    node.justify_content = JustifyContent::Center;
    node.align_items = AlignItems::Center;
    world.entity_mut(e).insert(node);
    if let Some(label) = world.get::<Children>(e).and_then(|c| c.first().copied()) {
        world.entity_mut(label).insert(ZIndex(1));
    }
    let frames = if size == Vec2::new(67., 16.) {
        Some((10, 12, 11))
    } else if size == Vec2::new(123., 16.) {
        Some((14, 15, 15))
    } else {
        None
    };
    if let Some((hover, pressed, pressed_hover)) = frames {
        let child = picture(world, e, "npc_bottompanel", usize::MAX, Vec2::ZERO);
        world.entity_mut(child).insert(WellChrome { size, hover, pressed, pressed_hover });
    }
    // Normal button wells are already in the panel artwork; avoid scaling
    // a pressed-state sprite into an unrelated control's hit rectangle.
    world.entity_mut(e).insert(bevy::picking::hover::Hovered::default());
    e
}
fn scroll_arrows(world: &mut World, parent: Entity, target: Entity, at: Vec2, gap: f32) {
    let sheet =
        world.resource::<AssetServer>().load("retail://data/npc interaction/npc_buttons.sti");
    for (dy, step, first) in [(0., -1., 20), (gap, 1., 25)] {
        world
            .spawn((
                Button,
                FocusRing,
                TabIndex(0),
                Picture {
                    sheet: sheet.clone(),
                    frame: first,
                    at: at + Vec2::Y * dy,
                    bounds: Some(first),
                },
                ControlSkin::new(first, first + 1, first + 4, first + 2, first + 3),
                Pickable::default(),
                ChildOf(parent),
            ))
            .observe(move |_: On<Activate>, mut scrolls: Query<&mut ScrollPosition>| {
                if let Ok(mut scroll) = scrolls.get_mut(target) {
                    scroll.0.y = (scroll.0.y + step * 14.).max(0.);
                }
            });
    }
}
fn add_radio(world: &mut World, button: Entity) {
    if let Some(mut node) = world.get_mut::<Node>(button) {
        node.justify_content = JustifyContent::Start;
        node.padding.left = px(10);
    }
    let image = picture(world, button, "npc_bottompanel", 8, Vec2::new(0., 2.));
    world.entity_mut(image).insert(RadioMark);
}
fn paint_wells(
    sheets: Res<Assets<SpriteSheet>>,
    controls: Query<
        (
            Option<&bevy::picking::hover::Hovered>,
            Has<bevy::ui::Pressed>,
            Has<InteractionDisabled>,
            Has<bevy::ui::Selected>,
        ),
        With<Button>,
    >,
    mut parts: Query<
        (&ChildOf, &mut Picture, Option<&WellChrome>, Has<RadioMark>),
        Or<(With<WellChrome>, With<RadioMark>)>,
    >,
) {
    for (parent, mut picture, well, radio) in &mut parts {
        let Ok((hover, pressed, disabled, selected)) = controls.get(parent.parent()) else {
            continue;
        };
        let hover = hover.is_some_and(|h| h.get());
        let frame = if radio {
            if selected { 9 } else { 8 }
        } else if let Some(well) = well {
            if disabled {
                usize::MAX
            } else if pressed && hover {
                well.pressed_hover
            } else if pressed {
                well.pressed
            } else if hover {
                well.hover
            } else {
                usize::MAX
            }
        } else {
            continue;
        };
        let at = well
            .and_then(|well| {
                sheets.get(&picture.sheet)?.frames.get(frame).map(|f| {
                    ((well.size - Vec2::new(f.width as f32, f.height as f32)) / 2.).floor()
                })
            })
            .unwrap_or(picture.at);
        if picture.frame != frame || picture.at != at {
            picture.frame = frame;
            picture.at = at;
        }
    }
}
/// Every NPC takes items; only merchants buy and sell.
fn open_trade(world: &mut World, button: Entity, root: Entity) {
    world.entity_mut(button).observe(
        move |_: On<Activate>,
              mut states: Query<&mut DialogueState>,
              roots: Query<&ConversationView>,
              sessions: Query<&Conversation>,
              npcs: Query<&Npc>,
              definitions: Res<Assets<NpcCatalog>>,
              assets: Res<NpcAssets>| {
            let merchant = roots
                .get(root)
                .ok()
                .and_then(|view| sessions.get(view.0).ok())
                .and_then(|c| npcs.get(c.actor).ok())
                .and_then(|n| definitions.get(&assets.catalog)?.get(n.definition))
                .is_some_and(|d| d.owns_stock);
            if let Ok(mut state) = states.get_mut(root) {
                set_mode(&mut state, if merchant { Mode::Buy } else { Mode::Give });
            }
        },
    );
}
fn leave_on(world: &mut World, button: Entity, root: Entity) {
    world.entity_mut(button).observe(
        move |_: On<Activate>,
              roots: Query<&ConversationView>,
              mut requests: MessageWriter<ConversationRequest>| {
            if let Ok(view) = roots.get(root) {
                requests.write(ConversationRequest::Close { conversation: view.0 });
            }
        },
    );
}
/// Retail `ShowNpcDialogueTopicMenu`: an NPC not yet friendly may be
/// charmed, talked to or bribed before it talks of anything else.
fn build_menu(world: &mut World, canvas: Entity, root: Entity) {
    let group = world.spawn((Node::default(), MenuPanel, Pickable::IGNORE, ChildOf(canvas))).id();
    let left = panel(world, group, Vec2::new(23., 358.), Vec2::new(141., 92.), 0);
    let name = label(world, left, "", Vec2::new(5., 2.), 132.);
    let bold = world.resource::<AssetServer>().load("retail://data/fonts/wiz_text_font_bold.sti");
    let mut text = world.get_mut::<BitmapText>(name).unwrap();
    text.justify = bevy::text::Justify::Center;
    text.font = bold;
    world.entity_mut(name).insert(Caption::Name);
    let strings = world
        .get_resource::<super::screen_art::StringTable>()
        .and_then(|t| world.resource::<Assets<crate::content::ui::TextTable>>().get(&t.0));
    let labels = [
        (0x727, "Charm"),
        (0x728, "Small Talk"),
        (0x729, "Bribe"),
        (0x723, "Attack"),
        (0x724, "Trade"),
        (0x725, "Leave"),
    ]
    .map(|(id, fallback)| strings.and_then(|s| s.get(id)).unwrap_or(fallback).to_owned());
    for (i, text) in labels.into_iter().enumerate() {
        let e = action(
            world,
            left,
            &text,
            Vec2::new(2. + 66. * (i % 2) as f32, 17. + 16. * (i / 2) as f32),
            Vec2::new(67., 16.),
        );
        match i {
            0 | 1 => {
                let approach = if i == 0 { Approach::Charm } else { Approach::SmallTalk };
                world.entity_mut(e).insert(ApproachButton(approach)).observe(
                    move |_: On<Activate>,
                          roots: Query<&ConversationView>,
                          sessions: Query<&Conversation>,
                          mut requests: MessageWriter<ConversationRequest>| {
                        if let Ok(view) = roots.get(root)
                            && let Ok(c) = sessions.get(view.0)
                        {
                            requests.write(ConversationRequest::Approach {
                                conversation: view.0,
                                revision: c.revision,
                                approach,
                            });
                        }
                    },
                );
            }
            // Retail's bribe opens the give layout.
            2 => {
                world.entity_mut(e).observe(
                    move |_: On<Activate>, mut states: Query<&mut DialogueState>| {
                        if let Ok(mut state) = states.get_mut(root) {
                            set_mode(&mut state, Mode::Give);
                        }
                    },
                );
            }
            4 => open_trade(world, e, root),
            5 => leave_on(world, e, root),
            _ => {
                world.entity_mut(e).insert(InteractionDisabled);
            }
        }
    }
}
fn set_mode(state: &mut DialogueState, mode: Mode) {
    if state.mode != mode {
        state.mode = mode;
        state.selected = None;
        state.error = None;
    }
}
fn build(world: &mut World, session: Entity) -> Entity {
    // OpenNpcDialoguePanel selects portrait mode before building its controls.
    world.resource_mut::<super::main_screen::ScreenLayout>().mode =
        super::main_screen::LayoutMode::Closed;
    let root = world
        .spawn((
            ConversationView(session),
            DialogueState {
                selected: None,
                error: None,
                mode: Mode::Talk,
                pool: true,
                category: None,
                alphabetical: false,
                item_group: None,
                usability: None,
            },
            Node {
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(40),
            Pickable::IGNORE,
            TabGroup::modal(),
        ))
        .id();
    world.entity_mut(root).observe(
        move |mut event: On<FocusedInput<KeyboardInput>>,
              mut states: Query<(&ConversationView, &mut DialogueState)>,
              mut requests: MessageWriter<ConversationRequest>| {
            if event.input.state.is_pressed() && event.input.logical_key == Key::Escape {
                event.propagate(false);
                let Ok((view, mut state)) = states.get_mut(root) else { return };
                if state.mode != Mode::Talk {
                    set_mode(&mut state, Mode::Talk);
                } else {
                    requests.write(ConversationRequest::Close { conversation: view.0 });
                }
            }
        },
    );
    let canvas = world
        .spawn((
            Node { width: px(640), height: px(480), ..default() },
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .id();
    // Retail CreateNpcDialogueControls: the NPC interface replaces the lower
    // panels, not the world view. Keep both layouts alive as native UI nodes.
    for trade in [false, true] {
        let group = world
            .spawn((Node::default(), ModePanel(trade), Pickable::IGNORE, ChildOf(canvas)))
            .id();
        let left = panel(
            world,
            group,
            Vec2::new(23., 358.),
            Vec2::new(141., 92.),
            if trade { 17 } else { 0 },
        );
        let name = label(world, left, "", Vec2::new(5., 2.), 132.);
        world.get_mut::<BitmapText>(name).unwrap().justify = bevy::text::Justify::Center;
        world.entity_mut(name).insert(Caption::Name);
        let bold =
            world.resource::<AssetServer>().load("retail://data/fonts/wiz_text_font_bold.sti");
        world.get_mut::<BitmapText>(name).unwrap().font = bold;
        let labels = if trade {
            ["Buy", "Sell", "Give", "Steal", "Party Items", "PC Items"]
        } else {
            ["Trade", "Pickpocket", "Magic", "Recruit", "Attack", "Leave"]
        };
        for (i, text) in labels.into_iter().enumerate() {
            let e = action(
                world,
                left,
                text,
                Vec2::new(2. + 66. * (i % 2) as f32, 17. + 16. * (i / 2) as f32),
                Vec2::new(67., 16.),
            );
            match (trade, i) {
                (false, 0) => open_trade(world, e, root),
                (true, 0..=2) => {
                    let mode = [Mode::Buy, Mode::Sell, Mode::Give][i];
                    world.entity_mut(e).insert(ModeButton(mode));
                    if mode != Mode::Give {
                        world.entity_mut(e).insert(TradeAvailable);
                    }
                    world.entity_mut(e).observe(
                        move |_: On<Activate>, mut states: Query<&mut DialogueState>| {
                            if let Ok(mut state) = states.get_mut(root) {
                                set_mode(&mut state, mode);
                            }
                        },
                    );
                }
                (false, 2) => {
                    world.entity_mut(e).observe(move |_: On<Activate>, roots: Query<&ConversationView>, conversations: Query<&crate::game::npcs::conversation::Conversation>,
                        selected: Res<crate::presentation::main_screen::SelectedMember>, roster: Single<&crate::game::party::roster::PartyRoster>,
                        art: Res<crate::presentation::main_screen::magic_view::MagicViewArt>, mut commands: Commands| {
                        if let Ok(view) = roots.get(root) && conversations.get(view.0).is_ok_and(|c| c.ready()) && let Some(caster) = selected.member(&roster) {
                            crate::presentation::main_screen::magic_view::open(&mut commands, &art, caster, false, Some(crate::presentation::main_screen::magic_view::ViewTarget::Conversation(view.0)));
                        }
                    });
                }
                (false, 5) => leave_on(world, e, root),
                (true, 4..=5) => {
                    let pool = i == 4;
                    world.entity_mut(e).insert(PoolButton(pool));
                    world.entity_mut(e).observe(
                        move |_: On<Activate>, mut states: Query<&mut DialogueState>| {
                            if let Ok(mut state) = states.get_mut(root)
                                && state.pool != pool
                            {
                                state.pool = pool;
                                state.selected = None;
                                state.error = None;
                            }
                        },
                    );
                }
                _ => {
                    world.entity_mut(e).insert(InteractionDisabled);
                }
            }
        }
        if trade {
            let sheet = world
                .resource::<AssetServer>()
                .load("retail://data/npc interaction/npc_itemfilters.sti");
            for (i, filter) in [
                ItemFilter::Group(ItemGroup::Hand),
                ItemFilter::Group(ItemGroup::Accessory),
                ItemFilter::Usable(false),
                ItemFilter::Group(ItemGroup::Body),
                ItemFilter::Group(ItemGroup::Other),
                ItemFilter::Usable(true),
            ]
            .into_iter()
            .enumerate()
            {
                let first = i * 5;
                let x = [7., 25., 43., 62., 80., 98.][i];
                world
                    .spawn((
                        Button,
                        FocusRing,
                        TabIndex(0),
                        filter,
                        Pickable::default(),
                        Picture {
                            sheet: sheet.clone(),
                            frame: first,
                            at: Vec2::new(x, 70.),
                            bounds: Some(first),
                        },
                        ControlSkin::new(first, first + 2, first + 1, first + 4, first + 3),
                        ChildOf(left),
                    ))
                    .observe(move |_: On<Activate>, mut states: Query<&mut DialogueState>| {
                        if let Ok(mut state) = states.get_mut(root) {
                            match filter {
                                ItemFilter::Group(group) => {
                                    state.item_group = if state.item_group == Some(group) {
                                        None
                                    } else {
                                        Some(group)
                                    }
                                }
                                ItemFilter::Usable(party) => {
                                    state.usability = if state.usability == Some(party) {
                                        None
                                    } else {
                                        Some(party)
                                    }
                                }
                            }
                            state.selected = None;
                            state.error = None;
                        }
                    });
            }
            let back = action(world, left, "Back", Vec2::new(114., 71.), Vec2::new(23., 16.));
            let children: Vec<_> =
                world.get::<Children>(back).into_iter().flatten().copied().collect();
            for child in children {
                world.despawn(child);
            }
            let sheet = world
                .resource::<AssetServer>()
                .load("retail://data/npc interaction/npc_buttons.sti");
            world.entity_mut(back).insert((
                Picture { sheet, frame: 0, at: Vec2::new(114., 71.), bounds: Some(0) },
                ControlSkin::new(0, 1, 4, 2, 3),
                Pickable::default(),
            ));
            world.entity_mut(back).insert(ModeFocus(true));
            world.entity_mut(back).observe(
                move |_: On<Activate>, mut states: Query<&mut DialogueState>| {
                    if let Ok(mut state) = states.get_mut(root) {
                        set_mode(&mut state, Mode::Talk);
                    }
                },
            );
        }
        let centre = panel(world, group, Vec2::new(164., 358.), Vec2::new(312., 92.), 1);
        let list = world
            .spawn((
                ScrollArea,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(4),
                    top: px(8),
                    width: px(284),
                    height: px(76),
                    flex_direction: FlexDirection::Column,
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
                ChildOf(centre),
            ))
            .id();
        if trade {
            world.entity_mut(list).insert(TradeList);
            scroll_arrows(world, centre, list, Vec2::new(298., 4.), 71.);
        } else {
            world.entity_mut(list).insert(Summary);
            scroll_arrows(world, centre, list, Vec2::new(298., 4.), 71.);
            let overlay = world
                .spawn((
                    ChoiceOverlay::default(),
                    Node { display: Display::None, ..absolute(Vec2::ZERO, Vec2::new(640., 480.)) },
                    GlobalZIndex(100),
                    Pickable::default(),
                    ChildOf(group),
                ))
                .id();
            // Retail option/price dialogs require an answer; Escape must not
            // bubble through and close the conversation underneath them.
            world.entity_mut(overlay).observe(|mut event: On<FocusedInput<KeyboardInput>>| {
                if event.input.logical_key == Key::Escape {
                    event.propagate(false);
                }
            });
            let assets = world.resource::<AssetServer>().clone();
            let choices = world
                .spawn((
                    Choices {
                        font: assets.load("retail://data/fonts/wiz_text_font_monopalette.sti"),
                    },
                    absolute(Vec2::new(220., 184.), Vec2::new(200., 70.)),
                    ChildOf(overlay),
                ))
                .id();
            let assets = world.resource::<AssetServer>().clone();
            super::screen_art::spawn_dialog_frame(&mut world.commands(), &assets, choices, false);
            for purse in [false, true] {
                let text =
                    label(world, choices, "", Vec2::new(10., if purse { 55. } else { 10. }), 180.);
                world.entity_mut(text).insert(ChoiceCaption(purse));
                world.get_mut::<BitmapText>(text).unwrap().justify = bevy::text::Justify::Center;
            }
        }
        if trade {
            let right = panel(world, group, Vec2::new(476., 358.), Vec2::new(141., 92.), 6);
            let offer = label(world, right, "", Vec2::new(4., 4.), 132.);
            world.entity_mut(offer).insert(Offer);
            world.get_mut::<BitmapText>(offer).unwrap().justify = bevy::text::Justify::Center;
            world.spawn((ItemPreview, Picture::default(), Visibility::Hidden, ChildOf(right)));
            let count = label(world, right, "", Vec2::new(5., 62.), 45.);
            world.entity_mut(count).insert(TradeCount);
            world.get_mut::<BitmapText>(count).unwrap().justify = bevy::text::Justify::Right;
            let price = label(world, right, "", Vec2::new(83., 58.), 51.);
            world.entity_mut(price).insert(TradePrice);
            world.get_mut::<BitmapText>(price).unwrap().justify = bevy::text::Justify::Right;
            world
                .spawn((
                    Button,
                    FocusRing,
                    TabIndex(0),
                    QuantityControl,
                    Pickable::default(),
                    Picture {
                        sheet: world
                            .resource::<AssetServer>()
                            .load("retail://data/npc interaction/npc_buttons.sti"),
                        frame: 30,
                        at: Vec2::new(53., 54.),
                        bounds: Some(30),
                    },
                    ControlSkin::new(30, 31, 34, 32, 33),
                    ChildOf(right),
                ))
                .observe(move |_: On<Activate>, mut commands: Commands| {
                    commands.queue(move |world: &mut World| quantity::open(world, root, canvas));
                });
            // Retail's Give mode also gives gold: the amount typed here when
            // no item is chosen.
            let gift_gold = world
                .spawn((
                    GiftGold,
                    TextInput,
                    EditableText {
                        max_characters: Some(9),
                        allow_newlines: false,
                        ..EditableText::new("")
                    },
                    bevy::text::EditableTextFilter::new(|c| c.is_ascii_digit()),
                    TextFont {
                        font: world
                            .resource::<AssetServer>()
                            .load("retail://data/fonts/wiz_text_font.sti#font")
                            .into(),
                        font_size: 11.0.into(),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    TabIndex(0),
                    Node {
                        display: Display::None,
                        ..absolute(Vec2::new(83., 58.), Vec2::new(51., 12.))
                    },
                    ChildOf(right),
                ))
                .id();
            let confirm = action(world, right, "", Vec2::new(56., 34.), Vec2::new(80., 18.));
            world.entity_mut(confirm).insert(Confirm);
            let caption = world.get::<Children>(confirm).unwrap()[0];
            world.entity_mut(caption).insert(Caption::TradeAction);
            world.entity_mut(confirm).observe(
                move |_: On<Activate>,
                      mut states: Query<&mut DialogueState>,
                      roots: Query<&ConversationView>,
                      sessions: Query<&Conversation>,
                      gold: Query<&EditableText, With<GiftGold>>,
                      mut requests: MessageWriter<ConversationRequest>| {
                    let Ok(mut state) = states.get_mut(root) else { return };
                    let amount = gold.get(gift_gold).map_or(0, gift_amount);
                    if state.mode == Mode::Give
                        && state.selected.is_none()
                        && amount > 0
                        && let Ok(view) = roots.get(root)
                        && let Ok(c) = sessions.get(view.0)
                    {
                        requests.write(ConversationRequest::GiveGold {
                            conversation: view.0,
                            revision: c.revision,
                            amount,
                        });
                        set_mode(&mut state, Mode::Talk);
                    } else if let Some(q) = state.selected
                        && roots.get(root).is_ok_and(|v| v.0 == q.conversation)
                    {
                        if state.mode != Mode::Give {
                            requests.write(ConversationRequest::Trade(q));
                        } else if let TradeSource::Party(item) = q.source {
                            requests.write(ConversationRequest::Give {
                                conversation: q.conversation,
                                revision: q.revision,
                                item,
                            });
                            // Retail returns to the transcript for the answer.
                            set_mode(&mut state, Mode::Talk);
                        }
                    }
                },
            );
            label(world, right, "Party gold:", Vec2::new(5., 77.), 76.);
            let gold = label(world, right, "", Vec2::new(83., 77.), 51.);
            world.entity_mut(gold).insert(Caption::Gold);
            world.get_mut::<BitmapText>(gold).unwrap().justify = bevy::text::Justify::Right;
        } else {
            let topics = world
                .spawn((
                    absolute(Vec2::new(476., 283.), Vec2::new(141., 18.)),
                    BackgroundColor(Color::BLACK),
                    ChildOf(group),
                ))
                .id();
            world.entity_mut(topics).insert(TopicList);
            let rows = world
                .spawn((
                    ScrollArea,
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(6),
                        top: px(4),
                        width: px(128),
                        height: px(144),
                        overflow: Overflow::scroll_y(),
                        flex_direction: FlexDirection::Column,
                        ..default()
                    },
                    ChildOf(topics),
                ))
                .id();
            world.entity_mut(rows).insert((TopicRows, ZIndex(1)));
            scroll_arrows(world, topics, rows, Vec2::new(126., 3.), 8.);
            let filters = panel(world, group, Vec2::new(476., 303.), Vec2::new(141., 55.), 5);
            let sort = action(
                world,
                filters,
                "Sort Alphabetically",
                Vec2::new(6., 2.),
                Vec2::new(130., 12.),
            );
            add_radio(world, sort);
            world.entity_mut(sort).insert(SortToggle);
            world.entity_mut(sort).observe(
                move |_: On<Activate>, mut states: Query<&mut DialogueState>| {
                    if let Ok(mut state) = states.get_mut(root) {
                        state.alphabetical = !state.alphabetical;
                    }
                },
            );
            for (i, (text, category)) in [
                ("People", Some(TopicCategory::Person)),
                ("Places", Some(TopicCategory::Place)),
                ("Items", Some(TopicCategory::Item)),
                ("Misc", Some(TopicCategory::Misc)),
                ("All", None),
            ]
            .into_iter()
            .enumerate()
            {
                let e = action(
                    world,
                    filters,
                    text,
                    Vec2::new(6. + 61. * (i % 2) as f32, 15. + 12. * (i / 2) as f32),
                    Vec2::new(61., 12.),
                );
                world.entity_mut(e).insert(CategoryButton(category));
                add_radio(world, e);
                world.entity_mut(e).observe(
                    move |_: On<Activate>, mut states: Query<&mut DialogueState>| {
                        if let Ok(mut state) = states.get_mut(root)
                            && state.category != category
                        {
                            state.category = category;
                        }
                    },
                );
            }
            let input_panel = panel(world, group, Vec2::new(476., 358.), Vec2::new(141., 92.), 13);
            let input = world
                .spawn((
                    TopicInput,
                    ModeFocus(false),
                    TextInput,
                    EditableText {
                        max_characters: Some(128),
                        allow_newlines: false,
                        ..EditableText::new("")
                    },
                    TextFont {
                        font: world
                            .resource::<AssetServer>()
                            .load("retail://data/fonts/wiz_text_font.sti#font")
                            .into(),
                        font_size: 11.0.into(),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    TabIndex(0),
                    absolute(Vec2::new(9., 8.), Vec2::new(122., 20.)),
                    ChildOf(input_panel),
                ))
                .id();
            world.entity_mut(input).observe(
                |mut e: On<FocusedInput<KeyboardInput>>, mut commands: Commands| {
                    if e.input.state.is_pressed() && e.input.logical_key == Key::Enter {
                        e.propagate(false);
                        commands.entity(e.original_event_target()).insert(SubmitTopic);
                    }
                },
            );
            for (i, text) in ["+", "-", "Talk About", "Where Is"].into_iter().enumerate() {
                let at = match i {
                    0 => Vec2::new(17., 35.),
                    1 => Vec2::new(38., 35.),
                    2 => Vec2::new(9., 51.),
                    _ => Vec2::new(9., 67.),
                };
                let e = action(
                    world,
                    input_panel,
                    text,
                    at,
                    Vec2::new(if i < 2 { 17. } else { 123. }, 16.),
                );
                if i < 2 {
                    let children: Vec<_> =
                        world.get::<Children>(e).into_iter().flatten().copied().collect();
                    for child in children {
                        world.despawn(child);
                    }
                    let first = if i == 0 { 5 } else { 10 };
                    let sheet = world
                        .resource::<AssetServer>()
                        .load("retail://data/npc interaction/npc_buttons.sti");
                    world.entity_mut(e).insert((
                        Picture { sheet, frame: first, at, bounds: Some(first) },
                        ControlSkin::new(first, first + 1, first + 4, first + 2, first + 3),
                        Pickable::default(),
                    ));
                }
                match i {
                    2 => {
                        world.entity_mut(e).observe(
                            move |_: On<Activate>, mut commands: Commands| {
                                commands.entity(input).insert(SubmitTopic);
                            },
                        );
                    }
                    0 | 1 => {
                        world.entity_mut(e).observe(
                            move |_: On<Activate>,
                                  inputs: Query<&EditableText>,
                                  mut facts: ResMut<StoryFacts>| {
                                let Ok(editor) = inputs.get(input) else { return };
                                if editor.is_composing() {
                                    return;
                                }
                                let text = editor.value().to_string().trim().to_owned();
                                if text.is_empty() {
                                    return;
                                }
                                if i == 1 {
                                    facts.topics.retain(|(_, t)| !t.eq_ignore_ascii_case(&text));
                                } else {
                                    facts.remember_topic(TopicCategory::Misc, &text);
                                }
                            },
                        );
                    }
                    _ => {
                        world.entity_mut(e).insert(InteractionDisabled);
                    }
                }
            }
            world.resource_mut::<InputFocus>().set(input, FocusCause::Navigated);
        }
    }
    build_menu(world, canvas, root);
    let bubble = world
        .spawn((
            QuoteBubble { line: None },
            Button,
            FocusRing,
            TabIndex(0),
            ScrollArea,
            Node {
                position_type: PositionType::Absolute,
                left: px(180),
                top: px(20),
                width: px(280),
                max_height: px(180),
                padding: UiRect::all(px(12)),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            BackgroundColor(Color::BLACK),
            ChildOf(canvas),
        ))
        .id();
    let background =
        world.resource::<AssetServer>().load("retail://data/npc interaction/npc_popup_back.pcx");
    world.entity_mut(bubble).insert(ImageNode {
        image: background,
        rect: Some(Rect::new(0., 0., 280., 180.)),
        ..default()
    });
    let text = bitmap(world, bubble, String::new(), 256.);
    let font = world.resource::<AssetServer>().load("retail://data/fonts/font12point1.sti");
    world.get_mut::<BitmapText>(text).unwrap().font = font;
    world.entity_mut(text).insert((
        Caption::Quote,
        BitmapWrap { width: 256, line_spacing: 2 },
        BitmapInk { foreground: 208, shadow: None },
    ));
    world.entity_mut(bubble).observe(
        move |_: On<Activate>,
              bubbles: Query<&QuoteBubble>,
              roots: Query<&ConversationView>,
              mut requests: MessageWriter<ConversationRequest>| {
            if let Ok(bubble) = bubbles.get(bubble)
                && let Some((session, speech)) = bubble.line
                && roots.get(root).is_ok_and(|v| v.0 == session)
            {
                requests.write(ConversationRequest::Advance { conversation: session, speech });
            }
        },
    );
    root
}
#[derive(Component)]
struct TopicRows;

fn select(world: &mut World, view: Entity, session: Entity, source: TradeSource, quantity: u32) {
    if world.get::<ConversationView>(view).is_none_or(|v| v.0 != session) {
        return;
    }
    // A gift has no price: one of the item is chosen as it is.
    let giving = world.get::<DialogueState>(view).is_some_and(|s| s.mode == Mode::Give);
    let result = if giving {
        world
            .get::<Conversation>(session)
            .map(|c| TradeQuote {
                conversation: session,
                revision: c.revision,
                source,
                quantity: 1,
                gold: 0,
            })
            .ok_or(trade::TradeError::StaleSelection)
    } else {
        quote_trade(world, session, source, quantity)
    };
    let Some(mut v) = world.get_mut::<DialogueState>(view) else { return };
    match result {
        Ok(q) => {
            v.selected = Some(q);
            v.error = None
        }
        Err(e) => {
            v.error = Some(format!("Trade unavailable: {e:?}"));
            v.selected = None;
        }
    }
}
/// Resolve a retained control through its real UI hierarchy, not a global
/// singleton or a separate registry of UI entity IDs.
#[derive(bevy::ecs::system::SystemParam)]
struct ViewScope<'w, 's> {
    roots: Query<'w, 's, &'static ConversationView>,
    states: Query<'w, 's, &'static DialogueState>,
    parents: Query<'w, 's, &'static ChildOf>,
}
impl ViewScope<'_, '_> {
    fn owner(&self, entity: Entity) -> Option<(Entity, Entity)> {
        scope(entity, &self.parents, &self.roots)
    }
}

/// Existing hierarchy owns controls; the relationship owns the complete view.
fn scope(
    entity: Entity,
    parents: &Query<&ChildOf>,
    roots: &Query<&ConversationView>,
) -> Option<(Entity, Entity)> {
    roots.get(entity).ok().map(|v| (entity, v.0)).or_else(|| {
        parents.iter_ancestors(entity).find_map(|root| roots.get(root).ok().map(|v| (root, v.0)))
    })
}
/// A dialogue the party has left plays out without its view.
fn close_views(
    mut commands: Commands,
    left: Query<&ConversationViews, Added<crate::game::npcs::conversation::Closing>>,
) {
    for views in &left {
        for &view in &views.0 {
            commands.entity(view).despawn();
        }
    }
}
fn create_views(
    mut commands: Commands,
    sessions: Query<
        (Entity, Option<&ConversationViews>),
        (With<Conversation>, Without<crate::game::npcs::conversation::Closing>),
    >,
) {
    for (session, views) in &sessions {
        if views.is_none_or(|v| v.0.is_empty()) {
            commands.queue(move |world: &mut World| {
                if world.get::<Conversation>(session).is_some()
                    && world.get::<crate::game::npcs::conversation::Closing>(session).is_none()
                    && world.get::<ConversationViews>(session).is_none_or(|v| v.0.is_empty())
                {
                    build(world, session);
                }
            });
        }
    }
}
fn project_text(
    views: ViewScope,
    sessions: Query<&Conversation>,
    speech: Query<&ConversationSpeech>,
    npcs: Query<&Npc>,
    definitions: Res<Assets<NpcCatalog>>,
    assets: Res<NpcAssets>,
    parties: Query<&PartyInventory, With<crate::game::party::roster::Party>>,
    mut labels: Query<(Entity, &Caption, &mut BitmapText)>,
) {
    for (entity, caption, mut text) in &mut labels {
        let Some((root, session)) = views.owner(entity) else { continue };
        let (Ok(state), Ok(c)) = (views.states.get(root), sessions.get(session)) else { continue };
        let value = match caption {
            Caption::Name => match state.mode {
                Mode::Talk => npcs
                    .get(c.actor)
                    .ok()
                    .and_then(|n| definitions.get(&assets.catalog)?.get(n.definition))
                    .map_or("NPC", |d| d.source_name.as_str())
                    .to_owned(),
                Mode::Buy => "Buy".into(),
                Mode::Sell => "Sell".into(),
                Mode::Give => "Give".into(),
            },
            Caption::Gold => parties.single().map(|p| format!("{}g", p.gold)).unwrap_or_default(),
            Caption::TradeAction => match state.mode {
                Mode::Buy => "Buy",
                Mode::Give => "Give",
                _ => "Sell",
            }
            .into(),
            Caption::Quote => {
                if let ConversationWait::Line { speech: entity, .. } = c.wait {
                    speech.get(entity).map(|s| s.text.clone()).unwrap_or_default()
                } else {
                    String::new()
                }
            }
        };
        if text.value != value {
            text.value = value;
        }
    }
}
/// What the NPC says as it hands over gold or an item, and as an item leaves
/// a member's pack or the party's items (retail `ProcessNpcQuoteEntry`,
/// `RemoveNpcScriptItem`), and the experience notice (`AwardPartyExperience`).
const QUEST_GOLD: usize = 0x7e9;
const QUEST_ITEM: usize = 0x7ea;
const ITEM_TAKEN_FROM: usize = 0x7ec;
const ITEM_TAKEN: usize = 0x7ed;
const QUEST_EXPERIENCE: usize = 0x232;
/// What an NPC says, in turn, to a question it has nothing to say to.
const NO_ANSWER: [usize; 2] = [1867, 1868];

fn sync_transcript(
    views: ViewScope,
    mut commands: Commands,
    assets: Res<AssetServer>,
    strings: super::screen_art::GameStrings,
    sessions: Query<&Conversation>,
    lists: Query<(Entity, Option<&Children>), With<Summary>>,
    mut rows: Query<(&TranscriptRow, &mut BitmapText)>,
) {
    for (list, children) in &lists {
        let Some((_, session)) = views.owner(list) else { continue };
        let Ok(c) = sessions.get(session) else { continue };
        // The current line is still in its speech bubble. Do not print it a
        // second time in the transcript before playback has finished.
        let count = c
            .transcript
            .len()
            .saturating_sub(usize::from(matches!(c.wait, ConversationWait::Line { .. })));
        let mut lines: Vec<_> = c.transcript[..count]
            .iter()
            .map(|line| match line {
                TranscriptLine::Npc(text) => (text.clone(), None),
                // Retail alternates its two answers for having nothing to say.
                TranscriptLine::NoAnswer { npc, second } => {
                    (strings.get(NO_ANSWER[usize::from(*second)]).replacen("%s", npc, 1), None)
                }
                TranscriptLine::Question(text) => (text.clone(), Some("palettebronze")),
                TranscriptLine::Reply(text) => (text.clone(), Some("palettegreen")),
                // Retail shows gifts and removals in the NPC's own quote.
                TranscriptLine::Reward(QuestReward::Gold { giver, amount }) => (
                    strings
                        .get(QUEST_GOLD)
                        .replacen("%s", giver, 1)
                        .replace("%d", &amount.to_string()),
                    None,
                ),
                TranscriptLine::Reward(QuestReward::Item { name }) => {
                    (strings.get(QUEST_ITEM).replacen("%s", name, 1), None)
                }
                TranscriptLine::Reward(QuestReward::Taken { member: Some(member) }) => {
                    (strings.get(ITEM_TAKEN_FROM).replacen("%s", member, 1), None)
                }
                TranscriptLine::Reward(QuestReward::Taken { member: None }) => {
                    (strings.get(ITEM_TAKEN).to_owned(), None)
                }
                TranscriptLine::Reward(QuestReward::Experience { amount_each }) => (
                    strings.get(QUEST_EXPERIENCE).replace("%d", &amount_each.to_string()),
                    Some("palettegreen"),
                ),
            })
            // A recording without a line ("SOUND") shows nothing.
            .filter(|(text, _)| !text.is_empty())
            .collect();
        if let ConversationWait::Blocked { quote, entry, reason } = &c.wait {
            lines.push((
                format!("Content gap at quote {quote}, entry {entry:?}: {reason}"),
                Some("palettered"),
            ));
        }
        if let Some(outcome) = c.outcome() {
            let text = match outcome {
                Outcome::NoTopic => "No supported topic matches that input.".into(),
                Outcome::Traded => "Trade complete.".into(),
                Outcome::TradeRefused(error) => format!("Trade refused: {error:?}"),
                Outcome::GiftRefused(error) => format!("Gift refused: {error:?}"),
            };
            lines.push((text, Some("palettebronze")));
        }
        let old: Vec<_> = children
            .into_iter()
            .flatten()
            .filter_map(|e| rows.get(*e).ok().map(|(row, _)| (*e, row.0)))
            .collect();
        let mut changed = old.len() != lines.len();
        for (entity, index) in &old {
            if *index >= lines.len() {
                commands.entity(*entity).despawn();
            }
        }
        for (index, (value, palette)) in lines.into_iter().enumerate() {
            let text = BitmapText {
                font: assets.load("retail://data/fonts/wiz_text_font.sti"),
                palette: palette.map(|name| assets.load(format!("retail://data/fonts/{name}.sti"))),
                value,
                width: 284.,
                ..default()
            };
            if let Some((entity, _)) = old.iter().find(|(_, i)| *i == index) {
                if let Ok((_, mut current)) = rows.get_mut(*entity) {
                    changed |= *current != text;
                    current.set_if_neq(text);
                }
            } else {
                commands.spawn((
                    TranscriptRow(index),
                    text,
                    BitmapWrap { width: 284, line_spacing: 0 },
                    Node { position_type: PositionType::Relative, flex_shrink: 0., ..default() },
                    ChildOf(list),
                ));
            }
        }
        if changed {
            commands.entity(list).insert(ScrollToNewest);
        }
    }
}
fn scroll_to_newest(
    mut commands: Commands,
    mut lists: Query<(Entity, &ComputedNode, &mut ScrollPosition), With<ScrollToNewest>>,
) {
    for (entity, node, mut scroll) in &mut lists {
        scroll.0.y = (node.content_size().y - node.size().y).max(0.) * node.inverse_scale_factor();
        commands.entity(entity).remove::<ScrollToNewest>();
    }
}
fn set_enabled(commands: &mut Commands, entity: Entity, enabled: bool, disabled: bool) {
    if enabled == disabled {
        if enabled {
            commands.entity(entity).remove::<InteractionDisabled>();
        } else {
            commands.entity(entity).insert(InteractionDisabled);
        }
    }
}
fn project_modes(
    views: ViewScope,
    mut commands: Commands,
    sessions: Query<&Conversation>,
    npcs: Query<&Npc>,
    definitions: Res<Assets<NpcCatalog>>,
    assets: Res<NpcAssets>,
    clock: Res<crate::game::time::GameClock>,
    mut panels: Query<
        (Entity, &mut Node, Option<&ModePanel>, Option<&mut QuoteBubble>, Has<MenuPanel>),
        Or<(With<ModePanel>, With<QuoteBubble>, With<MenuPanel>)>,
    >,
    approaches: Query<(Entity, &ApproachButton, Has<InteractionDisabled>)>,
    controls: Query<
        (
            Entity,
            Has<InteractionDisabled>,
            Has<bevy::ui::Selected>,
            Option<&ModeButton>,
            Option<&PoolButton>,
            Option<&CategoryButton>,
            Option<&ItemFilter>,
            Has<TradeAvailable>,
            Has<SortToggle>,
        ),
        Or<(
            With<ModeButton>,
            With<PoolButton>,
            With<CategoryButton>,
            With<ItemFilter>,
            With<TradeAvailable>,
            With<SortToggle>,
        )>,
    >,
) {
    for (entity, mut node, panel, bubble, menu) in &mut panels {
        let Some((root, session)) = views.owner(entity) else { continue };
        let (Ok(state), Ok(c)) = (views.states.get(root), sessions.get(session)) else { continue };
        let visible = if let Some(mut bubble) = bubble {
            let line = if let ConversationWait::Line { speech, .. } = c.wait {
                Some((session, speech))
            } else {
                None
            };
            if bubble.line != line {
                bubble.line = line;
            }
            state.mode == Mode::Talk && line.is_some()
        } else if menu {
            state.mode == Mode::Talk && c.menu
        } else {
            panel.is_some_and(|p| p.0 == (state.mode != Mode::Talk))
                && !(c.menu && state.mode == Mode::Talk)
        };
        let display = if visible { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
    // Each approach waits half a day once tried.
    for (entity, button, disabled) in &approaches {
        let Some((_, session)) = views.owner(entity) else { continue };
        let Some(npc) = sessions.get(session).ok().and_then(|c| npcs.get(c.actor).ok()) else {
            continue;
        };
        let at = match button.0 {
            Approach::Charm => npc.charmed_at,
            Approach::SmallTalk => npc.small_talk_at,
        };
        set_enabled(&mut commands, entity, approach_ready(at, clock.ms), disabled);
    }
    for (entity, disabled, was_selected, mode, pool, category, filter, trade, sort) in &controls {
        let Some((root, session)) = views.owner(entity) else { continue };
        let (Ok(state), Ok(c)) = (views.states.get(root), sessions.get(session)) else { continue };
        if trade {
            let can_trade = npcs
                .get(c.actor)
                .ok()
                .and_then(|n| definitions.get(&assets.catalog)?.get(n.definition))
                .is_some_and(|d| d.owns_stock);
            set_enabled(&mut commands, entity, can_trade, disabled);
        } else if pool.is_some() {
            set_enabled(
                &mut commands,
                entity,
                matches!(state.mode, Mode::Sell | Mode::Give),
                disabled,
            );
        }
        let selected = (sort && state.alphabetical)
            || mode.is_some_and(|m| state.mode == m.0)
            || pool.is_some_and(|p| state.pool == p.0)
            || category.is_some_and(|c| state.category == c.0)
            || filter.is_some_and(|f| match f {
                ItemFilter::Group(g) => state.item_group == Some(*g),
                ItemFilter::Usable(p) => state.usability == Some(*p),
            });
        if selected != was_selected {
            if selected {
                commands.entity(entity).insert(bevy::ui::Selected);
            } else {
                commands.entity(entity).remove::<bevy::ui::Selected>();
            }
        }
    }
}
/// Paint labels from the native selection state; sprite controls use ControlSkin.
fn colour_selections(
    views: ViewScope,
    controls: Query<(Has<bevy::ui::Selected>, Has<InteractionDisabled>), With<Button>>,
    mut labels: Query<(Entity, &ChildOf, &mut ImageNode), With<BitmapText>>,
) {
    for (entity, parent, mut image) in &mut labels {
        if views.owner(entity).is_none() {
            continue;
        }
        if let Ok((selected, disabled)) = controls.get(parent.parent()) {
            let color = if disabled {
                Color::srgb(0.5, 0.5, 0.5)
            } else if selected {
                Color::srgb(1., 0.9, 0.4)
            } else {
                Color::WHITE
            };
            let color = color.with_alpha(image.color.alpha());
            if image.color != color {
                image.color = color;
            }
        }
    }
}
/// DialogTextEntry uses brown keywords, white selection and yellow hover.
fn paint_topic_text(
    views: ViewScope,
    assets: Res<AssetServer>,
    rows: Query<(Entity, &TopicRow, &bevy::picking::hover::Hovered, &Children)>,
    sessions: Query<&Conversation>,
    inputs: Query<(Entity, &EditableText), With<TopicInput>>,
    mut texts: Query<&mut BitmapText>,
) {
    for (entity, row, hovered, children) in &rows {
        let Some((root, session)) = views.owner(entity) else { continue };
        let selected = inputs.iter().any(|(input, text)| {
            views.owner(input).is_some_and(|(owner, _)| owner == root)
                && text.value().to_string().eq_ignore_ascii_case(&row.0)
        });
        let name = if hovered.0 {
            "paletteyellow"
        } else if sessions
            .get(session)
            .is_ok_and(|c| c.learned_topics.contains(&row.0.to_ascii_lowercase()))
        {
            "palettegreen"
        } else if selected {
            "palettewhite"
        } else {
            "palettebrown"
        };
        let palette = Some(assets.load(format!("retail://data/fonts/{name}.sti")));
        for child in children.iter() {
            if let Ok(mut text) = texts.get_mut(child)
                && text.palette != palette
            {
                text.palette = palette.clone();
            }
        }
    }
}
fn route_mode_focus(
    views: ViewScope,
    panels: Query<&ModePanel>,
    targets: Query<(Entity, &ModeFocus)>,
    mut focus: ResMut<InputFocus>,
) {
    let Some(focused) = focus.get() else { return };
    let Some((root, _)) = views.owner(focused) else { return };
    let Ok(state) = views.states.get(root) else { return };
    if views
        .parents
        .iter_ancestors(focused)
        .any(|p| panels.get(p).is_ok_and(|p| p.0 != (state.mode != Mode::Talk)))
        && let Some((target, _)) = targets.iter().find(|(e, f)| {
            f.0 == (state.mode != Mode::Talk)
                && views.owner(*e).is_some_and(|(owner, _)| owner == root)
        })
    {
        focus.set(target, FocusCause::Navigated);
    }
}

fn row_button(
    commands: &mut Commands,
    parent: Entity,
    font: Handle<SpriteSheet>,
    value: String,
) -> (Entity, Entity) {
    let entity = commands
        .spawn((
            Button,
            FocusRing,
            TabIndex(0),
            Node { min_height: px(12), flex_shrink: 0., ..default() },
            ChildOf(parent),
        ))
        .id();
    let text = commands
        .spawn((
            BitmapText { font, value, ..default() },
            Node { position_type: PositionType::Relative, flex_shrink: 0., ..default() },
            Pickable::IGNORE,
            ChildOf(entity),
        ))
        .id();
    (entity, text)
}
fn activate_choice(
    event: On<Activate>,
    views: ViewScope,
    choices: Query<&Choice>,
    mut requests: MessageWriter<ConversationRequest>,
) {
    let Ok(choice) = choices.get(event.entity) else { return };
    let Some((_, session)) = views.owner(event.entity) else { return };
    if choice.session != session {
        return;
    }
    requests.write(match &choice.answer {
        Answer::Reply(text) => ConversationRequest::Reply {
            conversation: session,
            revision: choice.revision,
            text: text.clone(),
        },
        Answer::Price(accept) => ConversationRequest::PriceAnswer {
            conversation: session,
            revision: choice.revision,
            accept: *accept,
        },
    });
}
fn activate_trade(
    event: On<Activate>,
    views: ViewScope,
    rows: Query<&TradeRow>,
    mut commands: Commands,
) {
    let Ok(row) = rows.get(event.entity) else { return };
    let Some((root, session)) = views.owner(event.entity) else { return };
    if row.session != session {
        return;
    }
    let (source, quantity) = (row.source, row.quantity);
    commands.queue(move |world: &mut World| select(world, root, session, source, quantity));
}
fn activate_topic(
    event: On<Activate>,
    views: ViewScope,
    rows: Query<&TopicRow>,
    mut inputs: Query<(Entity, &mut EditableText), With<TopicInput>>,
) {
    let Ok(row) = rows.get(event.entity) else { return };
    let Some((root, _)) = views.owner(event.entity) else { return };
    for (entity, mut input) in &mut inputs {
        if views.owner(entity).is_some_and(|(owner, _)| owner == root) {
            input.clear();
            input.queue_edit(bevy::text::TextEdit::Insert(row.0.clone().into()));
        }
    }
}
fn sync_choices(
    views: ViewScope,
    mut commands: Commands,
    sheets: Res<Assets<SpriteSheet>>,
    strings: super::screen_art::GameStrings,
    sessions: Query<&Conversation>,
    parties: Query<&PartyInventory, With<crate::game::party::roster::Party>>,
    mut focus: ResMut<InputFocus>,
    mut overlays: Query<(Entity, &mut Node, &mut ChoiceOverlay), Without<Choices>>,
    mut lists: Query<
        (Entity, &Choices, &ChildOf, &mut Node, Option<&Children>),
        (With<Choices>, Without<ChoiceOverlay>, Without<Choice>),
    >,
    mut choices: Query<
        (&mut Choice, &Children, &mut Node),
        (Without<Choices>, Without<ChoiceOverlay>),
    >,
    mut labels: Query<(&mut BitmapText, Option<&ChoiceCaption>)>,
) {
    for (list, art, parent, mut node, children) in &mut lists {
        let font = &art.font;
        let string = |id, fallback: &str| {
            strings.table().and_then(|s| s.get(id)).unwrap_or(fallback).to_owned()
        };
        let Some((_, session)) = views.owner(list) else { continue };
        let Ok(c) = sessions.get(session) else { continue };
        let answers: Vec<(String, Answer)> = match &c.wait {
            // A typed answer goes through the topic input instead.
            ConversationWait::Options { words, typed: false, .. } => {
                words.iter().map(|w| (w.clone(), Answer::Reply(w.clone()))).collect()
            }
            ConversationWait::Price { .. } => vec![
                (string(0x7df, "Pay"), Answer::Price(true)),
                (string(0x7e0, "Decline"), Answer::Price(false)),
            ],
            // Retail's confirmation dialog for giving all the party's gold.
            ConversationWait::GiveAll { .. } => {
                vec![("OK".into(), Answer::Price(true)), ("Cancel".into(), Answer::Price(false))]
            }
            _ => vec![],
        };
        let Ok((overlay, mut overlay_node, mut state)) = overlays.get_mut(parent.parent()) else {
            continue;
        };
        let visible = !answers.is_empty();
        let was_visible = overlay_node.display != Display::None;
        if visible != was_visible {
            overlay_node.display = if visible { Display::Flex } else { Display::None };
            if visible {
                state.previous_focus = focus.get();
                commands.entity(overlay).insert((TabGroup::modal(), bevy::ui_widgets::ModalDialog));
            } else {
                commands.entity(overlay).remove::<(TabGroup, bevy::ui_widgets::ModalDialog)>();
                if let Some(previous) = state.previous_focus.take() {
                    focus.set(previous, FocusCause::Navigated);
                }
            }
        }
        let price = matches!(c.wait, ConversationWait::Price { .. });
        let button_width = sheets.get(font).map_or(70., |f| {
            answers.iter().map(|(t, _)| f.text_width(t)).max().unwrap_or(0) as f32 + 6.
        });
        let width = if price {
            (button_width * 2. + 50.).max(200.)
        } else {
            let width = (button_width + 10.) * answers.len() as f32 + 30.;
            if answers.len() == 2 { width.max(200.) } else { width }
        };
        let layout = absolute(
            Vec2::new((640. - width) / 2., 184.),
            Vec2::new(width, if price { 80. } else { 70. }),
        );
        node.set_if_neq(layout);
        if let Some(children) = children {
            for child in children.iter() {
                if let Ok((mut text, Some(caption))) = labels.get_mut(child) {
                    let value = if caption.0 {
                        if price {
                            format!(
                                "{} {}g",
                                string(0x7e6, "Party gold:"),
                                parties.single().map_or(0, |p| p.gold)
                            )
                        } else {
                            String::new()
                        }
                    } else if let ConversationWait::Price { gold, .. } = c.wait {
                        string(0x7e5, "Price: %d gold").replace("%d", &gold.to_string())
                    } else if matches!(c.wait, ConversationWait::GiveAll { .. }) {
                        string(0x7d6, "Give all your gold?")
                    } else {
                        string(0x7e4, "Choose a response")
                    };
                    if text.value != value {
                        text.value = value;
                    }
                    if text.width != width - 20. {
                        text.width = width - 20.;
                    }
                }
            }
        }
        let old: Vec<_> = children
            .into_iter()
            .flatten()
            .filter_map(|e| choices.get(*e).ok().map(|(c, _, _)| (*e, c.index)))
            .collect();
        for (entity, index) in &old {
            if *index >= answers.len() {
                commands.entity(*entity).despawn();
            }
        }
        let left = (width - ((button_width + 10.) * answers.len() as f32 - 10.)) / 2.;
        for (index, (label, answer)) in answers.into_iter().enumerate() {
            let value = Choice { session, index, revision: c.revision, answer };
            let mut layout = absolute(
                Vec2::new(
                    left + index as f32 * (button_width + 10.),
                    if price { 30. } else { 35. },
                ),
                Vec2::new(button_width, 20.),
            );
            layout.justify_content = JustifyContent::Center;
            layout.align_items = AlignItems::Center;
            if let Some((entity, _)) = old.iter().find(|(_, i)| *i == index) {
                if let Ok((mut choice, children, mut node)) = choices.get_mut(*entity) {
                    choice.set_if_neq(value);
                    node.set_if_neq(layout);
                    if let Some(child) = children.first()
                        && let Ok((mut text, _)) = labels.get_mut(*child)
                        && text.value != label
                    {
                        text.value = label;
                    }
                }
            } else {
                let (entity, text) = row_button(&mut commands, list, font.clone(), label);
                commands.entity(text).insert(BitmapInk { foreground: 4, shadow: Some(5) });
                commands.entity(entity).insert((value, layout));
            }
        }
    }
}

fn sync_topic_rows(
    views: ViewScope,
    mut commands: Commands,
    assets: Res<AssetServer>,
    facts: Res<StoryFacts>,
    lists: Query<(Entity, Option<&Children>), With<TopicRows>>,
    rows: Query<&TopicRow>,
) {
    for (list, children) in &lists {
        let Some((root, _)) = views.owner(list) else { continue };
        let Ok(state) = views.states.get(root) else { continue };
        let mut topics: Vec<_> = facts
            .topics
            .iter()
            .filter(|(c, _)| state.category.is_none_or(|wanted| wanted == *c))
            .map(|(_, t)| t.clone())
            .collect();
        if state.alphabetical {
            topics.sort_by_key(|t| t.to_lowercase());
        }
        let old: Vec<_> = children
            .into_iter()
            .flatten()
            .filter_map(|e| rows.get(*e).ok().map(|r| (*e, &r.0)))
            .collect();
        for (entity, text) in &old {
            if !topics.contains(text) {
                commands.entity(*entity).despawn();
            }
        }
        let mut ordered = Vec::with_capacity(topics.len());
        for text in topics {
            if let Some((entity, _)) = old.iter().find(|(_, t)| **t == text) {
                ordered.push(*entity);
            } else {
                let (entity, _) = row_button(
                    &mut commands,
                    list,
                    assets.load("retail://data/fonts/wiz_text_font.sti"),
                    text.clone(),
                );
                commands.entity(entity).insert(Node {
                    height: px(12),
                    min_height: px(12),
                    flex_shrink: 0.,
                    align_items: AlignItems::Center,
                    ..default()
                });
                ordered.push(entity);
                commands
                    .entity(entity)
                    .insert((TopicRow(text.clone()), bevy::picking::hover::Hovered::default()));
            }
        }
        if children.is_none_or(|c| c.iter().ne(ordered.iter().copied())) {
            commands.entity(list).replace_children(&ordered);
        }
    }
}
/// The gold amount typed to give the NPC.
#[derive(Component)]
struct GiftGold;
fn gift_amount(text: &EditableText) -> u32 {
    text.value().to_string().parse().unwrap_or(0)
}
#[derive(bevy::ecs::system::SystemParam)]
struct TradeWidgets<'w, 's> {
    lists: Query<'w, 's, (Entity, Option<&'static Children>), With<TradeList>>,
    row_data: Query<'w, 's, (&'static mut TradeRow, &'static Children)>,
    offers: Query<'w, 's, Entity, With<Offer>>,
    captions: Query<'w, 's, (Entity, Has<TradeCount>), Or<(With<TradeCount>, With<TradePrice>)>>,
    quantities: Query<'w, 's, (Entity, Has<InteractionDisabled>), With<QuantityControl>>,
    texts: Query<'w, 's, &'static mut BitmapText>,
    controls: Query<'w, 's, (Entity, Has<InteractionDisabled>), With<Confirm>>,
    previews:
        Query<'w, 's, (Entity, &'static mut Picture, &'static mut Visibility), With<ItemPreview>>,
    gifts: Query<'w, 's, (Entity, &'static EditableText, &'static mut Node), With<GiftGold>>,
}
fn sync_trade_rows(
    views: ViewScope,
    mut commands: Commands,
    assets: Res<AssetServer>,
    content: crate::game::data::Content,
    sessions: Query<&Conversation>,
    npcs: Query<&Npc>,
    parties: Query<(&PartyRoster, &PartyInventory), With<crate::game::party::roster::Party>>,
    members: Query<(&crate::game::characters::Character, &Carried)>,
    selection: Res<SelectedMember>,
    widgets: TradeWidgets,
) {
    let TradeWidgets {
        lists,
        mut row_data,
        offers,
        captions,
        quantities,
        mut texts,
        controls,
        mut previews,
        mut gifts,
    } = widgets;
    let Some(items) = content.items() else { return };
    let Ok((roster, party)) = parties.single() else { return };
    for (list, children) in &lists {
        let Some((root, session)) = views.owner(list) else { continue };
        let (Ok(state), Ok(c)) = (views.states.get(root), sessions.get(session)) else { continue };
        if state.mode == Mode::Talk {
            continue;
        }
        let Ok(npc) = npcs.get(c.actor) else { continue };
        let member = selection.0.filter(|e| roster.contains(e));
        let allows = |d: &crate::content::items::ItemDefinition| {
            state.item_group.is_none_or(|group| group == ItemGroup::of(d.class))
                && state.usability.is_none_or(|party_wide| {
                    if party_wide {
                        roster.iter().any(|e| members.get(e).is_ok_and(|(c, _)| c.can_use(d)))
                    } else {
                        member.and_then(|e| members.get(e).ok()).is_some_and(|(c, _)| c.can_use(d))
                    }
                })
        };
        let mut rows = Vec::new();
        if state.mode == Mode::Buy {
            for lot in &npc.stock.lots {
                if let Some(d) = items.get(lot.item)
                    && trade::on_sale(lot, d)
                    && allows(d)
                {
                    rows.push((
                        TradeSource::Stock(lot.id),
                        format!("{} ({})", d.name, lot.units),
                        trade::trade_quantity(d, lot.units),
                    ));
                }
            }
        } else {
            let carried = member.and_then(|e| members.get(e).ok()).map(|(_, c)| c);
            let pool = party.pool.stacks.iter().filter(|_| state.pool);
            let backpack = carried
                .into_iter()
                .flat_map(|c| c.backpack.iter().flatten())
                .filter(|_| !state.pool);
            for item in pool.chain(backpack) {
                if let Some(d) = items.get(item.item)
                    && allows(d)
                {
                    let owner = if state.pool {
                        ItemOwner::Pool
                    } else {
                        let Some(member) = member else { continue };
                        ItemOwner::Member(member)
                    };
                    rows.push((
                        TradeSource::Party(ItemRef { owner, id: item.id }),
                        item.display_name(d, true),
                        trade::trade_quantity(d, item.units()),
                    ));
                }
            }
        }
        let quote = state.selected.filter(|q| {
            q.conversation == session
                && q.revision == c.revision
                && rows.iter().any(|r| r.0 == q.source)
        });
        let item = quote
            .and_then(|q| match q.source {
                TradeSource::Stock(id) => {
                    npc.stock.lots.iter().find(|l| l.id == id).map(|l| l.item)
                }
                TradeSource::Party(item) => match item.owner {
                    ItemOwner::Pool => {
                        party.pool.stacks.iter().find(|s| s.id == item.id).map(|s| s.item)
                    }
                    ItemOwner::Member(e) => {
                        members.get(e).ok()?.1.find_stack(item.id).map(|s| s.item)
                    }
                },
            })
            .and_then(|id| items.get(id));
        let offer = if let Some(error) = &state.error {
            error.clone()
        } else if let Some(item) = item {
            item.name.clone()
        } else {
            String::new()
        };
        for entity in &offers {
            if views.owner(entity).is_some_and(|(owner, _)| owner == root)
                && let Ok(mut text) = texts.get_mut(entity)
                && text.value != offer
            {
                text.value = offer.clone();
            }
        }
        let units = quote
            .and_then(|q| match q.source {
                TradeSource::Stock(id) => {
                    npc.stock.lots.iter().find(|l| l.id == id).map(|l| l.units)
                }
                TradeSource::Party(r) => match r.owner {
                    ItemOwner::Pool => {
                        party.pool.stacks.iter().find(|s| s.id == r.id).map(|s| s.units())
                    }
                    ItemOwner::Member(e) => {
                        members.get(e).ok()?.1.find_stack(r.id).map(|s| s.units())
                    }
                },
            })
            .unwrap_or(0);
        for (entity, count) in &captions {
            if views.owner(entity).is_some_and(|(owner, _)| owner == root)
                && let Ok(mut text) = texts.get_mut(entity)
            {
                let value = quote.map_or_else(String::new, |q| {
                    if count {
                        if units > 1 { q.quantity.to_string() } else { String::new() }
                    } else if state.mode == Mode::Give {
                        String::new()
                    } else {
                        format!("{}g", q.gold)
                    }
                });
                if text.value != value {
                    text.value = value;
                }
            }
        }
        for (entity, disabled) in &quantities {
            if views.owner(entity).is_some_and(|(owner, _)| owner == root) {
                set_enabled(
                    &mut commands,
                    entity,
                    state.mode != Mode::Give
                        && units > 1
                        && item.is_some_and(|d| d.class != EquipClass::Ammunition),
                    disabled,
                );
            }
        }
        let mut gold_typed = false;
        for (entity, text, mut node) in &mut gifts {
            if views.owner(entity).is_some_and(|(owner, _)| owner == root) {
                let shown = state.mode == Mode::Give && quote.is_none();
                let display = if shown { Display::Flex } else { Display::None };
                if node.display != display {
                    node.display = display;
                }
                gold_typed = shown && gift_amount(text) > 0;
            }
        }
        for (entity, disabled) in &controls {
            if views.owner(entity).is_some_and(|(owner, _)| owner == root) {
                set_enabled(&mut commands, entity, quote.is_some() || gold_typed, disabled);
            }
        }
        for (entity, mut picture, mut visibility) in &mut previews {
            if !views.owner(entity).is_some_and(|(owner, _)| owner == root) {
                continue;
            }
            visibility.set_if_neq(if item.is_some() {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
            if let Some(item) = item {
                picture.set_if_neq(Picture {
                    sheet: assets.load(format!("retail://{}", item.image)),
                    frame: 1,
                    at: Vec2::new(5., 20.),
                    bounds: None,
                });
            }
        }
        let old: Vec<_> = children
            .into_iter()
            .flatten()
            .filter_map(|e| row_data.get(*e).ok().map(|(r, _)| (*e, r.source)))
            .collect();
        for (entity, source) in &old {
            if !rows.iter().any(|r| r.0 == *source) {
                commands.entity(*entity).despawn();
            }
        }
        let mut ordered = Vec::with_capacity(rows.len());
        for (source, text, quantity) in rows {
            if let Some((entity, _)) = old.iter().find(|(_, s)| *s == source) {
                ordered.push(*entity);
                if let Ok((mut row, children)) = row_data.get_mut(*entity) {
                    row.set_if_neq(TradeRow { session, source, quantity });
                    if let Some(child) = children.first()
                        && let Ok(mut label) = texts.get_mut(*child)
                        && label.value != text
                    {
                        label.value = text;
                    }
                }
            } else {
                let (entity, label) = row_button(
                    &mut commands,
                    list,
                    assets.load("retail://data/fonts/wiz_text_font.sti"),
                    text,
                );
                ordered.push(entity);
                commands.entity(label).insert(BitmapWrap { width: 280, line_spacing: 0 });
                commands.entity(entity).insert(TradeRow { session, source, quantity });
            }
        }
        if children.is_none_or(|c| c.iter().ne(ordered.iter().copied())) {
            commands.entity(list).replace_children(&ordered);
        }
    }
}
fn sync_bubble_geometry(
    mut commands: Commands,
    assets: Res<AssetServer>,
    sheets: Res<Assets<SpriteSheet>>,
    mut bubbles: Query<(Entity, &mut Node, &mut ImageNode, &Children), With<QuoteBubble>>,
    mut labels: Query<(&BitmapText, &mut BitmapWrap, &Caption)>,
    edges: Query<(), With<BubbleEdge>>,
) {
    for (bubble, mut node, mut image, children) in &mut bubbles {
        let Some(text_entity) = children
            .iter()
            .find(|e| labels.get(*e).is_ok_and(|(_, _, c)| matches!(c, Caption::Quote)))
        else {
            continue;
        };
        let Ok((text, mut wrap, _)) = labels.get_mut(text_entity) else { continue };
        let Some(font) = sheets.get(&text.font) else { continue };
        let width = (font.text_width(&text.value) + 24).clamp(25, 280);
        let wrapped = crate::content::ui::wrap_text(&font.frames, &text.value, width - 24);
        let line = font
            .frames
            .first()
            .map_or(12, |f| (f.height as i32 + f.offset[1] as i32).max(1) as u32);
        let height = (wrapped.split('\n').count() as u32 * (line + 2) - 2 + 24).min(198);
        if node.width == px(width as f32) && node.height == px(height as f32) {
            continue;
        }
        node.left = px(320. - width as f32 / 2.);
        node.width = px(width as f32);
        node.height = px(height as f32);
        node.max_height = px(198);
        image.rect = Some(Rect::new(0., 0., width as f32, height as f32));
        wrap.width = width - 24;
        for child in children {
            if edges.contains(*child) {
                commands.entity(*child).despawn();
            }
        }
        let mut parts = vec![
            (0, Vec2::ZERO),
            (2, Vec2::new(width as f32 - 16., 0.)),
            (5, Vec2::new(0., height as f32 - 16.)),
            (7, Vec2::new(width as f32 - 16., height as f32 - 16.)),
        ];
        for x in (16..width.saturating_sub(16)).step_by(16) {
            parts.push((1, Vec2::new(x as f32, 0.)));
            parts.push((6, Vec2::new(x as f32, height as f32 - 16.)));
        }
        for y in (16..height.saturating_sub(16)).step_by(16) {
            parts.push((3, Vec2::new(0., y as f32)));
            parts.push((4, Vec2::new(width as f32 - 8., y as f32)));
        }
        for (frame, at) in parts {
            commands.spawn((
                BubbleEdge,
                Picture {
                    sheet: assets.load("retail://data/npc interaction/npc_popup_edge.sti"),
                    frame,
                    at,
                    bounds: None,
                },
                ChildOf(bubble),
            ));
        }
    }
}
fn sync_topic_geometry(
    mut commands: Commands,
    assets: Res<AssetServer>,
    sheets: Res<Assets<SpriteSheet>>,
    mut panels: Query<(Entity, &mut Node, &Children), (With<TopicList>, Without<TopicRows>)>,
    mut lists: Query<(&mut Node, Option<&Children>), (With<TopicRows>, Without<TopicList>)>,
    chrome: Query<(), With<TopicChrome>>,
    mut arrows: Query<(&ControlSkin, &mut Picture)>,
) {
    let sheet = assets.load::<SpriteSheet>("retail://data/npc interaction/npc_bottompanel.sti");
    let Some((line, margin)) =
        sheets.get(&sheet).and_then(|s| Some((s.frames.get(3)?.height, s.frames.get(4)?.height)))
    else {
        return;
    };
    if line == 0 {
        return;
    }
    for (panel, mut node, children) in &mut panels {
        let Some(list) = children.iter().find(|e| lists.contains(*e)) else { continue };
        let Ok((mut list_node, rows)) = lists.get_mut(list) else { continue };
        let height = (rows.map_or(0, |c| c.len()) as u32 * line).clamp(line.min(255), 255);
        if node.height == px(height as f32) {
            continue;
        }
        node.top = px(301. - height as f32);
        node.height = px(height as f32);
        list_node.top = px(0);
        list_node.width = px(118);
        list_node.height = px(height as f32);
        for child in children {
            if let Ok((skin, mut picture)) = arrows.get_mut(*child) {
                picture.at =
                    Vec2::new(126., height as f32 - 18. + if skin.normal == 20 { 3. } else { 11. });
            }
            if chrome.contains(*child) {
                commands.entity(*child).despawn();
            }
        }
        let mut parts =
            vec![(2, Vec2::new(0., height as f32 - 18.)), (4, Vec2::new(0., -(margin as f32)))];
        for y in (0..height.saturating_sub(line)).step_by(line as usize) {
            parts.push((3, Vec2::new(0., y as f32)));
        }
        for (frame, at) in parts {
            commands.spawn((
                TopicChrome,
                Picture { sheet: sheet.clone(), frame, at, bounds: None },
                ChildOf(panel),
            ));
        }
    }
}

fn submit_topic(
    views: ViewScope,
    mut commands: Commands,
    inputs: Query<(Entity, &EditableText), With<SubmitTopic>>,
    sessions: Query<&Conversation>,
    mut requests: MessageWriter<ConversationRequest>,
) {
    for (entity, text) in &inputs {
        if !text.pending_edits.is_empty() || text.pending_paste.is_some() {
            continue;
        }
        if let Some((_, session)) = views.owner(entity)
            && let Ok(c) = sessions.get(session)
        {
            let text = text.value().to_string();
            // Retail's keyword dialog: a question the party answers by typing.
            requests.write(if matches!(c.wait, ConversationWait::Options { typed: true, .. }) {
                ConversationRequest::Reply { conversation: session, revision: c.revision, text }
            } else {
                ConversationRequest::Topic { conversation: session, revision: c.revision, text }
            });
        }
        commands.entity(entity).remove::<SubmitTopic>();
    }
}
fn play_lines(
    mut commands: Commands,
    lines: Query<(Entity, &ConversationSpeech), Added<ConversationSpeech>>,
    assets: Res<AssetServer>,
) {
    for (e, line) in &lines {
        if let Some(voice) = &line.voice {
            commands.entity(e).insert((
                bevy::audio::AudioPlayer::new(assets.load(super::voice::voice_path(voice))),
                bevy::audio::PlaybackSettings::ONCE,
            ));
        }
        // Retail `SpeakNpcSubquote`: the alert chime, while the line waits
        // out its text, not the sound.
        if line.alert {
            commands.spawn((
                bevy::audio::AudioPlayer::new(
                    assets.load("retail://data/sound/npcs/dialogue alert.wav"),
                ),
                bevy::audio::PlaybackSettings::DESPAWN,
            ));
        }
    }
}
fn watch_playback(
    mut commands: Commands,
    lines: Query<(Entity, &bevy::audio::AudioSink, Has<PlaybackStarted>), With<ConversationSpeech>>,
    mut events: MessageWriter<LinePlayback>,
) {
    use bevy::audio::AudioSinkPlayback;
    for (e, sink, started) in &lines {
        if !started {
            events.write(LinePlayback { speech: e, event: PlaybackEvent::Started });
            commands.entity(e).insert(PlaybackStarted);
        }
        if sink.empty() {
            events.write(LinePlayback { speech: e, event: PlaybackEvent::Finished });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{items::ItemCatalog, ui::TextTable};

    #[test]
    fn session_ownership_rejects_stale_choices_and_cleans_only_its_views() {
        let mut world = World::new();
        world.init_resource::<Messages<ConversationRequest>>();
        world.add_observer(activate_choice);
        let first = world.spawn_empty().id();
        let second = world.spawn_empty().id();
        let view = world.spawn((ConversationView(first), DialogueState::default())).id();
        let other = world.spawn((ConversationView(second), DialogueState::default())).id();
        let row = world
            .spawn((
                Button,
                Choice { session: first, index: 0, revision: 7, answer: Answer::Price(true) },
                ChildOf(view),
            ))
            .id();
        world.trigger(Activate { entity: row });
        let requests: Vec<_> =
            world.resource_mut::<Messages<ConversationRequest>>().drain().collect();
        assert!(
            matches!(&requests[..],[ConversationRequest::PriceAnswer{conversation,revision:7,accept:true}] if *conversation==first)
        );
        // A stale callback cannot act on the old session after replacement,
        // even if that old session still exists and has the same revision.
        world.entity_mut(view).insert(ConversationView(second));
        world.trigger(Activate { entity: row });
        assert!(world.resource::<Messages<ConversationRequest>>().is_empty());
        select(
            &mut world,
            view,
            first,
            TradeSource::Stock(crate::game::npcs::trade::StockLotId(1)),
            1,
        );
        assert!(world.get::<DialogueState>(view).unwrap().selected.is_none());
        world.despawn(first);
        assert!(world.get_entity(view).is_ok());
        world.despawn(second);
        for entity in [view, other, row] {
            assert!(world.get_entity(entity).is_err());
        }
    }

    #[test]
    fn topic_projection_retains_rows_and_late_art_without_crossing_view_boundaries() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<SpriteSheet>()
            .init_resource::<Assets<Image>>()
            .init_resource::<StoryFacts>()
            .init_resource::<crate::game::time::GameClock>()
            .init_resource::<Assets<NpcCatalog>>()
            .init_resource::<Assets<TextTable>>()
            .init_resource::<Assets<ItemCatalog>>()
            .init_resource::<NpcAssets>()
            .init_resource::<SelectedMember>()
            .init_resource::<InputFocus>()
            .add_message::<ConversationRequest>()
            .add_message::<LinePlayback>()
            .add_plugins(ConversationViewPlugin)
            .add_systems(Update, super::super::screen_art::render_text.after(sync_topic_rows));
        app.world_mut().resource_mut::<StoryFacts>().topics =
            vec![(TopicCategory::Person, "Burz".into()), (TopicCategory::Place, "Arnika".into())];
        let first = app.world_mut().spawn_empty().id();
        let second = app.world_mut().spawn_empty().id();
        let view = app
            .world_mut()
            .spawn((
                ConversationView(first),
                DialogueState { category: Some(TopicCategory::Person), ..default() },
            ))
            .id();
        let other = app
            .world_mut()
            .spawn((
                ConversationView(second),
                DialogueState { category: Some(TopicCategory::Place), ..default() },
            ))
            .id();
        let list = app.world_mut().spawn((TopicRows, ChildOf(view))).id();
        let other_list = app.world_mut().spawn((TopicRows, ChildOf(other))).id();
        app.update();
        let row = app.world().get::<Children>(list).unwrap()[0];
        let other_row = app.world().get::<Children>(other_list).unwrap()[0];
        assert_eq!(app.world().get::<TopicRow>(row).unwrap().0, "Burz");
        assert_eq!(app.world().get::<TopicRow>(other_row).unwrap().0, "Arnika");
        let label = app.world().get::<Children>(row).unwrap()[0];
        let font = app.world().get::<BitmapText>(label).unwrap().font.clone();
        let frames = vec![
            crate::content::ui::Frame {
                offset: [0, 0],
                width: 2,
                height: 3,
                rgba: vec![255; 24]
            };
            150
        ];
        app.world_mut()
            .resource_mut::<Assets<SpriteSheet>>()
            .insert(font.id(), SpriteSheet { frames, images: vec![], palette: None })
            .unwrap();
        let palette = app.world().get::<BitmapText>(label).unwrap().palette.clone().unwrap();
        app.world_mut()
            .resource_mut::<Assets<SpriteSheet>>()
            .insert(palette.id(), SpriteSheet { frames: vec![], images: vec![], palette: None })
            .unwrap();
        app.world_mut()
            .resource_mut::<StoryFacts>()
            .topics
            .push((TopicCategory::Person, "Antone".into()));
        app.world_mut().get_mut::<DialogueState>(view).unwrap().alphabetical = true;
        app.update();
        app.update();
        assert_eq!(app.world().get::<Children>(list).unwrap()[1], row);
        assert_eq!(app.world().get::<Children>(other_list).unwrap()[0], other_row);
        assert_eq!(app.world().get::<ImageNode>(label).unwrap().color.alpha(), 1.);
        assert_eq!(app.world().get::<Node>(label).unwrap().width, px(8));
        app.world_mut().despawn(first);
        assert!(app.world().get_entity(row).is_err());
        assert!(app.world().get_entity(other_row).is_ok());
    }
}

/// Reproducible inspection only: use normal conversation/quote validation and
/// never submit a purchase. Used by `--inspect-trade` with offscreen captures.
#[derive(Resource)]
pub struct InspectTrade(pub bool);
pub(crate) fn inspect_trade(world: &mut World) {
    let Some(split) = world.get_resource::<InspectTrade>().map(|s| s.0) else { return };
    let Some((session, actor, wait, ready)) = world
        .query::<(Entity, &Conversation)>()
        .iter(world)
        .next()
        .map(|(e, c)| (e, c.actor, c.wait.clone(), c.ready()))
    else {
        return;
    };
    if !ready {
        if let ConversationWait::Line { speech, .. } = wait {
            world.write_message(ConversationRequest::Advance { conversation: session, speech });
        }
        return;
    }
    let Some(view) = world
        .query::<(Entity, &ConversationView)>()
        .iter(world)
        .find(|(_, v)| v.0 == session)
        .map(|(e, _)| e)
    else {
        return;
    };
    let Some(canvas) = world.get::<Children>(view).and_then(|c| c.first().copied()) else { return };
    set_mode(&mut world.get_mut::<DialogueState>(view).unwrap(), Mode::Buy);
    let item = crate::game::data::GameData::items(world).and_then(|items| {
        world
            .get::<Npc>(actor)?
            .stock
            .lots
            .iter()
            .find(|lot| {
                items.get(lot.item).is_some_and(|d| {
                    trade::on_sale(lot, d)
                        && (!split || (lot.units > 1 && d.class != EquipClass::Ammunition))
                })
            })
            .map(|lot| (lot.id, trade::trade_quantity(items.get(lot.item).unwrap(), lot.units)))
    });
    if let Some((lot, count)) = item {
        select(world, view, session, TradeSource::Stock(lot), count);
        if split {
            quantity::open(world, view, canvas);
        }
    } else {
        warn!("no matching stock for --inspect-trade");
    }
    world.remove_resource::<InspectTrade>();
}
