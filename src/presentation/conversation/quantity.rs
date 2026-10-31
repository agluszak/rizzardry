//! Retail split-item presentation. This edits a quote, never inventory itself.
use super::layout::Ui;
use super::*;
use crate::content::ui::TextTable;
use crate::presentation::screen_art::GameStrings;
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

/// The party asks to change a draft's count to `n`; the input already
/// shows it.
#[derive(Message, Clone, Copy)]
pub(super) struct Requote {
    panel: Entity,
    n: u32,
}

/// What editing a draft reads and changes: the drafts, their count inputs,
/// the views they choose for, and the trades they quote.
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct Drafting<'w, 's> {
    drafts: Query<'w, 's, (&'static mut Draft, &'static ChildOf)>,
    counts: Query<'w, 's, (&'static mut EditableText, &'static ChildOf), With<Count>>,
    states: Query<'w, 's, &'static mut DialogueState>,
    trading: Trading<'w, 's>,
    commands: Commands<'w, 's>,
}

impl Drafting<'_, '_> {
    /// Quote the draft for `n`, and what would be left, rewriting its input
    /// when the change came from a button.
    fn change(&mut self, panel: Entity, n: u32, rewrite: bool) {
        let Ok((d, _)) = self.drafts.get(panel) else { return };
        let (q, max) = (d.quote, d.maximum);
        if n == 0 || n > max {
            return;
        }
        let Ok(quote) = self.trading.quote(q.conversation, q.source, n) else { return };
        let remaining = if max == n {
            0
        } else {
            let Ok(remaining) = self.trading.quote(q.conversation, q.source, max - n) else {
                return;
            };
            remaining.gold
        };
        let (mut draft, _) = self.drafts.get_mut(panel).unwrap();
        draft.quote = quote;
        draft.remaining_price = remaining;
        if rewrite {
            for (mut input, parent) in &mut self.counts {
                if parent.parent() == panel {
                    input.clear();
                    input.queue_edit(TextEdit::Insert(n.to_string().into()));
                }
            }
        }
    }

    /// Close the draft, choosing its count for the view when accepted and
    /// still affordable.
    fn finish(&mut self, panel: Entity, accept: bool) {
        let Ok((d, overlay)) = self.drafts.get(panel) else { return };
        let (view, mut q, maximum, overlay) = (d.view, d.quote, d.maximum, overlay.parent());
        if accept {
            let Some((count, _)) = self.counts.iter().find(|(_, p)| p.parent() == panel) else {
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
        if accept && self.trading.ready(q.conversation) {
            let Ok(verified) = self.trading.quote(q.conversation, q.source, q.quantity) else {
                return;
            };
            if matches!(q.source, TradeSource::Stock(_)) && self.trading.gold() < verified.gold {
                return;
            }
            let mut state = self.states.get_mut(view).unwrap();
            select(&mut state, &self.trading, q.conversation, q.source, q.quantity);
        }
        self.commands.entity(overlay).despawn();
    }
}

/// The split-item dialog over the trade view, for the offer chosen when
/// its count may change (retail's split dialog for a stack).
pub(super) fn open(
    event: On<Activate>,
    controls: Query<&QuantityControl>,
    drafts: Query<&Draft>,
    states: Query<&DialogueState>,
    trading: Trading,
    (catalogs, sheets, strings): (Res<Catalogs>, Res<Assets<SpriteSheet>>, GameStrings),
    (assets, focus): (Res<AssetServer>, Res<InputFocus>),
    mut commands: Commands,
) {
    let Ok(&QuantityControl { view, canvas }) = controls.get(event.entity) else { return };
    if drafts.iter().any(|d| d.view == view) {
        return;
    }
    let Some(q) = states.get(view).ok().and_then(DialogueState::quote) else { return };
    if trading.quote(q.conversation, q.source, q.quantity).is_err() {
        return;
    }
    let Some((item, maximum)) = trading.units(&q) else { return };
    let item = &catalogs.items[item];
    if maximum < 2 || item.class == EquipClass::Ammunition {
        return;
    }
    let remaining_price = if maximum == q.quantity {
        0
    } else {
        let Ok(remaining) = trading.quote(q.conversation, q.source, maximum - q.quantity) else {
            return;
        };
        remaining.gold
    };
    let overlay = commands
        .spawn((
            absolute(Vec2::ZERO, Vec2::new(640., 480.)),
            GlobalZIndex(100),
            Pickable::default(),
            TabGroup::modal(),
            ModalDialog,
            crate::presentation::retail_controls::RestoreFocus(focus.get()),
            ChildOf(canvas),
        ))
        .id();
    let unit_weight = item.weight;
    let panel = commands
        .spawn((
            Draft { view, quote: q, maximum, unit_weight, remaining_price },
            absolute(Vec2::new(161., 148.), Vec2::new(322., 189.)),
            ChildOf(overlay),
        ))
        .id();
    let mut ui = Ui { commands: &mut commands, assets: &assets };
    build(&mut ui, panel, item, &q, (&sheets, strings.table()));
    ui.observe(
        overlay,
        move |mut event: On<FocusedInput<KeyboardInput>>, mut drafting: Drafting| {
            if event.input.state.is_pressed()
                && matches!(event.input.logical_key, Key::Escape | Key::Enter)
            {
                event.propagate(false);
                drafting.finish(panel, event.input.logical_key == Key::Enter);
            }
        },
    );
}

/// The dialog's picture, labels, count and buttons.
fn build(
    ui: &mut Ui,
    panel: Entity,
    item: &crate::content::items::ItemDefinition,
    q: &TradeQuote,
    (sheets, strings): (&Assets<SpriteSheet>, Option<&TextTable>),
) {
    use bevy::text::Justify::{self, Center, Right};
    fn dialog(ui: &Ui, path: &str) -> Handle<SpriteSheet> {
        ui.assets.load(format!("retail://data/dialogs/{path}.sti"))
    }
    let picture = |sheet, at| Picture { sheet, frame: 0, at, bounds: None };
    let frame = dialog(ui, "popup_splititem");
    ui.commands.spawn((picture(frame.clone(), Vec2::ZERO), ChildOf(panel)));
    let image = ui.assets.load(crate::content::paths::retail(&item.image));
    ui.commands.spawn((picture(image, Vec2::new(24., 14.)), ChildOf(panel)));
    fn text(
        ui: &mut Ui,
        panel: Entity,
        value: &str,
        at: Vec2,
        width: f32,
        justify: Justify,
    ) -> Entity {
        let e = ui.text(panel, value, at, width, justify);
        ui.commands.entity(e).insert(ZIndex(1));
        e
    }
    text(ui, panel, &item.name, Vec2::new(72., 15.), 226., Center);
    let class_name =
        strings.and_then(|s| s.get(1087 + u8::from(item.class) as usize)).unwrap_or("");
    text(ui, panel, "Type", Vec2::new(72., 35.), 53., Center);
    let mut description = format!("{class_name} ({})", item.kind_name);
    let font = ui.assets.load::<SpriteSheet>("retail://data/fonts/wiz_text_font.sti");
    if let Some(font) = sheets.get(&font) {
        while font.text_width(&description) > 169 {
            description.pop();
        }
    }
    text(ui, panel, &description, Vec2::new(129., 35.), 169., Center);
    for (y, remaining) in [(53., true), (131., false)] {
        text(ui, panel, "Weight", Vec2::new(72., y), 53., Center);
        let weight = text(ui, panel, "", Vec2::new(129., y), 54., Right);
        ui.commands.entity(weight).insert(Weight(remaining));
    }
    text(ui, panel, "Value", Vec2::new(187., 53.), 53., Center);
    let remaining_price = text(ui, panel, "", Vec2::new(244., 53.), 54., Right);
    ui.commands.entity(remaining_price).insert(RemainingPrice);
    let buying = matches!(q.source, TradeSource::Stock(_));
    let (keep, take) = if buying {
        ("Items in Stock", "Items to Buy")
    } else {
        ("Items To Keep", "Items to Sell")
    };
    text(ui, panel, keep, Vec2::new(72., 73.), 110., Center);
    text(ui, panel, take, Vec2::new(72., 111.), 110., Center);
    text(ui, panel, "Value", Vec2::new(187., 131.), 53., Center);
    let remaining = text(ui, panel, "", Vec2::new(188., 73.), 51., Right);
    ui.commands.entity(remaining).insert(Remaining);
    let price = text(ui, panel, "", Vec2::new(244., 131.), 54., Right);
    ui.commands.entity(price).insert(Price);
    for at in [
        Vec2::new(188., 73.),
        Vec2::new(188., 111.),
        Vec2::new(244., 131.),
        Vec2::new(244., 53.),
        Vec2::new(129., 53.),
        Vec2::new(129., 131.),
    ] {
        let well = Picture { frame: 3, ..picture(frame.clone(), at) };
        ui.commands.spawn((well, ChildOf(panel)));
    }
    ui.commands.spawn((
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
            font: ui.assets.load("retail://data/fonts/wiz_text_font.sti#font").into(),
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
        let button = ui
            .commands
            .spawn((
                Button,
                FocusRing,
                TabIndex(0),
                Pickable::default(),
                Picture { sheet: dialog(ui, path), frame: first, at, bounds: Some(first) },
                ControlSkin::new(first, first + 1, first + 4, first + 2, first + 3),
                ChildOf(panel),
            ))
            .observe(move |_: On<Activate>, mut drafting: Drafting| {
                if let Some(accept) = answer {
                    drafting.finish(panel, accept);
                } else if let Ok((draft, _)) = drafting.drafts.get(panel) {
                    let n = (draft.quote.quantity as i64 + i64::from(delta))
                        .clamp(1, i64::from(draft.maximum)) as u32;
                    drafting.change(panel, n, true);
                }
            })
            .id();
        match answer {
            Some(true) => {
                ui.commands.entity(button).insert(Control::Accept);
            }
            None => {
                ui.commands.entity(button).insert(Control::Adjust(delta));
            }
            Some(false) => {}
        }
    }
}

/// Counts the party types are quoted.
pub(super) fn requote(mut requests: MessageReader<Requote>, mut drafting: Drafting) {
    for &Requote { panel, n } in requests.read() {
        drafting.change(panel, n, false);
    }
}

pub(super) fn sync(
    mut commands: Commands,
    mut requotes: MessageWriter<Requote>,
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
                requotes.write(Requote { panel, n });
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
