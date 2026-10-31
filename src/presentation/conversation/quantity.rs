//! Retail split-item presentation. This edits a quote, never inventory itself.
use super::*;
use crate::content::ui::TextTable;
use crate::presentation::screen_art::StringTable;
use bevy::text::{EditableTextFilter, TextEdit};
use bevy::ui_widgets::ModalDialog;

#[derive(Component)]
pub(super) struct Draft {
    view: Entity,
    quote: TradeQuote,
    maximum: u32,
    unit_weight: u16,
    remaining_price: u32,
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
#[derive(Component)]
pub(super) enum Control {
    Accept,
    Adjust(i32),
}

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
                let party = crate::game::party::roster::the_party(world);
                world
                    .get::<PartyInventory>(party)
                    .unwrap()
                    .pool
                    .stacks
                    .iter()
                    .find(|stack| stack.id == item.id)
                    .map(|s| (s.item, s.units()))
            }
            ItemOwner::Member(member) => world
                .get::<Carried>(member)
                .unwrap()
                .find_stack(item.id)
                .map(|s| (s.item, s.units())),
        },
    }
}

pub(super) fn open(world: &mut World, view: Entity, canvas: Entity) {
    if world.query::<&Draft>().iter(world).any(|d| d.view == view) {
        return;
    }
    let Some(q) = world.get::<DialogueState>(view).and_then(|s| s.quote()) else { return };
    if quote_trade(world, q.conversation, q.source, q.quantity).is_err() {
        return;
    }
    let Some((item, maximum)) = stack(world, q) else { return };
    let item = &world.resource::<crate::game::data::Catalogs>().items[item];
    if maximum < 2 || item.class == EquipClass::Ammunition {
        return;
    }
    let (name, image, unit_weight, class, generic) =
        (item.name.clone(), item.image.clone(), item.weight, item.class, item.kind_name.clone());
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
            crate::presentation::retail_controls::RestoreFocus(previous_focus),
            ChildOf(canvas),
        ))
        .id();
    let panel = world
        .spawn((
            Draft { view, quote: q, maximum, unit_weight, remaining_price },
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
    let class_name = world
        .get_resource::<StringTable>()
        .and_then(|strings| world.get_resource::<Assets<TextTable>>()?.get(&strings.0))
        .and_then(|s| s.get(1087 + u8::from(class) as usize))
        .unwrap_or("")
        .to_owned();
    label(world, panel, "Type", Vec2::new(72., 35.), 53.);
    let mut description = format!("{class_name} ({generic})");
    let font = assets.load::<SpriteSheet>("retail://data/fonts/wiz_text_font.sti");
    if let Some(font) = world.resource::<Assets<SpriteSheet>>().get(&font) {
        while font.text_width(&description) > 169 {
            description.pop();
        }
    }
    label(world, panel, &description, Vec2::new(129., 35.), 169.);
    for (y, remaining) in [(53., true), (131., false)] {
        label(world, panel, "Weight", Vec2::new(72., y), 53.);
        let weight = label(world, panel, "", Vec2::new(129., y), 54.);
        world.entity_mut(weight).insert((Weight(remaining), ZIndex(1)));
    }
    label(world, panel, "Value", Vec2::new(187., 53.), 53.);
    let remaining_price_label = label(world, panel, "", Vec2::new(244., 53.), 54.);
    world.entity_mut(remaining_price_label).insert((RemainingPrice, ZIndex(1)));
    let buying = matches!(q.source, TradeSource::Stock(_));
    label(
        world,
        panel,
        if buying { "Items in Stock" } else { "Items To Keep" },
        Vec2::new(72., 73.),
        110.,
    );
    label(
        world,
        panel,
        if buying { "Items to Buy" } else { "Items to Sell" },
        Vec2::new(72., 111.),
        110.,
    );
    label(world, panel, "Value", Vec2::new(187., 131.), 53.);
    let remaining = label(world, panel, "", Vec2::new(188., 73.), 51.);
    world.entity_mut(remaining).insert((Remaining, ZIndex(1)));
    let price = label(world, panel, "", Vec2::new(244., 131.), 54.);
    world.entity_mut(price).insert((Price, ZIndex(1)));
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
        let button = world
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
            })
            .id();
        if answer == Some(true) {
            world.entity_mut(button).insert(Control::Accept);
        } else if answer.is_none() {
            world.entity_mut(button).insert(Control::Adjust(delta));
        }
    }
    let children: Vec<_> = world.get::<Children>(panel).into_iter().flatten().copied().collect();
    for child in children {
        let right = world.get::<Remaining>(child).is_some()
            || world.get::<Price>(child).is_some()
            || world.get::<RemainingPrice>(child).is_some()
            || world.get::<Weight>(child).is_some();
        if let Some(mut text) = world.get_mut::<BitmapText>(child) {
            text.justify =
                if right { bevy::text::Justify::Right } else { bevy::text::Justify::Center };
            world.entity_mut(child).insert(ZIndex(1));
        }
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
    if n == 0 || n > max {
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
    let (view, mut q, maximum) = (d.view, d.quote, d.maximum);
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
    if accept && world.get::<Conversation>(q.conversation).is_some_and(Conversation::ready) {
        let Ok(verified) = quote_trade(world, q.conversation, q.source, q.quantity) else { return };
        if matches!(q.source, TradeSource::Stock(_)) {
            let party = crate::game::party::roster::the_party(world);
            let gold = world.get::<PartyInventory>(party).unwrap().gold;
            if gold < verified.gold {
                return;
            }
        }
        select(world, view, q.source, q.quantity);
    }
    world.despawn(overlay);
}
pub(super) fn sync(
    mut commands: Commands,
    drafts: Query<(Entity, &Draft, &ChildOf)>,
    sessions: Query<&Conversation>,
    inputs: Query<(Ref<EditableText>, &ChildOf), With<Count>>,
    buttons: Query<(Entity, &ChildOf, &Control, Has<InteractionDisabled>)>,
    parties: Query<&PartyInventory, With<crate::game::party::roster::Party>>,
    mut labels: Query<
        (&mut BitmapText, &ChildOf, Has<Remaining>, Has<RemainingPrice>, Option<&Weight>),
        Or<(With<Remaining>, With<Price>, With<RemainingPrice>, With<Weight>)>,
    >,
) {
    for (panel, d, parent) in &drafts {
        if !sessions.get(d.quote.conversation).is_ok_and(Conversation::ready) {
            commands.entity(parent.parent()).despawn();
            continue;
        }
        for (input, owner) in &inputs {
            if owner.parent() == panel
                && input.is_changed()
                && !input.is_composing()
                && let Ok(n) = input.value().to_string().parse::<u32>()
                && n != d.quote.quantity
            {
                commands.queue(move |world: &mut World| change(world, panel, n, false));
            }
        }
    }
    for (button, parent, kind, disabled) in &buttons {
        let Ok((panel, d, _)) = drafts.get(parent.parent()) else { continue };
        let entered = inputs
            .iter()
            .find(|(_, p)| p.parent() == panel)
            .and_then(|(input, _)| input.value().to_string().parse::<u32>().ok());
        let valid = entered.is_some_and(|n| n > 0 && n <= d.maximum && n == d.quote.quantity);
        let enabled = match kind {
            Control::Accept => {
                valid
                    && (!matches!(d.quote.source, TradeSource::Stock(_))
                        || parties.single().is_ok_and(|p| p.gold >= d.quote.gold))
            }
            Control::Adjust(delta) => {
                if *delta < 0 {
                    d.quote.quantity > 1
                } else {
                    d.quote.quantity < d.maximum
                }
            }
        };
        set_enabled(&mut commands, button, enabled, disabled);
    }
    for (mut text, parent, remaining, remaining_price, weight) in &mut labels {
        let Ok((_, d, _)) = drafts.get(parent.parent()) else { continue };
        let value = if let Some(weight) = weight {
            let count = if weight.0 { d.maximum - d.quote.quantity } else { d.quote.quantity };
            format!("{:.1}", f64::from(d.unit_weight) * f64::from(count) / 10.)
        } else if remaining {
            (d.maximum - d.quote.quantity).to_string()
        } else {
            (if remaining_price { d.remaining_price } else { d.quote.gold }).to_string()
        };
        if text.value != value {
            text.value = value;
        }
    }
}
