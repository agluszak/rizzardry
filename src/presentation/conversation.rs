//! Native dialogue widgets. All selected rows and quotes are drafts; gameplay
//! resolves item/stock identities again when a confirmation executes.
use super::main_screen::SelectedMember;
use super::retail_controls::{ControlSkin, FocusRing};
use super::screen_art::{BitmapText, BitmapWrap, Picture, absolute};
use crate::content::npcs::TopicCategory;
use crate::content::{
    items::{EquipClass, ItemCatalog},
    npcs::NpcCatalog,
    ui::SpriteSheet,
};
use crate::game::npcs::{
    Npc, NpcAssets,
    conversation::*,
    trade::{TradeQuote, TradeSource},
};
use crate::game::{
    data::GameData,
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
struct ConversationView {
    session: Entity,
    selected: Option<TradeQuote>,
    error: Option<String>,
    mode: Mode,
    pool: bool,
    category: Option<TopicCategory>,
    alphabetical: bool,
    item_group: Option<ItemGroup>,
    usability: Option<bool>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Talk,
    Buy,
    Sell,
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
#[derive(Component)]
struct ModeFocus(bool);
#[derive(Component)]
struct BubbleEdge;
#[derive(Component)]
struct TopicChrome;
#[derive(Component)]
struct NpcName;
#[derive(Component)]
struct TopicList;
#[derive(Component)]
struct TopicRow(String);
#[derive(Component)]
struct QuoteBubble;
#[derive(Component)]
struct QuoteText;
#[derive(Component)]
struct ItemPreview;
#[derive(Component)]
struct Gold;
#[derive(Component)]
struct Confirm;
#[derive(Component)]
struct ModeButton(Mode);
#[derive(Component)]
struct PoolButton(bool);
#[derive(Component)]
struct CategoryButton(Option<TopicCategory>);
#[derive(Component)]
struct TradeAvailable;

#[derive(Component)]
struct Summary;
#[derive(Component)]
struct Offer;
#[derive(Component)]
struct Choices;
#[derive(Clone)]
enum Answer {
    Reply(String),
    Price(bool),
}
#[derive(Component)]
struct Choice {
    index: usize,
    revision: u64,
    answer: Answer,
}
#[derive(Component)]
struct TopicInput;
#[derive(Component)]
struct SubmitTopic;
#[derive(Component)]
struct TradeList;
#[derive(Component, Clone, Copy)]
struct TradeRow {
    source: TradeSource,
    quantity: u32,
}
#[derive(Component)]
struct PlaybackStarted;

pub struct ConversationViewPlugin;
impl Plugin for ConversationViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (sync_view, play_lines, watch_playback)
                .chain()
                .after(crate::game::npcs::conversation::update_conversation),
        )
        .add_systems(PostUpdate, submit_topic.after(bevy::text::EditableTextSystems));
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
            Node { min_height: px(14), flex_shrink: 0.0, ..default() },
            ChildOf(parent),
        ))
        .id();
    bitmap(world, e, text.into(), 0.0);
    e
}
fn action(world: &mut World, parent: Entity, text: &str, at: Vec2, size: Vec2) -> Entity {
    let e = button(world, parent, text);
    world.entity_mut(e).insert(absolute(at, size));
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
fn set_mode(world: &mut World, root: Entity, mode: Mode) {
    if let Some(mut view) = world.get_mut::<ConversationView>(root) {
        view.mode = mode;
        view.selected = None;
        view.error = None;
    }
    let target = world
        .query::<(Entity, &ModeFocus)>()
        .iter(world)
        .find(|(_, f)| f.0 == (mode != Mode::Talk))
        .map(|(e, _)| e);
    if let Some(target) = target {
        world.resource_mut::<InputFocus>().set(target, FocusCause::Navigated);
    }
    for mut scroll in world.query_filtered::<&mut ScrollPosition, With<TradeList>>().iter_mut(world)
    {
        *scroll = default();
    }
}
fn close(world: &mut World, session: Entity) {
    world.write_message(ConversationRequest::Close { conversation: session });
}
fn build(world: &mut World, session: Entity) -> Entity {
    let root = world
        .spawn((
            ConversationView {
                session,
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
        move |mut e: On<FocusedInput<KeyboardInput>>, mut commands: Commands| {
            if e.input.state.is_pressed() && e.input.logical_key == Key::Escape {
                e.propagate(false);
                commands.queue(move |world: &mut World| {
                    if world.get::<ConversationView>(root).is_some_and(|v| v.mode != Mode::Talk) {
                        set_mode(world, root, Mode::Talk);
                    } else {
                        close(world, session);
                    }
                });
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
        world.entity_mut(name).insert(NpcName);
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
                (false, 0) | (true, 0) | (true, 1) => {
                    let mode = if i == 1 { Mode::Sell } else { Mode::Buy };
                    world.entity_mut(e).insert((ModeButton(mode), TradeAvailable));
                    world.entity_mut(e).observe(move |_: On<Activate>, mut commands: Commands| {
                        commands.queue(move |w: &mut World| set_mode(w, root, mode));
                    });
                }
                (false, 5) => {
                    world.entity_mut(e).observe(move |_: On<Activate>, mut commands: Commands| {
                        commands.queue(move |w: &mut World| close(w, session));
                    });
                }
                (true, 4..=5) => {
                    let pool = i == 4;
                    world.entity_mut(e).insert(PoolButton(pool));
                    world.entity_mut(e).observe(move |_: On<Activate>, mut commands: Commands| {
                        commands.queue(move |w: &mut World| {
                            if let Some(mut v) = w.get_mut::<ConversationView>(root) {
                                v.pool = pool;
                                v.selected = None;
                                v.error = None;
                            }
                        });
                    });
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
                    .observe(move |_: On<Activate>, mut commands: Commands| {
                        commands.queue(move |w: &mut World| {
                            if let Some(mut v) = w.get_mut::<ConversationView>(root) {
                                match filter {
                                    ItemFilter::Group(group) => {
                                        v.item_group = if v.item_group == Some(group) {
                                            None
                                        } else {
                                            Some(group)
                                        }
                                    }
                                    ItemFilter::Usable(party) => {
                                        v.usability = if v.usability == Some(party) {
                                            None
                                        } else {
                                            Some(party)
                                        }
                                    }
                                }
                                v.selected = None;
                                v.error = None;
                            }
                        });
                    });
            }
            let back = action(world, left, "Back", Vec2::new(114., 71.), Vec2::new(23., 16.));
            world.entity_mut(back).insert(ModeFocus(true));
            world.entity_mut(back).observe(move |_: On<Activate>, mut commands: Commands| {
                commands.queue(move |w: &mut World| set_mode(w, root, Mode::Talk));
            });
        }
        let centre = panel(world, group, Vec2::new(164., 358.), Vec2::new(312., 92.), 1);
        let list = world
            .spawn((
                ScrollArea,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(4),
                    top: px(4),
                    width: px(294),
                    height: px(if trade { 83 } else { 62 }),
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
            scroll_arrows(world, centre, list, Vec2::new(298., 4.), 48.);
            let text = bitmap(world, list, String::new(), 290.);
            world.entity_mut(text).insert(BitmapWrap { width: 290, line_spacing: 0 });
            world.spawn((
                Choices,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(4),
                    top: px(67),
                    width: px(300),
                    height: px(22),
                    overflow: Overflow::scroll_x(),
                    column_gap: px(6),
                    ..default()
                },
                ScrollArea,
                ChildOf(centre),
            ));
        }
        if trade {
            let right = panel(world, group, Vec2::new(476., 358.), Vec2::new(141., 92.), 6);
            let offer = label(world, right, "Select an item", Vec2::new(5., 4.), 130.);
            world.entity_mut(offer).insert((Offer, BitmapWrap { width: 130, line_spacing: 0 }));
            let preview = world.spawn((ItemPreview, Picture::default(), ChildOf(right))).id();
            world.entity_mut(preview).insert(Visibility::Hidden);
            for (i, (text, delta)) in [("-", -1), ("+", 1), ("Confirm", 0)].into_iter().enumerate()
            {
                let e = action(
                    world,
                    right,
                    text,
                    Vec2::new(
                        54. + if i < 2 { i as f32 * 25. } else { 0. },
                        if i < 2 { 36. } else { 55. },
                    ),
                    Vec2::new(if i < 2 { 22. } else { 78. }, 16.),
                );
                world.entity_mut(e).insert(Confirm);
                world.entity_mut(e).observe(move |_: On<Activate>, mut commands: Commands| {
                    commands.queue(move |w: &mut World| {
                        let Some(q) = w.get::<ConversationView>(root).and_then(|v| v.selected)
                        else {
                            return;
                        };
                        if delta == 0 {
                            w.write_message(ConversationRequest::Trade(q));
                        } else {
                            select(
                                w,
                                root,
                                session,
                                q.source,
                                if delta > 0 {
                                    q.quantity.saturating_add(1)
                                } else {
                                    q.quantity.saturating_sub(1).max(1)
                                },
                            );
                        }
                    });
                });
            }
            let gold = label(world, right, "", Vec2::new(5., 78.), 130.);
            world.entity_mut(gold).insert(Gold);
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
            world.entity_mut(sort).observe(move |_: On<Activate>, mut commands: Commands| {
                commands.queue(move |w: &mut World| {
                    if let Some(mut v) = w.get_mut::<ConversationView>(root) {
                        v.alphabetical = !v.alphabetical;
                    }
                });
            });
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
                world.entity_mut(e).observe(move |_: On<Activate>, mut commands: Commands| {
                    commands.queue(move |w: &mut World| {
                        if let Some(mut v) = w.get_mut::<ConversationView>(root) {
                            v.category = category;
                        }
                    });
                });
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
                    TextFont { font_size: 11.0.into(), ..default() },
                    TextColor(Color::srgb(0.95, 0.8, 0.6)),
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
                            move |_: On<Activate>, mut commands: Commands| {
                                commands.queue(move |w: &mut World| {
                                    let Some(text) = w
                                        .get::<EditableText>(input)
                                        .map(|t| t.value().trim().to_owned())
                                    else {
                                        return;
                                    };
                                    if text.is_empty() {
                                        return;
                                    }
                                    let mut facts = w.resource_mut::<StoryFacts>();
                                    if i == 1 {
                                        facts
                                            .topics
                                            .retain(|(_, t)| !t.eq_ignore_ascii_case(&text));
                                    } else if !facts
                                        .topics
                                        .iter()
                                        .any(|(_, t)| t.eq_ignore_ascii_case(&text))
                                    {
                                        facts.topics.push((TopicCategory::Misc, text));
                                    }
                                });
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
    let bubble = world
        .spawn((
            QuoteBubble,
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
    world.entity_mut(text).insert((QuoteText, BitmapWrap { width: 256, line_spacing: 2 }));
    world.entity_mut(bubble).observe(move |_: On<Activate>, mut commands: Commands| {
        commands.queue(move |w: &mut World| {
            if let Some(ConversationWait::Line { speech, .. }) =
                w.get::<Conversation>(session).map(|c| &c.wait)
            {
                let speech = *speech;
                w.write_message(ConversationRequest::Advance { conversation: session, speech });
            }
        });
    });
    root
}
#[derive(Component)]
struct TopicRows;

fn select(world: &mut World, view: Entity, session: Entity, source: TradeSource, quantity: u32) {
    let result = quote_trade(world, session, source, quantity);
    let Some(mut v) = world.get_mut::<ConversationView>(view) else { return };
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
fn sync_view(world: &mut World) {
    let session = world.query_filtered::<Entity, With<Conversation>>().iter(world).next();
    let views: Vec<_> = world
        .query::<(Entity, &ConversationView)>()
        .iter(world)
        .map(|(e, v)| (e, v.session))
        .collect();
    for (e, s) in views {
        if Some(s) != session {
            world.despawn(e);
        }
    }
    let Some(session) = session else { return };
    let root = world
        .query_filtered::<Entity, With<ConversationView>>()
        .iter(world)
        .next()
        .unwrap_or_else(|| build(world, session));
    let Some(party) = crate::game::party::roster::the_party(world) else { return };
    let c = world.get::<Conversation>(session).unwrap();
    let revision = c.revision;
    let answers: Vec<(String, Answer)> = match &c.wait {
        ConversationWait::Options { words, .. } => {
            words.iter().map(|word| (word.clone(), Answer::Reply(word.clone()))).collect()
        }
        ConversationWait::Price { gold, .. } => vec![
            (format!("Pay {gold} gold"), Answer::Price(true)),
            ("Decline".into(), Answer::Price(false)),
        ],
        _ => vec![],
    };
    let actor = c.actor;
    let def = world.get::<Npc>(actor).and_then(|n| {
        let assets = world.resource::<NpcAssets>();
        world
            .resource::<Assets<NpcCatalog>>()
            .get(&assets.catalog)
            .and_then(|c| c.get(n.definition))
    });
    let name = def.map_or("NPC", |d| d.source_name.as_str());
    let name = name.to_owned();
    let state = match &c.wait {
        ConversationWait::Blocked { quote, entry, reason } => {
            format!("Content gap at quote {quote}, entry {entry:?}: {reason}")
        }
        _ => String::new(),
    };
    let speech = if let ConversationWait::Line { speech, .. } = c.wait {
        world.get::<ConversationSpeech>(speech).map(|s| s.text.clone()).unwrap_or_default()
    } else {
        String::new()
    };
    let text = [c.transcript.join("\n"), state, c.result.clone().unwrap_or_default()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let view = world.get::<ConversationView>(root).unwrap();
    let (mut mode, pool, category, alphabetical) =
        (view.mode, view.pool, view.category, view.alphabetical);
    let (item_group, usability) = (view.item_group, view.usability);
    let can_trade = trade_allowed(world, actor);
    if !can_trade && mode != Mode::Talk {
        set_mode(world, root, Mode::Talk);
        mode = Mode::Talk;
    }
    for (panel, mut node) in world.query::<(&ModePanel, &mut Node)>().iter_mut(world) {
        node.display = if panel.0 == (mode != Mode::Talk) { Display::Flex } else { Display::None };
    }
    for mut node in world.query_filtered::<&mut Node, With<QuoteBubble>>().iter_mut(world) {
        node.display =
            if mode == Mode::Talk && !speech.is_empty() { Display::Flex } else { Display::None };
    }
    set_label::<QuoteText>(world, &speech);
    sync_bubble(world, &speech);
    set_label::<NpcName>(
        world,
        match mode {
            Mode::Talk => &name,
            Mode::Buy => "Buy",
            Mode::Sell => "Sell",
        },
    );
    let gold = world.get::<PartyInventory>(party).unwrap().gold;
    set_label::<Gold>(world, &format!("Party gold: {gold}"));
    let available: Vec<_> =
        world.query_filtered::<Entity, With<TradeAvailable>>().iter(world).collect();
    for e in available {
        enabled(world, e, can_trade);
    }
    let pool_buttons: Vec<_> =
        world.query::<(Entity, &PoolButton)>().iter(world).map(|(e, b)| (e, b.0)).collect();
    for (e, is_pool) in pool_buttons {
        enabled(world, e, mode == Mode::Sell);
        selected(world, e, pool == is_pool);
    }
    let mode_buttons: Vec<_> =
        world.query::<(Entity, &ModeButton)>().iter(world).map(|(e, b)| (e, b.0)).collect();
    for (e, m) in mode_buttons {
        selected(world, e, mode == m);
    }
    let category_buttons: Vec<_> =
        world.query::<(Entity, &CategoryButton)>().iter(world).map(|(e, b)| (e, b.0)).collect();
    for (e, c) in category_buttons {
        selected(world, e, category == c);
    }
    let filters: Vec<_> =
        world.query::<(Entity, &ItemFilter)>().iter(world).map(|(e, f)| (e, *f)).collect();
    for (e, f) in filters {
        selected(
            world,
            e,
            match f {
                ItemFilter::Group(g) => item_group == Some(g),
                ItemFilter::Usable(p) => usability == Some(p),
            },
        );
    }
    sync_topics(world, category, alphabetical);
    let summary = world
        .query_filtered::<&Children, With<Summary>>()
        .iter(world)
        .next()
        .and_then(|c| c.first().copied());
    if let Some(e) = summary
        && let Some(mut t) = world.get_mut::<BitmapText>(e)
        && t.value != text
    {
        t.value = text;
    }
    let choices = world.query_filtered::<Entity, With<Choices>>().iter(world).next().unwrap();
    let old_choices: Vec<_> =
        world.query::<(Entity, &Choice)>().iter(world).map(|(e, c)| (e, c.index)).collect();
    for (entity, index) in &old_choices {
        if *index >= answers.len() {
            world.despawn(*entity);
        }
    }
    for (index, (label, answer)) in answers.into_iter().enumerate() {
        if let Some((entity, _)) = old_choices.iter().find(|(_, i)| *i == index) {
            world.entity_mut(*entity).insert(Choice { index, revision, answer });
            if let Some(child) = world.get::<Children>(*entity).and_then(|c| c.first().copied())
                && let Some(mut text) = world.get_mut::<BitmapText>(child)
                && text.value != label
            {
                text.value = label;
            }
        } else {
            let entity = button(world, choices, &label);
            world.entity_mut(entity).insert(Choice { index, revision, answer });
            world.entity_mut(entity).observe(
                move |_: On<Activate>,
                      choices: Query<&Choice>,
                      mut requests: MessageWriter<ConversationRequest>| {
                    let Ok(choice) = choices.get(entity) else { return };
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
                },
            );
        }
    }
    if mode == Mode::Talk || !can_trade {
        return;
    }
    let member = world
        .resource::<SelectedMember>()
        .0
        .filter(|e| world.get::<PartyRoster>(party).is_some_and(|r| r.contains(e)));
    let items = world
        .get_resource::<GameData>()
        .and_then(|d| world.resource::<Assets<ItemCatalog>>().get(&d.items));
    let Some(items) = items else { return };
    let allows = |d: &crate::content::items::ItemDefinition| {
        item_group.is_none_or(|group| group == ItemGroup::of(d.class))
            && usability.is_none_or(|party_wide| {
                if party_wide {
                    world.get::<PartyRoster>(party).into_iter().flat_map(|r| r.iter()).any(|e| {
                        world
                            .get::<crate::game::characters::Character>(e)
                            .is_some_and(|c| c.can_use(d))
                    })
                } else {
                    member
                        .and_then(|e| world.get::<crate::game::characters::Character>(e))
                        .is_some_and(|c| c.can_use(d))
                }
            })
    };
    let mut rows = Vec::new();
    if mode == Mode::Buy
        && let Some(npc) = world.get::<Npc>(actor)
    {
        for lot in &npc.stock.lots {
            if lot.units == 0 || lot.available_at != 0 {
                continue;
            }
            if let Some(d) = items.get(lot.item)
                && !d.merchant_cannot_sell
                && allows(d)
            {
                rows.push((
                    TradeSource::Stock(lot.id),
                    format!("{} ({})", d.name, lot.units),
                    if d.class == EquipClass::Ammunition { lot.units } else { 1 },
                ));
            }
        }
    }
    if mode == Mode::Sell && pool {
        for item in &world.get::<PartyInventory>(party).unwrap().pool.stacks {
            if let Some(d) = items.get(item.item)
                && allows(d)
            {
                rows.push((
                    TradeSource::Party(ItemRef { owner: ItemOwner::Pool, id: item.id }),
                    item.display_name(d, true),
                    if d.class == EquipClass::Ammunition { item.units() } else { 1 },
                ));
            }
        }
    }
    if mode == Mode::Sell && !pool {
        if let Some(member) = member {
            if let Some(carried) = world.get::<Carried>(member) {
                for item in carried.backpack.iter().flatten() {
                    if let Some(d) = items.get(item.item)
                        && allows(d)
                    {
                        rows.push((
                            TradeSource::Party(ItemRef {
                                owner: ItemOwner::Member(member),
                                id: item.id,
                            }),
                            item.display_name(d, true),
                            if d.class == EquipClass::Ammunition { item.units() } else { 1 },
                        ));
                    }
                }
            }
        }
    }
    let view = world.get::<ConversationView>(root).unwrap();
    let quote = view.selected.filter(|q| rows.iter().any(|r| r.0 == q.source));
    let offer = if let Some(err) = &view.error {
        err.clone()
    } else if let Some(q) = quote {
        format!(
            "{}\n{} x  {} gold",
            rows.iter().find(|r| r.0 == q.source).unwrap().1,
            q.quantity,
            q.gold
        )
    } else {
        "Select an item".into()
    };
    let image = quote
        .and_then(|q| match q.source {
            TradeSource::Stock(id) => {
                world.get::<Npc>(actor)?.stock.lots.iter().find(|l| l.id == id).map(|l| l.item)
            }
            TradeSource::Party(item) => match item.owner {
                ItemOwner::Pool => world
                    .get::<PartyInventory>(party)?
                    .pool
                    .stacks
                    .iter()
                    .find(|s| s.id == item.id)
                    .map(|s| s.item),
                ItemOwner::Member(member) => {
                    world.get::<Carried>(member)?.find_stack(item.id).map(|s| s.item)
                }
            },
        })
        .and_then(|id| items.get(id))
        .map(|d| d.image.clone());
    set_label::<Offer>(world, &offer);
    let controls: Vec<_> = world.query_filtered::<Entity, With<Confirm>>().iter(world).collect();
    for e in controls {
        enabled(world, e, quote.is_some());
    }
    let handle = image.map(|p| world.resource::<AssetServer>().load(format!("retail://{p}")));
    for (mut picture, mut visibility) in
        world.query_filtered::<(&mut Picture, &mut Visibility), With<ItemPreview>>().iter_mut(world)
    {
        *visibility = if handle.is_some() { Visibility::Inherited } else { Visibility::Hidden };
        if let Some(sheet) = &handle {
            picture.set_if_neq(Picture {
                sheet: sheet.clone(),
                frame: 1,
                at: Vec2::new(5., 28.),
                bounds: None,
            });
        }
    }
    let old: Vec<_> =
        world.query::<(Entity, &TradeRow)>().iter(world).map(|(e, r)| (e, r.source)).collect();
    for (e, source) in &old {
        if !rows.iter().any(|r| r.0 == *source) {
            world.despawn(*e);
        }
    }
    let list = world.query_filtered::<Entity, With<TradeList>>().iter(world).next().unwrap();
    let mut ordered = Vec::with_capacity(rows.len());
    for (source, text, quantity) in rows {
        if let Some((e, _)) = old.iter().find(|(_, s)| *s == source) {
            ordered.push(*e);
            if let Some(mut row) = world.get_mut::<TradeRow>(*e)
                && row.quantity != quantity
            {
                row.quantity = quantity;
            }
            let child = world.get::<Children>(*e).and_then(|c| c.first().copied());
            if let Some(child) = child
                && let Some(mut label) = world.get_mut::<BitmapText>(child)
                && label.value != text
            {
                label.value = text;
            }
        } else {
            let e = button(world, list, &text);
            world.entity_mut(e).insert(TradeRow { source, quantity });
            ordered.push(e);
            if let Some(child) = world.get::<Children>(e).and_then(|c| c.first().copied()) {
                world.entity_mut(child).insert(BitmapWrap { width: 280, line_spacing: 0 });
            }
            world.entity_mut(e).observe(
                move |_: On<Activate>, rows: Query<&TradeRow>, mut commands: Commands| {
                    if let Ok(row) = rows.get(e) {
                        let (source, quantity) = (row.source, row.quantity);
                        commands.queue(move |world: &mut World| {
                            select(world, root, session, source, quantity)
                        });
                    }
                },
            );
        }
    }
    if world
        .get::<Children>(list)
        .is_none_or(|children| children.iter().copied().ne(ordered.iter().copied()))
    {
        world.entity_mut(list).replace_children(&ordered);
    }
}
/// PortraitQuote.cpp: 280px maximum, 12px margins, 2px line spacing,
/// centred at y=20; edge frames are tiled, never stretched.
fn sync_bubble(world: &mut World, speech: &str) {
    let font =
        world.resource::<AssetServer>().load::<SpriteSheet>("retail://data/fonts/font12point1.sti");
    let Some(font) = world.resource::<Assets<SpriteSheet>>().get(&font) else { return };
    let width = (font.text_width(speech) + 24).min(280).max(25);
    let wrapped = crate::content::ui::wrap_text(&font.frames, speech, width - 24);
    let line =
        font.frames.first().map_or(12, |f| (f.height as i32 + f.offset[1] as i32).max(1) as u32);
    let rows = wrapped.split('\n').count() as u32;
    let height = (rows * (line + 2) - 2 + 24).min(198);
    let bubble = world.query_filtered::<Entity, With<QuoteBubble>>().iter(world).next().unwrap();
    let changed = world
        .get::<Node>(bubble)
        .is_none_or(|n| n.width != px(width as f32) || n.height != px(height as f32));
    if !changed {
        return;
    }
    if let Some(mut n) = world.get_mut::<Node>(bubble) {
        n.left = px(320. - width as f32 / 2.);
        n.width = px(width as f32);
        n.height = px(height as f32);
        n.max_height = px(198);
    }
    if let Some(mut image) = world.get_mut::<ImageNode>(bubble) {
        image.rect = Some(Rect::new(0., 0., width as f32, height as f32));
    }
    for mut wrap in world.query_filtered::<&mut BitmapWrap, With<QuoteText>>().iter_mut(world) {
        wrap.width = width - 24;
    }
    let old: Vec<_> = world.query_filtered::<Entity, With<BubbleEdge>>().iter(world).collect();
    for e in old {
        world.despawn(e);
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
        let e = picture(world, bubble, "npc_popup_edge", frame, at);
        world.entity_mut(e).insert(BubbleEdge);
    }
}

fn set_label<T: Component>(world: &mut World, value: &str) {
    for mut text in world.query_filtered::<&mut BitmapText, With<T>>().iter_mut(world) {
        if text.value != value {
            text.value = value.to_owned();
        }
    }
}
fn enabled(world: &mut World, e: Entity, value: bool) {
    let disabled = world.get::<InteractionDisabled>(e).is_some();
    if value == disabled {
        if value {
            world.entity_mut(e).remove::<InteractionDisabled>();
        } else {
            world.entity_mut(e).insert(InteractionDisabled);
        }
    }
}
fn selected(world: &mut World, e: Entity, value: bool) {
    if world.get::<bevy::ui::Selected>(e).is_some() != value {
        if value {
            world.entity_mut(e).insert(bevy::ui::Selected);
        } else {
            world.entity_mut(e).remove::<bevy::ui::Selected>();
        }
    }
    let child = world.get::<Children>(e).and_then(|c| c.first().copied());
    if let Some(child) = child
        && let Some(mut image) = world.get_mut::<ImageNode>(child)
    {
        let color = if value { Color::srgb(1., 0.9, 0.4) } else { Color::WHITE };
        if image.color != color {
            image.color = color;
        }
    }
}
fn topic_geometry(world: &mut World, count: usize) {
    let sheet = world
        .resource::<AssetServer>()
        .load::<SpriteSheet>("retail://data/npc interaction/npc_bottompanel.sti");
    let Some((line, margin)) = world
        .resource::<Assets<SpriteSheet>>()
        .get(&sheet)
        .and_then(|s| Some((s.frames.get(3)?.height, s.frames.get(4)?.height)))
    else {
        return;
    };
    if line == 0 {
        return;
    }
    let height = (count as u32 * line).clamp(line, 255);
    let panel = world.query_filtered::<Entity, With<TopicList>>().iter(world).next().unwrap();
    if world.get::<Node>(panel).is_some_and(|n| n.height == px(height as f32)) {
        return;
    }
    if let Some(mut node) = world.get_mut::<Node>(panel) {
        node.top = px(301. - height as f32);
        node.height = px(height as f32);
    }
    for mut node in world.query_filtered::<&mut Node, With<TopicRows>>().iter_mut(world) {
        node.top = px(0);
        node.width = px(118);
        node.height = px(height as f32);
    }
    for (parent, skin, mut picture) in
        world.query::<(&ChildOf, &ControlSkin, &mut Picture)>().iter_mut(world)
    {
        if parent.parent() == panel {
            picture.at =
                Vec2::new(126., height as f32 - 18. + if skin.normal == 20 { 3. } else { 11. });
        }
    }
    let old: Vec<_> = world.query_filtered::<Entity, With<TopicChrome>>().iter(world).collect();
    for e in old {
        world.despawn(e);
    }
    let mut parts =
        vec![(2, Vec2::new(0., height as f32 - 18.)), (4, Vec2::new(0., -(margin as f32)))];
    for y in (0..height.saturating_sub(line)).step_by(line as usize) {
        parts.push((3, Vec2::new(0., y as f32)));
    }
    for (frame, at) in parts {
        let e = picture(world, panel, "npc_bottompanel", frame, at);
        world.entity_mut(e).insert(TopicChrome);
    }
}
fn sync_topics(world: &mut World, category: Option<TopicCategory>, alphabetical: bool) {
    let mut topics: Vec<_> = world
        .resource::<StoryFacts>()
        .topics
        .iter()
        .filter(|(c, _)| category.is_none_or(|wanted| wanted == *c))
        .map(|(_, t)| t.clone())
        .collect();
    topic_geometry(world, topics.len());
    if alphabetical {
        topics.sort_by_key(|t| t.to_lowercase());
    }
    let list = world.query_filtered::<Entity, With<TopicRows>>().iter(world).next().unwrap();
    let current: Vec<_> = world
        .get::<Children>(list)
        .into_iter()
        .flatten()
        .filter_map(|e| world.get::<TopicRow>(*e).map(|t| t.0.clone()))
        .collect();
    if current == topics {
        return;
    }
    let children: Vec<_> = world.get::<Children>(list).into_iter().flatten().copied().collect();
    for e in children {
        world.despawn(e);
    }
    let input = world.query_filtered::<Entity, With<TopicInput>>().iter(world).next().unwrap();
    for text in topics {
        let e = button(world, list, &text);
        world.entity_mut(e).insert(TopicRow(text.clone()));
        world.entity_mut(e).observe(move |_: On<Activate>, mut commands: Commands| {
            commands.entity(input).insert(EditableText {
                max_characters: Some(128),
                allow_newlines: false,
                ..EditableText::new(&text)
            });
        });
    }
}
fn submit_topic(
    mut commands: Commands,
    inputs: Query<(Entity, &EditableText), With<SubmitTopic>>,
    sessions: Query<(Entity, &Conversation)>,
    mut requests: MessageWriter<ConversationRequest>,
) {
    let Ok((session, c)) = sessions.single() else { return };
    for (entity, text) in &inputs {
        if !text.pending_edits.is_empty() || text.pending_paste.is_some() {
            continue;
        }
        requests.write(ConversationRequest::Topic {
            conversation: session,
            revision: c.revision,
            text: text.value().to_string(),
        });
        commands.entity(entity).remove::<SubmitTopic>();
    }
}
fn play_lines(
    mut commands: Commands,
    lines: Query<(Entity, &ConversationSpeech), Without<bevy::audio::AudioPlayer>>,
    assets: Res<AssetServer>,
) {
    for (e, line) in &lines {
        commands.entity(e).insert((
            bevy::audio::AudioPlayer::new(assets.load(super::voice::voice_path(&line.voice))),
            bevy::audio::PlaybackSettings::ONCE,
        ));
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
