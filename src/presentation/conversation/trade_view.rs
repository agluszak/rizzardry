//! Project merchandise once per view, retaining resolved definitions through the pass.
use super::*;
use crate::game::npcs::trade::MerchantStock;

/// The gold amount typed to give the NPC.
#[derive(Component)]
pub(super) struct GiftGold;
pub(super) fn gift_amount(text: &EditableText) -> u32 {
    text.value().to_string().parse().unwrap_or(0)
}
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct TradeWidgets<'w, 's> {
    lists: Query<'w, 's, (Entity, Option<&'static Children>), With<TradeList>>,
    row_data: Query<'w, 's, (&'static mut TradeRow, &'static Children)>,
    offers: Query<'w, 's, Entity, With<Offer>>,
    captions: Query<'w, 's, (Entity, Has<TradeCount>), Or<(With<TradeCount>, With<TradePrice>)>>,
    quantities: Query<'w, 's, (Entity, Has<InteractionDisabled>), With<QuantityControl>>,
    texts: Query<'w, 's, &'static mut BitmapText>,
    controls: Query<'w, 's, (Entity, Has<InteractionDisabled>), With<Confirm>>,
    previews: Query<'w, 's, (&'static mut Picture, &'static mut Visibility), With<ItemPreview>>,
    gifts: Query<'w, 's, (&'static EditableText, &'static mut Node), With<GiftGold>>,
}
struct Listing<'a> {
    source: TradeSource,
    text: String,
    quantity: u32,
    item: &'a crate::content::items::ItemDefinition,
    units: u32,
}

pub(super) fn sync_trade_rows(
    views: ViewScope,
    mut commands: Commands,
    assets: Res<AssetServer>,
    content: If<Res<crate::game::data::Catalogs>>,
    sessions: Query<&Conversation>,
    stocks: Query<&MerchantStock>,
    parties: Query<(Entity, &PartyRoster), With<crate::game::party::roster::Party>>,
    members: Query<(
        &crate::game::characters::Character,
        &crate::game::actors::Attributes,
        &crate::game::actors::Skills,
    )>,
    held: crate::game::items::Items,
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
    let items = &content.items;
    let Ok((party, roster)) = parties.single() else { return };
    for (list, children) in &lists {
        let session = views.session();
        let (state, c) = (views.state(), sessions.get(session).unwrap());
        if state.mode == Mode::Talk {
            continue;
        }
        let stock = stocks.get(c.actor).ok();
        let member = selection.0.filter(|e| roster.contains(e));
        let allows = |d: &crate::content::items::ItemDefinition| {
            state.item_group.is_none_or(|group| group == ItemGroup::of(d.class))
                && state.usability.is_none_or(|party_wide| {
                    let users = crate::game::items::users(d);
                    let admitted = |e| {
                        let (character, attributes, skills) = members.get(e).unwrap();
                        users.admit(character, attributes, skills)
                    };
                    if party_wide {
                        roster.iter().any(admitted)
                    } else {
                        member.is_some_and(admitted)
                    }
                })
        };
        let mut rows = Vec::new();
        if state.mode == Mode::Buy {
            let shelves = stock.map(|_| trade::shelves(&held, c.actor, items));
            for shelf in shelves.into_iter().flatten() {
                let d = &items[shelf.item];
                if !d.merchant_cannot_sell && allows(d) {
                    rows.push(Listing {
                        source: TradeSource::Stock(shelf.items[0]),
                        text: format!("{} ({})", d.name, shelf.units),
                        quantity: trade::trade_quantity(d, shelf.units),
                        item: d,
                        units: shelf.units,
                    });
                }
            }
        } else {
            let pool = held.pool(party).iter().copied().filter(|_| state.pool);
            let packed =
                member.into_iter().flat_map(|e| held.pack(e)).flatten().filter(|_| !state.pool);
            for item in pool.chain(packed) {
                let Some(d) = held.id(item).map(|id| &items[id]) else { continue };
                if allows(d) {
                    rows.push(Listing {
                        source: TradeSource::Party(item),
                        text: held.name(item, items, true).unwrap_or_default(),
                        quantity: trade::trade_quantity(d, held.units(item)),
                        item: d,
                        units: held.units(item),
                    });
                }
            }
        }
        let selection = state.selection.filter(|chosen| {
            chosen.source().is_some_and(|source| rows.iter().any(|r| r.source == source))
        });
        let selected = selection
            .and_then(Selection::source)
            .and_then(|source| rows.iter().find(|r| r.source == source));
        let item = selected.map(|r| r.item);
        let units = selected.map_or(0, |r| r.units);
        let offer = if let Some(Selection::Refused(error)) = state.selection {
            format!("Trade unavailable: {error:?}")
        } else if let Some(item) = item {
            item.name.clone()
        } else {
            String::new()
        };
        for entity in &offers {
            if let Ok(mut text) = texts.get_mut(entity)
                && text.value != offer
            {
                text.value = offer.clone();
            }
        }
        for (entity, count) in &captions {
            if let Ok(mut text) = texts.get_mut(entity) {
                let value = match selection {
                    Some(Selection::Priced(q)) if count && units > 1 => {
                        q.offer.quantity.to_string()
                    }
                    Some(Selection::Priced(q)) if !count => format!("{}g", q.offer.gold),
                    Some(Selection::Gift(_)) if count && units > 1 => "1".into(),
                    _ => String::new(),
                };
                if text.value != value {
                    text.value = value;
                }
            }
        }
        for (entity, disabled) in &quantities {
            set_enabled(
                &mut commands,
                entity,
                matches!(selection, Some(Selection::Priced(_)))
                    && units > 1
                    && item.is_some_and(|d| d.class != EquipClass::Ammunition),
                disabled,
            );
        }
        let mut gold_typed = false;
        for (text, mut node) in &mut gifts {
            let shown = state.mode == Mode::Give && selection.is_none();
            crate::presentation::screen_art::display_if(&mut node, shown);
            gold_typed = shown && gift_amount(text) > 0;
        }
        for (entity, disabled) in &controls {
            set_enabled(&mut commands, entity, selection.is_some() || gold_typed, disabled);
        }
        for (mut picture, mut visibility) in &mut previews {
            visibility.set_if_neq(if item.is_some() {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
            if let Some(item) = item {
                picture.set_if_neq(Picture {
                    sheet: assets.load(format!("retail://{}", item.image)),
                    // Retail ShowSelectedNpcTradeItem uses SetImage's default frame 0.
                    frame: 0,
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
            if !rows.iter().any(|r| r.source == *source) {
                commands.entity(*entity).despawn();
            }
        }
        let mut ordered = Vec::with_capacity(rows.len());
        for Listing { source, text, quantity, .. } in rows {
            if let Some((entity, _)) = old.iter().find(|(_, s)| *s == source) {
                ordered.push(*entity);
                if let Ok((mut row, children)) = row_data.get_mut(*entity) {
                    row.set_if_neq(TradeRow { source, quantity });
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
                commands.entity(entity).insert(TradeRow { source, quantity });
            }
        }
        if children.is_none_or(|c| c.iter().ne(ordered.iter().copied())) {
            commands.entity(list).replace_children(&ordered);
        }
    }
}
