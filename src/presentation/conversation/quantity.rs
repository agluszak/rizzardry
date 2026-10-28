//! Retail split-item presentation. This edits a quote, never inventory itself.
use super::*;
use bevy::text::{EditableTextFilter, TextEdit};
use bevy::ui_widgets::ModalDialog;

#[derive(Component)]
pub(super) struct Draft {
    view: Entity,
    quote: TradeQuote,
    maximum: u32,
    unit_weight: u16,
    remaining_price: u32,
    previous_focus: Option<Entity>,
}
#[derive(Component)]
pub(super) struct Count;
#[derive(Component)]
pub(super) struct Remaining;
#[derive(Component)]
pub(super) struct Price;
#[derive(Component)]
pub(super) struct RemainingPrice;
#[derive(Component)]
pub(super) struct Weight(bool);

fn stack(world: &mut World, q: TradeQuote) -> Option<(crate::content::items::ItemId, u32)> {
    let c = world.get::<Conversation>(q.conversation)?;
    match q.source {
        TradeSource::Stock(id) => world
            .get::<Npc>(c.actor)?
            .stock
            .lots
            .iter()
            .find(|lot| lot.id == id)
            .map(|lot| (lot.item, lot.units)),
        TradeSource::Party(item) => match item.owner {
            ItemOwner::Pool => {
                let party = crate::game::party::roster::the_party(world)?;
                world
                    .get::<PartyInventory>(party)?
                    .pool
                    .stacks
                    .iter()
                    .find(|stack| stack.id == item.id)
                    .map(|s| (s.item, s.units()))
            }
            ItemOwner::Member(member) => {
                world.get::<Carried>(member)?.find_stack(item.id).map(|s| (s.item, s.units()))
            }
        },
    }
}

pub(super) fn open(world: &mut World, view: Entity, canvas: Entity) {
    if world.query::<&Draft>().iter(world).any(|d| d.view == view) {
        return;
    }
    let Some(q) = world.get::<DialogueState>(view).and_then(|s| s.selected) else { return };
    if world.get::<ConversationView>(view).is_none_or(|v| v.0 != q.conversation)
        || world.get::<Conversation>(q.conversation).is_none_or(|c| c.revision != q.revision)
        || quote_trade(world, q.conversation, q.source, q.quantity).is_err()
    {
        return;
    }
    let Some((item, maximum)) = stack(world, q) else { return };
    let Some(items) = world.get_resource::<GameData>().map(|d| d.items.clone()) else { return };
    let Some(item) = world.resource::<Assets<ItemCatalog>>().get(&items).and_then(|c| c.get(item))
    else {
        return;
    };
    if maximum < 2 || item.class == EquipClass::Ammunition {
        return;
    }
    let (name, image, unit_weight, class, generic) = (
        item.name.clone(),
        item.image.clone(),
        item.weight,
        item.class,
        item.unidentified_name.clone(),
    );
    let remaining_price = if maximum == q.quantity {
        0
    } else {
        let Ok(remaining) = quote_trade(world, q.conversation, q.source, maximum - q.quantity)
        else {
            return;
        };
        remaining.gold
    };
    let assets = world.resource::<AssetServer>().clone();
    let previous_focus = world.resource::<InputFocus>().get();
    let overlay = world
        .spawn((
            absolute(Vec2::ZERO, Vec2::new(640., 480.)),
            GlobalZIndex(100),
            Pickable::default(),
            TabGroup::modal(),
            ModalDialog,
            ChildOf(canvas),
        ))
        .id();
    let panel = world
        .spawn((
            Draft { view, quote: q, maximum, unit_weight, remaining_price, previous_focus },
            absolute(Vec2::new(161., 148.), Vec2::new(322., 189.)),
            ChildOf(overlay),
        ))
        .id();
    world.spawn((
        Picture {
            sheet: assets.load("retail://data/dialogs/popup_splititem.sti"),
            frame: 0,
            at: Vec2::ZERO,
            bounds: None,
        },
        ChildOf(panel),
    ));
    world.spawn((
        Picture {
            sheet: assets.load(crate::content::paths::retail(&image)),
            frame: 0,
            at: Vec2::new(24., 14.),
            bounds: None,
        },
        ChildOf(panel),
    ));
    label(world, panel, &name, Vec2::new(72., 15.), 226.);
    let strings = assets.load::<TextTable>("retail://data/strings/stringdata.dat");
    let class_name = world
        .resource::<Assets<TextTable>>()
        .get(&strings)
        .and_then(|s| s.get(1087 + u8::from(class) as usize))
        .unwrap_or("")
        .to_owned();
    label(world, panel, "Type", Vec2::new(72., 35.), 53.);
    label(world, panel, &format!("{class_name} ({generic})"), Vec2::new(129., 35.), 169.);
    for (y, remaining) in [(53., true), (131., false)] {
        label(world, panel, "Weight", Vec2::new(72., y), 53.);
        let weight = label(world, panel, "", Vec2::new(129., y), 54.);
        world.entity_mut(weight).insert(Weight(remaining));
    }
    label(world, panel, "Price", Vec2::new(187., 53.), 53.);
    let remaining_price_label = label(world, panel, "", Vec2::new(244., 53.), 54.);
    world.entity_mut(remaining_price_label).insert(RemainingPrice);
    let buying = matches!(q.source, TradeSource::Stock(_));
    label(world, panel, if buying { "Available" } else { "Remaining" }, Vec2::new(72., 73.), 110.);
    label(world, panel, if buying { "Buy" } else { "Sell" }, Vec2::new(72., 111.), 110.);
    label(world, panel, "Price", Vec2::new(187., 131.), 53.);
    let remaining = label(world, panel, "", Vec2::new(188., 73.), 51.);
    world.entity_mut(remaining).insert(Remaining);
    let price = label(world, panel, "", Vec2::new(244., 131.), 54.);
    world.entity_mut(price).insert(Price);
    for at in [
        Vec2::new(188., 73.),
        Vec2::new(188., 111.),
        Vec2::new(244., 131.),
        Vec2::new(244., 53.),
        Vec2::new(129., 53.),
        Vec2::new(129., 131.),
    ] {
        world.spawn((
            Picture {
                sheet: assets.load("retail://data/dialogs/popup_splititem.sti"),
                frame: 3,
                at,
                bounds: None,
            },
            ChildOf(panel),
        ));
    }
    world.spawn((
        Count,
        TextInput,
        TextLayout::no_wrap(),
        TabIndex(0),
        EditableText {
            max_characters: Some(5),
            allow_newlines: false,
            ..EditableText::new(q.quantity.to_string())
        },
        EditableTextFilter::new(|c| c.is_ascii_digit()),
        TextFont {
            font: assets.load("retail://data/fonts/wiz_text_font.sti#font").into(),
            font_size: 11.0.into(),
            ..default()
        },
        TextColor(Color::WHITE),
        absolute(Vec2::new(189., 112.), Vec2::new(49., 12.)),
        ZIndex(1),
        ChildOf(panel),
    ));
    for (at, first, delta, answer) in [
        (Vec2::new(217., 90.), 9, -1, None),
        (Vec2::new(190., 90.), 4, 1, None),
        (Vec2::new(237., 149.), 0, 0, Some(true)),
        (Vec2::new(273., 149.), 5, 0, Some(false)),
    ] {
        let path = if answer.is_some() { "popup_confirmationbuttons" } else { "popup_splititem" };
        world
            .spawn((
                Button,
                FocusRing,
                TabIndex(0),
                Pickable::default(),
                Picture {
                    sheet: assets.load(format!("retail://data/dialogs/{path}.sti")),
                    frame: first,
                    at,
                    bounds: Some(first),
                },
                ControlSkin::new(first, first + 1, first + 4, first + 2, first + 3),
                ChildOf(panel),
            ))
            .observe(move |_: On<Activate>, mut commands: Commands| {
                commands.queue(move |world: &mut World| {
                    if let Some(accept) = answer {
                        finish(world, panel, accept);
                    } else if let Some(draft) = world.get::<Draft>(panel) {
                        let n = (draft.quote.quantity as i64 + i64::from(delta))
                            .clamp(1, i64::from(draft.maximum))
                            as u32;
                        change(world, panel, n, true);
                    }
                });
            });
    }
    world.entity_mut(overlay).observe(
        move |mut event: On<FocusedInput<KeyboardInput>>, mut commands: Commands| {
            if event.input.state.is_pressed()
                && matches!(event.input.logical_key, Key::Escape | Key::Enter)
            {
                event.propagate(false);
                let accept = event.input.logical_key == Key::Enter;
                commands.queue(move |world: &mut World| finish(world, panel, accept));
            }
        },
    );
}

fn change(world: &mut World, panel: Entity, n: u32, rewrite: bool) {
    let Some(d) = world.get::<Draft>(panel) else { return };
    let (q, max) = (d.quote, d.maximum);
    if n == 0
        || n > max
        || world.get::<Conversation>(q.conversation).is_none_or(|c| c.revision != q.revision)
    {
        return;
    }
    if let Ok(quote) = quote_trade(world, q.conversation, q.source, n) {
        let remaining = if max == n {
            0
        } else {
            let Ok(remaining) = quote_trade(world, q.conversation, q.source, max - n) else {
                return;
            };
            remaining.gold
        };
        let mut draft = world.get_mut::<Draft>(panel).unwrap();
        draft.quote = quote;
        draft.remaining_price = remaining;
        if rewrite {
            let children: Vec<_> =
                world.get::<Children>(panel).into_iter().flatten().copied().collect();
            for child in children {
                if let Some(mut input) = world.get_mut::<EditableText>(child) {
                    input.clear();
                    input.queue_edit(TextEdit::Insert(n.to_string().into()));
                }
            }
        }
    }
}
fn finish(world: &mut World, panel: Entity, accept: bool) {
    let Some(d) = world.get::<Draft>(panel) else { return };
    let (view, mut q, previous, maximum) = (d.view, d.quote, d.previous_focus, d.maximum);
    if accept {
        let Some(count) = world
            .get::<Children>(panel)
            .into_iter()
            .flatten()
            .find_map(|e| world.get::<EditableText>(*e))
        else {
            return;
        };
        if count.is_composing() {
            return;
        }
        let Ok(n) = count.value().to_string().parse::<u32>() else { return };
        if n == 0 || n > maximum {
            return;
        }
        q.quantity = n;
    }
    let Some(overlay) = world.get::<ChildOf>(panel).map(ChildOf::parent) else { return };
    if accept && world.get::<Conversation>(q.conversation).is_some_and(|c| c.revision == q.revision)
    {
        select(world, view, q.conversation, q.source, q.quantity);
    }
    world.despawn(overlay);
    if let Some(entity) = previous.filter(|e| world.get_entity(*e).is_ok()) {
        world.resource_mut::<InputFocus>().set(entity, FocusCause::Navigated);
    }
}
pub(super) fn sync(
    mut commands: Commands,
    drafts: Query<(Entity, &Draft, &ChildOf)>,
    sessions: Query<&Conversation>,
    inputs: Query<(&EditableText, &ChildOf), (With<Count>, Changed<EditableText>)>,
    mut labels: Query<
        (&mut BitmapText, &ChildOf, Has<Remaining>, Has<RemainingPrice>, Option<&Weight>),
        Or<(With<Remaining>, With<Price>, With<RemainingPrice>, With<Weight>)>,
    >,
) {
    for (panel, d, parent) in &drafts {
        if sessions.get(d.quote.conversation).is_err() {
            commands.entity(parent.parent()).despawn();
            continue;
        }
        for (input, owner) in &inputs {
            if owner.parent() == panel
                && !input.is_composing()
                && let Ok(n) = input.value().to_string().parse::<u32>()
                && n != d.quote.quantity
            {
                commands.queue(move |world: &mut World| change(world, panel, n, false));
            }
        }
    }
    for (mut text, parent, remaining, remaining_price, weight) in &mut labels {
        let Ok((_, d, _)) = drafts.get(parent.parent()) else { continue };
        let value = if let Some(weight) = weight {
            let count = if weight.0 { d.maximum - d.quote.quantity } else { d.quote.quantity };
            format!("{:.1}", f64::from(d.unit_weight) * f64::from(count) / 10.)
        } else if remaining {
            (d.maximum - d.quote.quantity).to_string()
        } else {
            format!("{}g", if remaining_price { d.remaining_price } else { d.quote.gold })
        };
        if text.value != value {
            text.value = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_and_stale_accept_never_replace_the_selected_quote() {
        for accept in [false, true] {
            let mut world = World::new();
            world.init_resource::<InputFocus>();
            let session = world.spawn_empty().id();
            let q = TradeQuote {
                conversation: session,
                revision: 7,
                source: TradeSource::Stock(crate::game::npcs::trade::StockLotId(1)),
                quantity: 1,
                gold: 10,
            };
            let view = world
                .spawn((
                    ConversationView(session),
                    DialogueState { selected: Some(q), ..default() },
                ))
                .id();
            let overlay = world.spawn(ChildOf(view)).id();
            let panel = world
                .spawn((
                    Draft {
                        view,
                        quote: q,
                        maximum: 5,
                        unit_weight: 1,
                        remaining_price: 40,
                        previous_focus: None,
                    },
                    ChildOf(overlay),
                ))
                .id();
            let input = world.spawn((EditableText::new("3"), ChildOf(panel))).id();
            finish(&mut world, panel, accept);
            assert_eq!(world.get::<DialogueState>(view).unwrap().selected, Some(q));
            assert!(world.get_entity(overlay).is_err());
            assert!(world.get_entity(input).is_err());
        }
    }
}
