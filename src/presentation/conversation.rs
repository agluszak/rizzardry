//! Native dialogue widgets. All selected rows and quotes are drafts; gameplay
//! resolves item/stock identities again when a confirmation executes.
use super::screen_art::BitmapText;
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
}
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
#[derive(Component)]
struct TradeControls;
#[derive(Component, Clone, Copy)]
struct TradeRow(TradeSource);
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
            BitmapText {
                font,
                value,
                width,
                at: Vec2::ZERO,
                palette: None,
                justify: bevy::text::Justify::Left,
            },
            Node { position_type: PositionType::Relative, min_height: px(16), ..default() },
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}
fn button(world: &mut World, parent: Entity, label: &str) -> Entity {
    let e = world
        .spawn((
            Button,
            Node {
                padding: UiRect::axes(px(8), px(3)),
                flex_shrink: 0.0,
                min_height: px(20),
                border: UiRect::all(px(1)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.12, 0.1, 0.07)),
            BorderColor::all(Color::srgb(0.7, 0.5, 0.3)),
            TabIndex(0),
            ChildOf(parent),
        ))
        .id();
    bitmap(world, e, label.into(), 0.0);
    e
}
fn build(world: &mut World, session: Entity) -> Entity {
    let root = world
        .spawn((
            ConversationView { session, selected: None, error: None },
            Node {
                position_type: PositionType::Absolute,
                left: percent(8),
                top: percent(4),
                width: percent(84),
                height: percent(92),
                padding: UiRect::all(px(10)),
                row_gap: px(6),
                flex_direction: FlexDirection::Column,
                border: UiRect::all(px(2)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.025, 0.02, 0.98)),
            BorderColor::all(Color::srgb(0.7, 0.5, 0.3)),
            GlobalZIndex(40),
            TabGroup::modal(),
            bevy::ui_widgets::ModalDialog,
        ))
        .id();
    world.entity_mut(root).observe(
        move |mut e: On<FocusedInput<KeyboardInput>>,
              mut requests: MessageWriter<ConversationRequest>| {
            if e.input.state.is_pressed() && e.input.logical_key == Key::Escape {
                e.propagate(false);
                requests.write(ConversationRequest::Close { conversation: session });
            }
        },
    );
    let summary = world
        .spawn((
            Summary,
            ScrollArea,
            Node {
                height: px(130),
                position_type: PositionType::Relative,
                width: percent(100),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    bitmap(world, summary, String::new(), 510.0);
    let input = world
        .spawn((
            TopicInput,
            TextInput,
            EditableText {
                max_characters: Some(128),
                allow_newlines: false,
                ..EditableText::new("")
            },
            TextFont { font_size: 14.0.into(), ..default() },
            TextColor(Color::srgb(0.95, 0.8, 0.6)),
            TabIndex(0),
            Node {
                min_height: px(25),
                width: percent(100),
                border: UiRect::all(px(1)),
                ..default()
            },
            BorderColor::all(Color::srgb(0.7, 0.5, 0.3)),
            ChildOf(root),
        ))
        .id();
    world.entity_mut(input).observe(
        |mut e: On<FocusedInput<KeyboardInput>>,
         editors: Query<&EditableText, With<TopicInput>>,
         mut commands: Commands| {
            let target = e.original_event_target();
            if e.input.state.is_pressed()
                && e.input.logical_key == Key::Enter
                && editors.get(target).is_ok_and(|t| !t.is_composing())
            {
                e.propagate(false);
                commands.entity(target).insert(SubmitTopic);
            }
        },
    );
    let actions = world.spawn((Node { column_gap: px(5), ..default() }, ChildOf(root))).id();
    for text in ["Ask", "Continue", "Close"] {
        let e = button(world, actions, text);
        world.entity_mut(e).observe(
            move |_: On<Activate>,
                  c: Query<&Conversation>,
                  inputs: Query<Entity, With<TopicInput>>,
                  mut commands: Commands,
                  mut requests: MessageWriter<ConversationRequest>| {
                let Ok(c) = c.get(session) else { return };
                match text {
                    "Ask" => {
                        for input in &inputs {
                            commands.entity(input).insert(SubmitTopic);
                        }
                    }
                    "Close" => {
                        requests.write(ConversationRequest::Close { conversation: session });
                    }
                    "Continue" => {
                        if let ConversationWait::Line { speech, .. } = c.wait {
                            requests.write(ConversationRequest::Advance {
                                conversation: session,
                                speech,
                            });
                        }
                    }
                    _ => {}
                }
            },
        );
    }
    world.spawn((Choices, Node { column_gap: px(5), ..default() }, ChildOf(root)));
    let offer = world.spawn((Offer, TradeControls, Node { min_height: px(18), ..default() }, ChildOf(root))).id();
    bitmap(world, offer, "Select merchandise or an item to sell.".into(), 510.0);
    let controls = world.spawn((TradeControls, Node { column_gap: px(5), ..default() }, ChildOf(root))).id();
    for (label, change) in [("-", -1), ("+", 1), ("Confirm trade", 0)] {
        let e = button(world, controls, label);
        world.entity_mut(e).observe(move |_: On<Activate>, mut commands: Commands| {
            commands.queue(move |world: &mut World| {
                let Some(view) = world.get::<ConversationView>(root) else { return };
                let selected = view.selected;
                let session = view.session;
                if change == 0 {
                    if let Some(q) = selected {
                        world.write_message(ConversationRequest::Trade(q));
                    }
                    return;
                }
                if let Some(q) = selected {
                    select(
                        world,
                        root,
                        session,
                        q.source,
                        if change > 0 {
                            q.quantity.saturating_add(1)
                        } else {
                            q.quantity.saturating_sub(1).max(1)
                        },
                    );
                }
            });
        });
    }
    world.spawn((
        TradeList,
        TradeControls,
        Node {
            flex_grow: 1.0,
            overflow: Overflow::scroll_y(),
            flex_direction: FlexDirection::Column,
            row_gap: px(2),
            ..default()
        },
        ChildOf(root),
    ));
    world.resource_mut::<InputFocus>().set(input, FocusCause::Navigated);
    root
}
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
    let state = match &c.wait {
        ConversationWait::Price { gold, .. } => format!("Information: {gold} gold."),
        ConversationWait::Options { words, .. } => words.join(" / "),
        ConversationWait::Blocked { quote, entry, reason } => {
            format!("Content gap at quote {quote}, entry {entry:?}: {reason}")
        }
        ConversationWait::Line { .. } => "Continue to finish this line.".into(),
        _ => "Enter a topic, or select an item below.".into(),
    };
    let lines = c.transcript.last().cloned().unwrap_or_default();
    let text = format!(
        "{name} - {} gold\n{lines}\n{state}\n{}",
        world.resource::<PartyInventory>().gold,
        c.result.as_deref().unwrap_or("")
    );
    let font = world
        .resource::<AssetServer>()
        .load::<SpriteSheet>("retail://data/fonts/wiz_text_font.sti");
    let text = world.resource::<Assets<SpriteSheet>>().get(&font).map_or_else(
        || text.clone(),
        |font| super::notices::wrap(&text, 510, |s| font.text_width(s)).join("\n"),
    );
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
    let can_trade = trade_allowed(world, actor);
    for mut node in world.query_filtered::<&mut Node, With<TradeControls>>().iter_mut(world) {
        node.display = if can_trade { Display::Flex } else { Display::None };
    }
    if !can_trade {
        return;
    }
    let items = world
        .get_resource::<GameData>()
        .and_then(|d| world.resource::<Assets<ItemCatalog>>().get(&d.items));
    let Some(items) = items else { return };
    let mut rows = Vec::new();
    if let Some(npc) = world.get::<Npc>(actor) {
        for lot in &npc.stock.lots {
            if lot.units == 0 || lot.available_at != 0 {
                continue;
            }
            if let Some(d) = items.get(lot.item)
                && !d.merchant_cannot_sell
            {
                rows.push((
                    TradeSource::Stock(lot.id),
                    format!("Buy: {} ({})", d.name, lot.units),
                    if d.class == EquipClass::Ammunition { lot.units } else { 1 },
                ));
            }
        }
    }
    for item in &world.resource::<PartyInventory>().pool.stacks {
        if let Some(d) = items.get(item.item) {
            rows.push((
                TradeSource::Party(ItemRef { owner: ItemOwner::Pool, id: item.id }),
                format!("Sell from party pool: {}", item.display_name(d, true)),
                if d.class == EquipClass::Ammunition { item.units() } else { 1 },
            ));
        }
    }
    for member in &world.resource::<PartyRoster>().0 {
        if let Some(carried) = world.get::<Carried>(*member) {
            for item in carried.backpack.iter().flatten() {
                if let Some(d) = items.get(item.item) {
                    rows.push((
                        TradeSource::Party(ItemRef { owner: ItemOwner::Member(*member), id: item.id }),
                        format!(
                            "Sell from {}: {}",
                            world
                                .get::<crate::game::characters::Character>(*member)
                                .map_or("member", |c| c.name.as_str()),
                            item.display_name(d, true)
                        ),
                        if d.class == EquipClass::Ammunition { item.units() } else { 1 },
                    ));
                }
            }
        }
    }
    let view = world.get::<ConversationView>(root).unwrap();
    let offer = if let Some(err) = &view.error {
        err.clone()
    } else if let Some(q) = view.selected {
        format!(
            "{}: {} units for {} gold",
            rows.iter().find(|r| r.0 == q.source).map_or("Selection unavailable", |r| r.1.as_str()),
            q.quantity,
            q.gold
        )
    } else {
        "Select merchandise or an item to sell.".into()
    };
    let label = world
        .query_filtered::<&Children, With<Offer>>()
        .iter(world)
        .next()
        .and_then(|c| c.first().copied());
    if let Some(e) = label
        && let Some(mut t) = world.get_mut::<BitmapText>(e)
        && t.value != offer
    {
        t.value = offer;
    }
    let old: Vec<_> =
        world.query::<(Entity, &TradeRow)>().iter(world).map(|(e, r)| (e, r.0)).collect();
    for (e, source) in &old {
        if !rows.iter().any(|r| r.0 == *source) {
            world.despawn(*e);
        }
    }
    let list = world.query_filtered::<Entity, With<TradeList>>().iter(world).next().unwrap();
    for (source, text, quantity) in rows {
        if let Some((e, _)) = old.iter().find(|(_, s)| *s == source) {
            let child = world.get::<Children>(*e).and_then(|c| c.first().copied());
            if let Some(child) = child
                && let Some(mut label) = world.get_mut::<BitmapText>(child)
                && label.value != text
            {
                label.value = text;
            }
        } else {
            let e = button(world, list, &text);
            world.entity_mut(e).insert(TradeRow(source));
            world.entity_mut(e).observe(move |_: On<Activate>, mut commands: Commands| {
                commands
                    .queue(move |world: &mut World| select(world, root, session, source, quantity));
            });
        }
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
