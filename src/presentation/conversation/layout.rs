//! Retained retail conversation layout and its input observers.
use super::*;
use crate::content::ui::TextTable;
use bevy::ecs::observer::IntoEntityObserver;

/// What building a view needs: commands to spawn it and the assets it
/// shows.
pub(super) struct Ui<'a, 'w, 's> {
    pub commands: &'a mut Commands<'w, 's>,
    pub assets: &'a AssetServer,
}

impl Ui<'_, '_, '_> {
    fn font(&self) -> Handle<SpriteSheet> {
        self.assets.load("retail://data/fonts/wiz_text_font.sti")
    }

    fn sheet(&self, path: &str) -> Handle<SpriteSheet> {
        self.assets.load(format!("retail://data/npc interaction/{path}.sti"))
    }

    /// Text that flows in its parent's layout.
    fn bitmap(&mut self, parent: Entity, text: BitmapText) -> Entity {
        let node = Node { position_type: PositionType::Relative, flex_shrink: 0.0, ..default() };
        self.commands.spawn((text, node, Pickable::IGNORE, ChildOf(parent))).id()
    }

    /// Text placed at `at` in its parent, justified as `justify`.
    pub(super) fn text(
        &mut self,
        parent: Entity,
        value: &str,
        at: Vec2,
        width: f32,
        justify: bevy::text::Justify,
    ) -> Entity {
        let text =
            BitmapText { font: self.font(), value: value.into(), at, width, justify, ..default() };
        let node = Node { position_type: PositionType::Absolute, flex_shrink: 0.0, ..default() };
        self.commands.spawn((text, node, Pickable::IGNORE, ChildOf(parent))).id()
    }

    pub(super) fn label(&mut self, parent: Entity, value: &str, at: Vec2, width: f32) -> Entity {
        self.text(parent, value, at, width, default())
    }

    /// The speaker's name, in bold across the top of a left panel.
    fn name_caption(&mut self, panel: Entity) {
        let font = self.assets.load("retail://data/fonts/wiz_text_font_bold.sti");
        let (at, width) = (Vec2::new(5., 2.), 132.);
        let text =
            BitmapText { font, at, width, justify: bevy::text::Justify::Center, ..default() };
        let node = Node { position_type: PositionType::Absolute, flex_shrink: 0.0, ..default() };
        self.commands.spawn((text, node, Caption::Name, Pickable::IGNORE, ChildOf(panel)));
    }

    fn picture(&mut self, parent: Entity, path: &str, frame: usize, at: Vec2) -> Entity {
        let sheet = self.sheet(path);
        self.commands.spawn((Picture { sheet, frame, at, bounds: None }, ChildOf(parent))).id()
    }

    fn panel(&mut self, parent: Entity, at: Vec2, size: Vec2, frame: usize) -> Entity {
        let e = self
            .commands
            .spawn((absolute(at, size), BackgroundColor(Color::BLACK), ChildOf(parent)))
            .id();
        self.picture(e, "npc_bottompanel", frame, Vec2::ZERO);
        e
    }

    /// A button drawn by its sprite sheet's frames from `first`.
    fn sprite_button(&mut self, parent: Entity, path: &str, first: usize, at: Vec2) -> Entity {
        let sheet = self.sheet(path);
        self.commands
            .spawn((
                Button,
                FocusRing,
                TabIndex(0),
                Pickable::default(),
                Picture { sheet, frame: first, at, bounds: Some(first) },
                ControlSkin::new(first, first + 1, first + 4, first + 2, first + 3),
                ChildOf(parent),
            ))
            .id()
    }

    /// A text button over the panel artwork, and its caption.
    fn action(&mut self, parent: Entity, text: &str, at: Vec2, size: Vec2) -> (Entity, Entity) {
        let mut node = absolute(at, size);
        node.justify_content = JustifyContent::Center;
        node.align_items = AlignItems::Center;
        // Normal button wells are already in the panel artwork; avoid scaling
        // a pressed-state sprite into an unrelated control's hit rectangle.
        let e = self
            .commands
            .spawn((
                Button,
                FocusRing,
                TabIndex(0),
                node,
                bevy::picking::hover::Hovered::default(),
                ChildOf(parent),
            ))
            .id();
        let font = self.font();
        let caption = self.bitmap(e, BitmapText { font, value: text.into(), ..default() });
        self.commands.entity(caption).insert(ZIndex(1));
        let frames = if size == Vec2::new(67., 16.) {
            Some((10, 12, 11))
        } else if size == Vec2::new(123., 16.) {
            Some((14, 15, 15))
        } else {
            None
        };
        if let Some((hover, pressed, pressed_hover)) = frames {
            let child = self.picture(e, "npc_bottompanel", usize::MAX, Vec2::ZERO);
            self.commands.entity(child).insert(WellChrome { size, hover, pressed, pressed_hover });
        }
        (e, caption)
    }

    fn scroll_arrows(&mut self, parent: Entity, target: Entity, at: Vec2, gap: f32) {
        for (dy, step, first) in [(0., -1., 20), (gap, 1., 25)] {
            let e = self.sprite_button(parent, "npc_buttons", first, at + Vec2::Y * dy);
            self.commands.entity(e).observe(
                move |_: On<Activate>, mut scrolls: Query<&mut ScrollPosition>| {
                    if let Ok(mut scroll) = scrolls.get_mut(target) {
                        scroll.0.y = (scroll.0.y + step * 14.).max(0.);
                    }
                },
            );
        }
    }

    /// A text button with a radio mark, its caption after the mark.
    fn radio(&mut self, parent: Entity, text: &str, at: Vec2, size: Vec2) -> Entity {
        let (e, _) = self.action(parent, text, at, size);
        let mut node = absolute(at, size);
        node.align_items = AlignItems::Center;
        node.justify_content = JustifyContent::Start;
        node.padding.left = px(10);
        self.commands.entity(e).insert(node);
        let image = self.picture(e, "npc_bottompanel", 8, Vec2::new(0., 2.));
        self.commands.entity(image).insert(RadioMark);
        e
    }

    /// An input of retail's text font.
    fn input(&self, max_characters: usize, node: Node) -> impl Bundle + use<> {
        (
            TextInput,
            EditableText {
                max_characters: Some(max_characters),
                allow_newlines: false,
                ..EditableText::new("")
            },
            TextFont {
                font: self.assets.load("retail://data/fonts/wiz_text_font.sti#font").into(),
                font_size: 11.0.into(),
                ..default()
            },
            TextColor(Color::WHITE),
            TabIndex(0),
            node,
        )
    }

    pub(super) fn observe<M>(&mut self, entity: Entity, observer: impl IntoEntityObserver<M>) {
        self.commands.entity(entity).observe(observer);
    }

    fn disable(&mut self, entity: Entity) {
        self.commands.entity(entity).insert((InteractionDisabled, TabIndex(-1)));
    }
}
pub(super) fn paint_wells(
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
fn open_trade(ui: &mut Ui, button: Entity, root: Entity) {
    ui.observe(
        button,
        move |_: On<Activate>,
              mut states: Query<&mut DialogueState>,
              roots: Query<&ConversationView>,
              sessions: Query<&Conversation>,
              npcs: Query<&crate::game::npcs::ActorId>,
              catalogs: Res<Catalogs>| {
            let c = sessions.get(roots.get(root).unwrap().0).unwrap();
            let merchant = catalogs.npcs[npcs.get(c.actor).unwrap().npc()].owns_stock;
            states.get_mut(root).unwrap().set_mode(if merchant { Mode::Buy } else { Mode::Give });
        },
    );
}
/// Activating `button` edits its view's dialogue state.
fn edit_state(
    ui: &mut Ui,
    button: Entity,
    root: Entity,
    edit: impl Fn(&mut DialogueState) + Send + Sync + 'static,
) {
    ui.observe(button, move |_: On<Activate>, mut states: Query<&mut DialogueState>| {
        if let Ok(mut state) = states.get_mut(root) {
            edit(&mut state);
        }
    });
}
/// Activating `button` asks this of its view's conversation.
fn request_on(ui: &mut Ui, button: Entity, root: Entity, talk: Talk) {
    ui.observe(
        button,
        move |_: On<Activate>,
              roots: Query<&ConversationView>,
              mut requests: MessageWriter<ConversationRequest>| {
            if let Ok(view) = roots.get(root) {
                requests.write(ConversationRequest::new(view.0, talk.clone()));
            }
        },
    );
}
/// The left panel's six actions, two to a row.
fn left_action(ui: &mut Ui, left: Entity, i: usize, text: &str) -> Entity {
    let at = Vec2::new(2. + 66. * (i % 2) as f32, 17. + 16. * (i / 2) as f32);
    ui.action(left, text, at, Vec2::new(67., 16.)).0
}
/// Retail `ShowNpcDialogueTopicMenu`: an NPC not yet friendly may be
/// charmed, talked to or bribed before it talks of anything else.
fn build_menu(ui: &mut Ui, strings: Option<&TextTable>, canvas: Entity, root: Entity) {
    let group =
        ui.commands.spawn((Node::default(), MenuPanel, Pickable::IGNORE, ChildOf(canvas))).id();
    let left = ui.panel(group, Vec2::new(23., 358.), Vec2::new(141., 92.), 0);
    ui.name_caption(left);
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
        let e = left_action(ui, left, i, &text);
        match i {
            0 | 1 => {
                let approach = if i == 0 { Approach::Charm } else { Approach::SmallTalk };
                ui.commands.entity(e).insert(ApproachButton(approach));
                request_on(ui, e, root, Talk::Approach { approach });
            }
            // Retail's bribe opens the give layout.
            2 => edit_state(ui, e, root, move |state| state.set_mode(Mode::Give)),
            4 => open_trade(ui, e, root),
            5 => request_on(ui, e, root, Talk::Close),
            _ => ui.disable(e),
        }
    }
}
/// The left panel: the speaker's name and the mode's actions, and in trade
/// the item filters and the way back.
fn build_actions(ui: &mut Ui, group: Entity, root: Entity, trade: bool) {
    let left =
        ui.panel(group, Vec2::new(23., 358.), Vec2::new(141., 92.), if trade { 17 } else { 0 });
    ui.name_caption(left);
    let labels = if trade {
        ["Buy", "Sell", "Give", "Steal", "Party Items", "PC Items"]
    } else {
        ["Trade", "Pickpocket", "Magic", "Recruit", "Attack", "Leave"]
    };
    for (i, text) in labels.into_iter().enumerate() {
        let e = left_action(ui, left, i, text);
        match (trade, i) {
            (false, 0) => open_trade(ui, e, root),
            (true, 0..=2) => {
                let mode = [Mode::Buy, Mode::Sell, Mode::Give][i];
                ui.commands.entity(e).insert(ModeButton(mode));
                if mode != Mode::Give {
                    ui.commands.entity(e).insert(TradeAvailable);
                }
                edit_state(ui, e, root, move |state| state.set_mode(mode));
            }
            (false, 2) => ui.observe(e, move |_: On<Activate>, roots: Query<&ConversationView>, conversations: Query<&Conversation>,
                selected: Res<crate::presentation::main_screen::SelectedMember>, roster: Single<&crate::game::party::roster::PartyRoster>,
                art: Res<crate::presentation::main_screen::magic_view::MagicViewArt>, mut commands: Commands| {
                if let Ok(view) = roots.get(root) && conversations.get(view.0).is_ok_and(|c| c.ready()) && let Some(caster) = selected.member(&roster) {
                    crate::presentation::main_screen::magic_view::open(&mut commands, &art, caster, false, Some(crate::presentation::main_screen::magic_view::ViewTarget::Conversation(view.0)));
                }
            }),
            (false, 3) => request_on(ui, e, root, Talk::Recruit),
            (false, 5) => request_on(ui, e, root, Talk::Close),
            (true, 4..=5) => {
                let pool = i == 4;
                ui.commands.entity(e).insert(PoolButton(pool));
                edit_state(ui, e, root, move |state| {
                    if state.pool != pool {
                        state.pool = pool;
                        state.offer = None;
                    }
                });
            }
            _ => ui.disable(e),
        }
    }
    if !trade {
        return;
    }
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
        let e = ui.sprite_button(left, "npc_itemfilters", first, Vec2::new(x, 70.));
        ui.commands
            .entity(e)
            .insert((filter, ControlSkin::new(first, first + 2, first + 1, first + 4, first + 3)));
        edit_state(ui, e, root, move |state| {
            match filter {
                ItemFilter::Group(group) => {
                    state.item_group =
                        if state.item_group == Some(group) { None } else { Some(group) }
                }
                ItemFilter::Usable(party) => {
                    state.usability =
                        if state.usability == Some(party) { None } else { Some(party) }
                }
            }
            state.offer = None;
        });
    }
    let back = ui.sprite_button(left, "npc_buttons", 0, Vec2::new(114., 71.));
    ui.commands.entity(back).insert(ModeFocus(true));
    edit_state(ui, back, root, move |state| state.set_mode(Mode::Talk));
}
/// The trade layout's right panel: the offer, its price and count, the gift
/// of gold and the confirmation.
fn build_offer(ui: &mut Ui, group: Entity, root: Entity, canvas: Entity) {
    use bevy::text::Justify;
    let right = ui.panel(group, Vec2::new(476., 358.), Vec2::new(141., 92.), 6);
    let offer = ui.text(right, "", Vec2::new(4., 4.), 132., Justify::Center);
    ui.commands.entity(offer).insert(Offer);
    ui.commands.spawn((ItemPreview, Picture::default(), Visibility::Hidden, ChildOf(right)));
    let count = ui.text(right, "", Vec2::new(5., 62.), 45., Justify::Right);
    ui.commands.entity(count).insert(TradeCount);
    let price = ui.text(right, "", Vec2::new(83., 58.), 51., Justify::Right);
    ui.commands.entity(price).insert(TradePrice);
    let quantity = ui.sprite_button(right, "npc_buttons", 30, Vec2::new(53., 54.));
    ui.commands.entity(quantity).insert(QuantityControl { view: root, canvas });
    ui.observe(quantity, quantity::open);
    // Retail's Give mode also gives gold: the amount typed here when
    // no item is chosen.
    let input = ui.input(
        9,
        Node { display: Display::None, ..absolute(Vec2::new(83., 58.), Vec2::new(51., 12.)) },
    );
    let gift_gold = ui
        .commands
        .spawn((
            GiftGold,
            input,
            bevy::text::EditableTextFilter::new(|c| c.is_ascii_digit()),
            ChildOf(right),
        ))
        .id();
    let (confirm, caption) = ui.action(right, "", Vec2::new(56., 34.), Vec2::new(80., 18.));
    ui.commands.entity(confirm).insert(Confirm);
    ui.commands.entity(caption).insert(Caption::TradeAction);
    ui.observe(
        confirm,
        move |_: On<Activate>,
              mut states: Query<&mut DialogueState>,
              roots: Query<&ConversationView>,
              gold: Query<&EditableText, With<GiftGold>>,
              mut requests: MessageWriter<ConversationRequest>,
              mut trades: MessageWriter<crate::game::npcs::conversation::TradeRequest>| {
            let Ok(mut state) = states.get_mut(root) else { return };
            let amount = gold.get(gift_gold).map_or(0, gift_amount);
            if state.mode == Mode::Give
                && state.quote().is_none()
                && amount > 0
                && let Ok(view) = roots.get(root)
            {
                requests.write(ConversationRequest::new(view.0, Talk::GiveGold { amount }));
                state.set_mode(Mode::Talk);
            } else if let Some(q) = state.quote()
                && roots.get(root).is_ok_and(|v| v.0 == q.conversation)
            {
                if state.mode != Mode::Give {
                    trades.write(crate::game::npcs::conversation::TradeRequest(q));
                    // The trade settles the selection.
                    state.offer = None;
                } else if let TradeSource::Party(item) = q.source {
                    requests.write(ConversationRequest::new(q.conversation, Talk::Give { item }));
                    // Retail returns to the transcript for the answer.
                    state.set_mode(Mode::Talk);
                }
            }
        },
    );
    ui.label(right, "Party gold:", Vec2::new(5., 77.), 76.);
    let gold = ui.text(right, "", Vec2::new(83., 77.), 51., Justify::Right);
    ui.commands.entity(gold).insert(Caption::Gold);
}
/// The talk layout's topics, their filters and the question input. Returns
/// the input, which takes the focus.
fn build_topics(ui: &mut Ui, group: Entity, root: Entity) -> Entity {
    let topics = ui
        .commands
        .spawn((
            absolute(Vec2::new(476., 283.), Vec2::new(141., 18.)),
            BackgroundColor(Color::BLACK),
            TopicList,
            ChildOf(group),
        ))
        .id();
    let rows = ui
        .commands
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
            TopicRows,
            ZIndex(1),
            ChildOf(topics),
        ))
        .id();
    ui.scroll_arrows(topics, rows, Vec2::new(126., 3.), 8.);
    let filters = ui.panel(group, Vec2::new(476., 303.), Vec2::new(141., 55.), 5);
    let sort = ui.radio(filters, "Sort Alphabetically", Vec2::new(6., 2.), Vec2::new(130., 12.));
    ui.commands.entity(sort).insert(SortToggle);
    edit_state(ui, sort, root, move |state| state.alphabetical = !state.alphabetical);
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
        let at = Vec2::new(6. + 61. * (i % 2) as f32, 15. + 12. * (i / 2) as f32);
        let e = ui.radio(filters, text, at, Vec2::new(61., 12.));
        ui.commands.entity(e).insert(CategoryButton(category));
        edit_state(ui, e, root, move |state| state.category = category);
    }
    let input_panel = ui.panel(group, Vec2::new(476., 358.), Vec2::new(141., 92.), 13);
    let field = ui.input(128, absolute(Vec2::new(9., 8.), Vec2::new(122., 20.)));
    let input = ui.commands.spawn((TopicInput, ModeFocus(false), field, ChildOf(input_panel))).id();
    ui.observe(input, |mut e: On<FocusedInput<KeyboardInput>>, mut commands: Commands| {
        if e.input.state.is_pressed() && e.input.logical_key == Key::Enter {
            e.propagate(false);
            commands.entity(e.original_event_target()).insert(SubmitTopic::Ask);
        }
    });
    // Remembering and forgetting a topic are the sprite buttons.
    for (first, at, edit) in [
        (5, Vec2::new(17., 35.), NotebookEdit::Remember),
        (10, Vec2::new(38., 35.), NotebookEdit::Forget),
    ] {
        let e = ui.sprite_button(input_panel, "npc_buttons", first, at);
        ui.observe(e, move |_: On<Activate>, mut commands: Commands| {
            commands.entity(input).insert(SubmitTopic::Edit(edit));
        });
    }
    let (ask, _) = ui.action(input_panel, "Talk About", Vec2::new(9., 51.), Vec2::new(123., 16.));
    ui.observe(ask, move |_: On<Activate>, mut commands: Commands| {
        commands.entity(input).insert(SubmitTopic::Ask);
    });
    let (where_is, _) =
        ui.action(input_panel, "Where Is", Vec2::new(9., 67.), Vec2::new(123., 16.));
    ui.disable(where_is);
    input
}
/// The view of a conversation (retail `OpenNpcDialoguePanel` and
/// `CreateNpcDialogueControls`): the NPC interface replaces the lower
/// panels, not the world view; both its layouts stay alive as native UI.
/// Returns the view and the input that takes the focus.
pub(super) fn build(ui: &mut Ui, strings: Option<&TextTable>, session: Entity) -> (Entity, Entity) {
    let root = ui
        .commands
        .spawn((
            ConversationView(session),
            DialogueState {
                offer: None,
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
    ui.observe(
        root,
        move |mut event: On<FocusedInput<KeyboardInput>>,
              mut states: Query<(&ConversationView, &mut DialogueState)>,
              mut requests: MessageWriter<ConversationRequest>| {
            if event.input.state.is_pressed() && event.input.logical_key == Key::Escape {
                event.propagate(false);
                let Ok((view, mut state)) = states.get_mut(root) else { return };
                if state.mode != Mode::Talk {
                    state.set_mode(Mode::Talk);
                } else {
                    requests.write(ConversationRequest::new(view.0, Talk::Close));
                }
            }
        },
    );
    let canvas = ui
        .commands
        .spawn((
            Node { width: px(640), height: px(480), ..default() },
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .id();
    let mut focus = root;
    for trade in [false, true] {
        let group = ui
            .commands
            .spawn((Node::default(), ModePanel(trade), Pickable::IGNORE, ChildOf(canvas)))
            .id();
        build_actions(ui, group, root, trade);
        let centre = ui.panel(group, Vec2::new(164., 358.), Vec2::new(312., 92.), 1);
        let node = Node {
            position_type: PositionType::Absolute,
            left: px(4),
            top: px(8),
            width: px(284),
            height: px(76),
            flex_direction: FlexDirection::Column,
            overflow: Overflow::scroll_y(),
            ..default()
        };
        let list = ui.commands.spawn((ScrollArea, node, ChildOf(centre))).id();
        ui.scroll_arrows(centre, list, Vec2::new(298., 4.), 71.);
        if trade {
            ui.commands.entity(list).insert(TradeList);
            build_offer(ui, group, root, canvas);
        } else {
            ui.commands.entity(list).insert(Summary);
            build_choices(ui, group);
            focus = build_topics(ui, group, root);
        }
    }
    build_menu(ui, strings, canvas, root);
    let bubble = ui
        .commands
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
            ImageNode {
                image: ui.assets.load("retail://data/npc interaction/npc_popup_back.pcx"),
                rect: Some(Rect::new(0., 0., 280., 180.)),
                ..default()
            },
            ChildOf(canvas),
        ))
        .id();
    let font = ui.assets.load("retail://data/fonts/font12point1.sti");
    let text = ui.bitmap(bubble, BitmapText { font, width: 256., ..default() });
    ui.commands.entity(text).insert((
        Caption::Quote,
        BitmapWrap { width: 256, line_spacing: 2 },
        BitmapInk { foreground: 208, shadow: None },
    ));
    ui.observe(bubble, advance_bubble);
    (root, focus)
}
/// The dialog of choices and prices, over the talk layout: retail option
/// and price dialogs require an answer, so Escape must not bubble through
/// and close the conversation underneath them.
fn build_choices(ui: &mut Ui, group: Entity) {
    let overlay = ui
        .commands
        .spawn((
            ChoiceOverlay,
            Node { display: Display::None, ..absolute(Vec2::ZERO, Vec2::new(640., 480.)) },
            GlobalZIndex(100),
            Pickable::default(),
            ChildOf(group),
        ))
        .id();
    ui.observe(overlay, |mut event: On<FocusedInput<KeyboardInput>>| {
        if event.input.logical_key == Key::Escape {
            event.propagate(false);
        }
    });
    let font = ui.assets.load("retail://data/fonts/wiz_text_font_monopalette.sti");
    let choices = ui
        .commands
        .spawn((
            Choices { font },
            absolute(Vec2::new(220., 184.), Vec2::new(200., 70.)),
            ChildOf(overlay),
        ))
        .id();
    crate::presentation::screen_art::spawn_dialog_frame(ui.commands, ui.assets, choices, false);
    for purse in [false, true] {
        let at = Vec2::new(10., if purse { 55. } else { 10. });
        let text = ui.text(choices, "", at, 180., bevy::text::Justify::Center);
        ui.commands.entity(text).insert(ChoiceCaption(purse));
    }
}
