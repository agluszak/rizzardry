//! Native dialogue widgets. All selected rows and quotes are drafts; gameplay
//! resolves item/stock identities again when a confirmation executes.
mod layout;
mod quantity;
mod speech;
mod trade_view;
mod transcript;
use super::main_screen::SelectedMember;
use super::retail_controls::{ControlSkin, FocusRing, set_enabled};
use super::screen_art::{BitmapInk, BitmapText, BitmapWrap, Picture, absolute};
use crate::content::npcs::TopicCategory;
use crate::content::{items::EquipClass, ui::SpriteSheet};
use crate::game::data::Catalogs;
use crate::game::npcs::{
    Npc,
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
use layout::{build, label, paint_wells};
use trade_view::{GiftGold, gift_amount, sync_trade_rows};
use transcript::{scroll_to_newest, sync_transcript};
#[derive(Component)]
#[relationship(relationship_target = ConversationViews)]
struct ConversationView(pub Entity);
#[derive(Component)]
#[relationship_target(relationship = ConversationView, linked_spawn)]
struct ConversationViews(Vec<Entity>);
#[derive(Component, Default)]
struct DialogueState {
    offer: Option<Result<TradeQuote, trade::TradeError>>,
    mode: Mode,
    pool: bool,
    category: Option<TopicCategory>,
    alphabetical: bool,
    item_group: Option<ItemGroup>,
    usability: Option<bool>,
}
impl DialogueState {
    fn quote(&self) -> Option<TradeQuote> {
        self.offer?.ok()
    }

    fn set_mode(&mut self, mode: Mode) {
        if self.mode != mode {
            self.mode = mode;
            self.offer = None;
        }
    }
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
struct QuoteBubble;
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
struct ChoiceOverlay;
#[derive(Component)]
struct ChoiceCaption(bool);
#[derive(Clone, PartialEq)]
enum Answer {
    Reply(String),
    Price(bool),
}
#[derive(Component, PartialEq)]
struct Choice {
    index: usize,
    answer: Answer,
}
#[derive(Component)]
pub(crate) struct TopicInput;
#[derive(Component)]
enum SubmitTopic {
    Ask,
    Edit(NotebookEdit),
}
#[derive(Component)]
struct TradeList;
#[derive(Component, Clone, Copy, PartialEq)]
struct TradeRow {
    source: TradeSource,
    quantity: u32,
}
pub struct ConversationViewPlugin;
impl Plugin for ConversationViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(speech::plugin)
            .add_observer(activate_choice)
            .add_observer(activate_trade)
            .add_observer(activate_topic)
            .add_observer(create_view)
            .add_systems(
                Update,
                (
                    close_views,
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
            .map(|_| TradeQuote { conversation: session, trade: 0, source, quantity: 1, gold: 0 })
            .ok_or(trade::TradeError::StaleSelection)
    } else {
        quote_trade(world, session, source, quantity)
    };
    let Some(mut v) = world.get_mut::<DialogueState>(view) else { return };
    v.offer = Some(result);
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
fn create_view(event: On<Add<Conversation>>, mut commands: Commands) {
    let session = event.entity;
    commands.queue(move |world: &mut World| {
        if world.get::<Conversation>(session).is_some()
            && world.get::<crate::game::npcs::conversation::Closing>(session).is_none()
        {
            build(world, session);
        }
    });
}
fn advance_bubble(
    event: On<Activate>,
    views: ViewScope,
    sessions: Query<&Conversation>,
    mut requests: MessageWriter<ConversationRequest>,
) {
    let Some((_, session)) = views.owner(event.entity) else { return };
    if let Ok(conversation) = sessions.get(session)
        && let ConversationWait::Line { speech, .. } = conversation.wait
    {
        requests.write(ConversationRequest::Advance { conversation: session, speech });
    }
}

fn project_text(
    views: ViewScope,
    sessions: Query<&Conversation>,
    speech: Query<&ConversationSpeech>,
    npcs: Query<&crate::game::npcs::ActorId>,
    catalogs: If<Res<Catalogs>>,
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
                    .and_then(|n| catalogs.npcs.get(n.npc()))
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
fn project_modes(
    views: ViewScope,
    mut commands: Commands,
    sessions: Query<&Conversation>,
    npcs: Query<(&crate::game::npcs::ActorId, &Npc)>,
    catalogs: If<Res<Catalogs>>,
    clock: Res<crate::game::time::GameClock>,
    mut panels: Query<
        (Entity, &mut Node, Option<&ModePanel>, Has<QuoteBubble>, Has<MenuPanel>),
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
        let visible = if bubble {
            state.mode == Mode::Talk && matches!(c.wait, ConversationWait::Line { .. })
        } else if menu {
            state.mode == Mode::Talk && c.menu
        } else {
            panel.is_some_and(|p| p.0 == (state.mode != Mode::Talk))
                && !(c.menu && state.mode == Mode::Talk)
        };
        crate::presentation::screen_art::display_if(&mut node, visible);
    }
    // Each approach waits half a day once tried.
    for (entity, button, disabled) in &approaches {
        let Some((_, session)) = views.owner(entity) else { continue };
        let Some((_, npc)) = sessions.get(session).ok().and_then(|c| npcs.get(c.actor).ok()) else {
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
                .and_then(|(id, _)| catalogs.npcs.get(id.npc()))
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
    requests.write(match &choice.answer {
        Answer::Reply(text) => {
            ConversationRequest::Reply { conversation: session, text: text.clone() }
        }
        Answer::Price(accept) => {
            ConversationRequest::PriceAnswer { conversation: session, accept: *accept }
        }
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
    focus: Res<InputFocus>,
    mut overlays: Query<(Entity, &mut Node), (With<ChoiceOverlay>, Without<Choices>)>,
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
        let Ok((overlay, mut overlay_node)) = overlays.get_mut(parent.parent()) else {
            continue;
        };
        let visible = !answers.is_empty();
        let was_visible = overlay_node.display != Display::None;
        if visible != was_visible {
            overlay_node.display = if visible { Display::Flex } else { Display::None };
            if visible {
                commands.entity(overlay).insert((
                    TabGroup::modal(),
                    bevy::ui_widgets::ModalDialog,
                    crate::presentation::retail_controls::RestoreFocus(focus.get()),
                ));
            } else {
                commands.entity(overlay).remove::<(
                    TabGroup,
                    bevy::ui_widgets::ModalDialog,
                    crate::presentation::retail_controls::RestoreFocus,
                )>();
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
            let value = Choice { index, answer };
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

impl SubmitTopic {
    /// Only committed editor text becomes a gameplay request. Normalization
    /// and notebook matching belong to the game.
    fn request(&self, text: &EditableText, conversation: Entity) -> Option<ConversationRequest> {
        if text.is_composing() {
            return None;
        }
        let text = text.value().to_string();
        Some(match *self {
            Self::Ask => ConversationRequest::Topic { conversation, text },
            Self::Edit(edit) => ConversationRequest::EditNotebook { conversation, edit, text },
        })
    }
}

fn submit_topic(
    views: ViewScope,
    mut commands: Commands,
    inputs: Query<(Entity, &EditableText, &SubmitTopic)>,
    sessions: Query<&Conversation>,
    mut requests: MessageWriter<ConversationRequest>,
) {
    for (entity, text, submit) in &inputs {
        if !text.pending_edits.is_empty() || text.pending_paste.is_some() {
            continue;
        }
        if let Some((_, session)) = views.owner(entity)
            && let Ok(c) = sessions.get(session)
            && let Some(request) = submit.request(text, session)
        {
            let request = match request {
                ConversationRequest::Topic { conversation, text }
                    if matches!(c.wait, ConversationWait::Options { typed: true, .. }) =>
                {
                    ConversationRequest::Reply { conversation, text }
                }
                request => request,
            };
            requests.write(request);
        }
        commands.entity(entity).remove::<SubmitTopic>();
    }
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::content::ui::TextTable;

    pub(crate) fn check_session_views(make: impl Fn(Entity) -> Conversation) {
        let mut app = crate::presentation::test_input::input_app();
        app.init_asset::<SpriteSheet>()
            .init_asset::<Image>()
            .init_asset::<Font>()
            .init_resource::<super::super::main_screen::ScreenLayout>()
            .add_observer(create_view)
            .add_message::<ConversationRequest>()
            .add_systems(Update, sync_transcript);
        let actor = app.world_mut().spawn_empty().id();
        let mut conversation = make(actor);
        conversation.transcript =
            vec![TranscriptLine::Question("Arnika".into()), TranscriptLine::Npc("A town.".into())];
        let session = app.world_mut().spawn(conversation).id();
        app.update();
        let view = app.world().get::<ConversationViews>(session).unwrap().0[0];
        let rows: Vec<_> = app
            .world_mut()
            .query::<(Entity, &TranscriptRow)>()
            .iter(app.world())
            .map(|(e, _)| e)
            .collect();
        assert_eq!(rows.len(), 2);
        for _ in 0..3 {
            app.update();
        }
        assert!(rows.iter().all(|row| app.world().get::<BitmapText>(*row).is_some()));
        assert_eq!(app.world().get::<ConversationViews>(session).unwrap().0.as_slice(), &[view]);
        app.world_mut().get_mut::<Conversation>(session).unwrap().wait =
            ConversationWait::Blocked { quote: 3, entry: None, reason: "Missing".into() };
        app.update();
        assert_eq!(app.world_mut().query::<&TranscriptRow>().iter(app.world()).count(), 3);
        app.world_mut().get_mut::<Conversation>(session).unwrap().wait = ConversationWait::Ready;
        app.update();
        assert_eq!(app.world_mut().query::<&TranscriptRow>().iter(app.world()).count(), 2);
        let bubble = app
            .world_mut()
            .query_filtered::<Entity, With<QuoteBubble>>()
            .single(app.world())
            .unwrap();
        let speech = app.world_mut().spawn_empty().id();
        app.world_mut().get_mut::<Conversation>(session).unwrap().wait =
            ConversationWait::Line { speech };
        app.world_mut().trigger(Activate { entity: bubble });
        let requests: Vec<_> =
            app.world_mut().resource_mut::<Messages<ConversationRequest>>().drain().collect();
        assert!(
            matches!(&requests[..], [ConversationRequest::Advance { conversation, speech: line }]
            if *conversation == session && *line == speech)
        );
        app.world_mut().get_mut::<Conversation>(session).unwrap().wait = ConversationWait::Ready;
        app.world_mut().trigger(Activate { entity: bubble });
        assert!(app.world().resource::<Messages<ConversationRequest>>().is_empty());
        app.world_mut().despawn(session);
        assert!(app.world().get_entity(view).is_err());
        assert!(rows.into_iter().all(|e| app.world().get_entity(e).is_err()));
        // Both operations precede the queued view build.
        app.world_mut().commands().spawn(make(actor)).despawn();
        app.update();
        assert_eq!(app.world_mut().query::<&ConversationView>().iter(app.world()).count(), 0);
    }

    #[test]
    fn notebook_input_submits_committed_text_not_ime_composition() {
        let session = Entity::from_raw_u32(42).unwrap();
        let mut text = EditableText::default();
        text.editor.set_text(" Ship ");
        for edit in [NotebookEdit::Remember, NotebookEdit::Forget] {
            let request = SubmitTopic::Edit(edit).request(&text, session).unwrap();
            assert!(matches!(request,
                ConversationRequest::EditNotebook { conversation, edit: action, text }
                if conversation == session && action == edit && text == " Ship "
            ));
        }
        let mut fonts = bevy::text::FontCx::default();
        let mut layout = bevy::text::LayoutCx::default();
        text.editor.driver(&mut fonts, &mut layout).set_compose("new", None);
        assert!(text.is_composing());
        for edit in [NotebookEdit::Remember, NotebookEdit::Forget] {
            assert!(SubmitTopic::Edit(edit).request(&text, session).is_none());
        }
    }

    #[test]
    fn session_ownership_rejects_stale_choices_and_cleans_only_its_views() {
        let mut world = World::new();
        world.init_resource::<Messages<ConversationRequest>>();
        world.add_observer(activate_choice);
        world.add_observer(activate_trade);
        let first = world.spawn_empty().id();
        let second = world.spawn_empty().id();
        let view = world.spawn((ConversationView(first), DialogueState::default())).id();
        let other = world.spawn((ConversationView(second), DialogueState::default())).id();
        let row = world
            .spawn((Button, Choice { index: 0, answer: Answer::Price(true) }, ChildOf(view)))
            .id();
        world.trigger(Activate { entity: row });
        let requests: Vec<_> =
            world.resource_mut::<Messages<ConversationRequest>>().drain().collect();
        assert!(
            matches!(&requests[..],[ConversationRequest::PriceAnswer{conversation,accept:true}] if *conversation==first)
        );
        let other_row = world
            .spawn((Button, Choice { index: 0, answer: Answer::Price(false) }, ChildOf(other)))
            .id();
        world.trigger(Activate { entity: other_row });
        let requests: Vec<_> =
            world.resource_mut::<Messages<ConversationRequest>>().drain().collect();
        assert!(matches!(&requests[..], [ConversationRequest::PriceAnswer {
            conversation, accept: false
        }] if *conversation == second));
        let trade = world
            .spawn((
                TradeRow {
                    source: TradeSource::Stock(crate::game::npcs::trade::StockLotId(1)),
                    quantity: 1,
                },
                ChildOf(other),
            ))
            .id();
        world.trigger(Activate { entity: trade });
        world.flush();
        // The stale stock is refused on its own view, never the first view.
        assert!(world.get::<DialogueState>(other).unwrap().offer.is_some_and(|o| o.is_err()));
        assert!(world.get::<DialogueState>(view).unwrap().offer.is_none());
        // A removed view's retained callback cannot route into the other session.
        world.despawn(first);
        assert!(world.get_entity(view).is_err());
        assert!(world.get_entity(row).is_err());
        assert!(world.get_entity(other).is_ok());
        world.trigger(Activate { entity: row });
        assert!(world.resource::<Messages<ConversationRequest>>().is_empty());
        // Queued trade selection retains its accepted identity and revalidates it.
        select(
            &mut world,
            other,
            first,
            TradeSource::Stock(crate::game::npcs::trade::StockLotId(1)),
            1,
        );
        assert!(world.get::<DialogueState>(other).unwrap().quote().is_none());
        world.despawn(second);
        for entity in [other, other_row, trade] {
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
            .init_resource::<crate::game::data::Catalogs>()
            .init_resource::<Assets<TextTable>>()
            .init_resource::<SelectedMember>()
            .init_resource::<InputFocus>()
            .add_message::<ConversationRequest>()
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
