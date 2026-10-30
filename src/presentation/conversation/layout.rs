//! Retained retail conversation layout and its input observers.
use super::*;

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
pub(super) fn label(world: &mut World, parent: Entity, text: &str, at: Vec2, width: f32) -> Entity {
    let e = bitmap(world, parent, text.into(), width);
    world.get_mut::<BitmapText>(e).unwrap().at = at;
    world.get_mut::<Node>(e).unwrap().position_type = PositionType::Absolute;
    e
}
/// The speaker's name, in bold across the top of a left panel.
fn name_caption(world: &mut World, panel: Entity) {
    let name = label(world, panel, "", Vec2::new(5., 2.), 132.);
    let bold = world.resource::<AssetServer>().load("retail://data/fonts/wiz_text_font_bold.sti");
    let mut text = world.get_mut::<BitmapText>(name).unwrap();
    text.justify = bevy::text::Justify::Center;
    text.font = bold;
    world.entity_mut(name).insert(Caption::Name);
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
fn open_trade(world: &mut World, button: Entity, root: Entity) {
    world.entity_mut(button).observe(
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
    world: &mut World,
    button: Entity,
    root: Entity,
    edit: impl Fn(&mut DialogueState) + Send + Sync + 'static,
) {
    world.entity_mut(button).observe(
        move |_: On<Activate>, mut states: Query<&mut DialogueState>| {
            if let Ok(mut state) = states.get_mut(root) {
                edit(&mut state);
            }
        },
    );
}
/// Activating `button` asks this of its view's conversation.
fn request_on(
    world: &mut World,
    button: Entity,
    root: Entity,
    request: impl Fn(Entity) -> ConversationRequest + Send + Sync + 'static,
) {
    world.entity_mut(button).observe(
        move |_: On<Activate>,
              roots: Query<&ConversationView>,
              mut requests: MessageWriter<ConversationRequest>| {
            if let Ok(view) = roots.get(root) {
                requests.write(request(view.0));
            }
        },
    );
}
fn leave_on(world: &mut World, button: Entity, root: Entity) {
    request_on(world, button, root, |conversation| ConversationRequest::Close { conversation });
}
/// Retail `ShowNpcDialogueTopicMenu`: an NPC not yet friendly may be
/// charmed, talked to or bribed before it talks of anything else.
fn build_menu(world: &mut World, canvas: Entity, root: Entity) {
    let group = world.spawn((Node::default(), MenuPanel, Pickable::IGNORE, ChildOf(canvas))).id();
    let left = panel(world, group, Vec2::new(23., 358.), Vec2::new(141., 92.), 0);
    name_caption(world, left);
    let strings = world
        .get_resource::<crate::presentation::screen_art::StringTable>()
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
                world.entity_mut(e).insert(ApproachButton(approach));
                request_on(world, e, root, move |conversation| ConversationRequest::Approach {
                    conversation,
                    approach,
                });
            }
            // Retail's bribe opens the give layout.
            2 => {
                edit_state(world, e, root, move |state| {
                    state.set_mode(Mode::Give);
                });
            }
            4 => open_trade(world, e, root),
            5 => leave_on(world, e, root),
            _ => {
                world.entity_mut(e).insert(InteractionDisabled);
            }
        }
    }
}
/// The left panel: the speaker's name and the mode's actions, and in trade
/// the item filters and the way back.
fn build_actions(world: &mut World, group: Entity, root: Entity, trade: bool) {
    let left =
        panel(world, group, Vec2::new(23., 358.), Vec2::new(141., 92.), if trade { 17 } else { 0 });
    name_caption(world, left);
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
                edit_state(world, e, root, move |state| {
                    state.set_mode(mode);
                });
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
            (false, 3) => {
                request_on(world, e, root, |conversation| ConversationRequest::Recruit {
                    conversation,
                });
            }
            (false, 5) => leave_on(world, e, root),
            (true, 4..=5) => {
                let pool = i == 4;
                world.entity_mut(e).insert(PoolButton(pool));
                edit_state(world, e, root, move |state| {
                    if state.pool != pool {
                        state.pool = pool;
                        state.offer = None;
                    }
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
                .observe(move |_: On<Activate>, mut states: Query<&mut DialogueState>| {
                    if let Ok(mut state) = states.get_mut(root) {
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
                    }
                });
        }
        let back = action(world, left, "Back", Vec2::new(114., 71.), Vec2::new(23., 16.));
        let children: Vec<_> = world.get::<Children>(back).into_iter().flatten().copied().collect();
        for child in children {
            world.despawn(child);
        }
        let sheet =
            world.resource::<AssetServer>().load("retail://data/npc interaction/npc_buttons.sti");
        world.entity_mut(back).insert((
            Picture { sheet, frame: 0, at: Vec2::new(114., 71.), bounds: Some(0) },
            ControlSkin::new(0, 1, 4, 2, 3),
            Pickable::default(),
        ));
        world.entity_mut(back).insert(ModeFocus(true));
        edit_state(world, back, root, move |state| {
            state.set_mode(Mode::Talk);
        });
    }
}
/// The trade layout's right panel: the offer, its price and count, the gift
/// of gold and the confirmation.
fn build_offer(world: &mut World, group: Entity, root: Entity, canvas: Entity) {
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
            Node { display: Display::None, ..absolute(Vec2::new(83., 58.), Vec2::new(51., 12.)) },
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
              gold: Query<&EditableText, With<GiftGold>>,
              mut requests: MessageWriter<ConversationRequest>| {
            let Ok(mut state) = states.get_mut(root) else { return };
            let amount = gold.get(gift_gold).map_or(0, gift_amount);
            if state.mode == Mode::Give
                && state.quote().is_none()
                && amount > 0
                && let Ok(view) = roots.get(root)
            {
                requests.write(ConversationRequest::GiveGold { conversation: view.0, amount });
                state.set_mode(Mode::Talk);
            } else if let Some(q) = state.quote()
                && roots.get(root).is_ok_and(|v| v.0 == q.conversation)
            {
                if state.mode != Mode::Give {
                    requests.write(ConversationRequest::Trade(q));
                    // The trade settles the selection.
                    state.offer = None;
                } else if let TradeSource::Party(item) = q.source {
                    requests
                        .write(ConversationRequest::Give { conversation: q.conversation, item });
                    // Retail returns to the transcript for the answer.
                    state.set_mode(Mode::Talk);
                }
            }
        },
    );
    label(world, right, "Party gold:", Vec2::new(5., 77.), 76.);
    let gold = label(world, right, "", Vec2::new(83., 77.), 51.);
    world.entity_mut(gold).insert(Caption::Gold);
    world.get_mut::<BitmapText>(gold).unwrap().justify = bevy::text::Justify::Right;
}
/// The talk layout's topics, their filters and the question input.
fn build_topics(world: &mut World, group: Entity, root: Entity) {
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
    let sort =
        action(world, filters, "Sort Alphabetically", Vec2::new(6., 2.), Vec2::new(130., 12.));
    add_radio(world, sort);
    world.entity_mut(sort).insert(SortToggle);
    edit_state(world, sort, root, move |state| {
        state.alphabetical = !state.alphabetical;
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
        add_radio(world, e);
        edit_state(world, e, root, move |state| state.category = category);
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
                commands.entity(e.original_event_target()).insert(SubmitTopic::Ask);
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
        let e =
            action(world, input_panel, text, at, Vec2::new(if i < 2 { 17. } else { 123. }, 16.));
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
                world.entity_mut(e).observe(move |_: On<Activate>, mut commands: Commands| {
                    commands.entity(input).insert(SubmitTopic::Ask);
                });
            }
            0 | 1 => {
                world.entity_mut(e).observe(move |_: On<Activate>, mut commands: Commands| {
                    let edit = if i == 1 { NotebookEdit::Forget } else { NotebookEdit::Remember };
                    commands.entity(input).insert(SubmitTopic::Edit(edit));
                });
            }
            _ => {
                world.entity_mut(e).insert(InteractionDisabled);
            }
        }
    }
    world.resource_mut::<InputFocus>().set(input, FocusCause::Navigated);
}
pub(super) fn build(world: &mut World, session: Entity) -> Entity {
    // OpenNpcDialoguePanel selects portrait mode before building its controls.
    world.resource_mut::<crate::presentation::main_screen::ScreenLayout>().mode =
        crate::presentation::main_screen::LayoutMode::Closed;
    let root = world
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
    world.entity_mut(root).observe(
        move |mut event: On<FocusedInput<KeyboardInput>>,
              mut states: Query<(&ConversationView, &mut DialogueState)>,
              mut requests: MessageWriter<ConversationRequest>| {
            if event.input.state.is_pressed() && event.input.logical_key == Key::Escape {
                event.propagate(false);
                let Ok((view, mut state)) = states.get_mut(root) else { return };
                if state.mode != Mode::Talk {
                    state.set_mode(Mode::Talk);
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
        build_actions(world, group, root, trade);
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
                    ChoiceOverlay,
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
            crate::presentation::screen_art::spawn_dialog_frame(
                &mut world.commands(),
                &assets,
                choices,
                false,
            );
            for purse in [false, true] {
                let text =
                    label(world, choices, "", Vec2::new(10., if purse { 55. } else { 10. }), 180.);
                world.entity_mut(text).insert(ChoiceCaption(purse));
                world.get_mut::<BitmapText>(text).unwrap().justify = bevy::text::Justify::Center;
            }
        }
        if trade {
            build_offer(world, group, root, canvas);
        } else {
            build_topics(world, group, root);
        }
    }
    build_menu(world, canvas, root);
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
    world.entity_mut(text).insert((
        Caption::Quote,
        BitmapWrap { width: 256, line_spacing: 2 },
        BitmapInk { foreground: 208, shadow: None },
    ));
    world.entity_mut(bubble).observe(advance_bubble);
    root
}
