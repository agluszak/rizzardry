//! Retail's spell casting and item use views over the bottom of the main
//! screen (`MGSSpellCasting.cpp`, `MGSUseItemSelect.cpp`): realms, the realm's spells, their
//! power pips and the cast's cost; or the member's usable items. The view
//! holds only the unconfirmed choice. It commits as soon as a source, a power
//! and a target are chosen, sending a [`MagicRequest`] gameplay accepts or
//! refuses.
//!
//! Spells for one victim of the party wait for a portrait click; spells at
//! enemies take the nearest monster in reach before the party, as retail's
//! auto-targeting option does. Retail's pointer picking of monsters in the 3D
//! view is not ported. Lists use native stepped scrollbars; pool items keep
//! their shared ownership through selection and payment.

use bevy::input_focus::InputFocus;
use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Selected;
use bevy::ui_widgets::{Activate, Button as UiButton, SliderRange, SliderValue};
use bevy_enhanced_input::prelude::*;
use enumset::EnumSet;

use crate::content::characters::Realm;
use crate::content::items::ItemCatalog;
use crate::content::spells::{MAX_POWER, PowerChoice, SpellCatalog, SpellDefinition, Targeting};
use crate::content::ui::{SpriteSheet, TextTable};
use crate::game::characters::{Character, Vitals};
use crate::game::combat::Battle;
use crate::game::data::GameData;
use crate::game::inventory::{Carried, PartyInventory};
use crate::game::magic::casting::{
    AUTO_POWER, Aim, MagicChoice, Source, UseContext, can_activate, usable_items, usable_now,
};
use crate::game::magic::{LearnedSpells, MagicRequest, SpellPoints, max_points, modelled};
use crate::game::party::roster::PartyRoster;
use crate::game::world::population::Monster;
use crate::presentation::retail_controls::{
    ControlSkin, FocusRing, ScrollWheel, scroll_wheel, set_scroll_maximum, step_scrollbar,
};
use crate::presentation::screen_art::{BitmapText, Picture, absolute};

/// The view's art (retail catalog entries 0x190-0x198, 0x86, 0x8e, 0x97).
#[derive(Resource, Clone)]
pub struct MagicViewArt {
    spell_panels: Handle<SpriteSheet>,
    realm_buttons: Handle<SpriteSheet>,
    realms: [Handle<SpriteSheet>; 6],
    pips: Handle<SpriteSheet>,
    cancel: Handle<SpriteSheet>,
    scroll: Handle<SpriteSheet>,
    item_panels: Handle<SpriteSheet>,
    font: Handle<SpriteSheet>,
    red: Handle<SpriteSheet>,
    orange: Handle<SpriteSheet>,
    blue: Handle<SpriteSheet>,
    yellow: Handle<SpriteSheet>,
    strings: Handle<TextTable>,
}

pub(super) fn load_magic_view_art(mut commands: Commands, assets: Res<AssetServer>) {
    let sheet = |path: &str| assets.load(format!("retail://data/{path}.sti"));
    commands.insert_resource(MagicViewArt {
        spell_panels: sheet("spell casting/magicbottomnew"),
        realm_buttons: sheet("spell casting/realmhighlightnew"),
        realms: ["fire", "water", "air", "earth", "mental", "divine"]
            .map(|realm| sheet(&format!("spell casting/{realm}_realm"))),
        pips: sheet("spell casting/powerbuttons"),
        cancel: sheet("main interface/confirmation_buttons"),
        scroll: sheet("main interface/main_scroll"),
        item_panels: sheet("main interface/item_bottompanel"),
        font: sheet("fonts/wiz_text_font"),
        red: sheet("fonts/palettered"),
        orange: sheet("fonts/paletteorange"),
        blue: sheet("fonts/paletteblue"),
        yellow: sheet("fonts/paletteyellow"),
        strings: assets.load("retail://data/strings/stringdata.dat"),
    });
}

/// The view's keys: Escape closes it, ahead of every screen beneath it (a
/// camp's Escape would end the rest). Its controls are tab stops and it
/// takes focus when it opens, so the world's keys and the panel it may
/// cover do not act while it is up; it gives focus back when it closes.
#[derive(Component, Clone, Default)]
pub struct MagicKeys;

#[derive(InputAction)]
#[action_output(bool)]
struct CloseMagic;

/// What had focus when the view opened, to have it back when it closes.
#[derive(Component, Debug)]
struct RestoreFocus(Option<Entity>);

pub fn magic_keys(app: &mut App) {
    app.add_input_context::<MagicKeys>().add_observer(give_focus_back);
}

fn close_by_key(event: On<Start<CloseMagic>>, mut commands: Commands) {
    commands.entity(event.context).despawn();
}

/// However the view closes, focus goes back where it was, if that is
/// still there.
fn give_focus_back(
    remove: On<Remove<MagicView>>,
    views: Query<&RestoreFocus>,
    exists: Query<()>,
    mut focus: Option<ResMut<InputFocus>>,
) {
    let (Ok(RestoreFocus(previous)), Some(focus)) = (views.get(remove.entity), focus.as_mut())
    else {
        return;
    };
    match previous.filter(|&entity| exists.contains(entity)) {
        Some(entity) => focus.set(entity, bevy::input_focus::FocusCause::Navigated),
        None => focus.clear(),
    }
}

/// The open view and its unconfirmed choice.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct MagicView {
    pub caster: Entity,
    pub items: bool,
    /// The device attempt it casts at, if any.
    pub attempt: Option<Entity>,
    pool: bool,
    realm: Option<Realm>,
    source: Option<Source>,
    /// A chosen pip: power 1..=7, or [`AUTO_POWER`].
    power: Option<u8>,
    /// The party member a one-victim spell is aimed at, from a portrait.
    member: Option<Entity>,
}

impl MagicView {
    /// Whether a portrait click now aims the spell rather than selects.
    pub fn wants_member(&self) -> bool {
        self.attempt.is_none() && self.source.is_some() && self.member.is_none()
    }

    pub fn aim_at(&mut self, member: Entity) {
        self.member = Some(member);
    }
}

#[derive(Component)]
pub(super) struct MagicScroll(Entity);

#[derive(Component, Clone, Copy)]
pub(super) struct RealmButton(Realm);
#[derive(Component, Clone, Copy)]
pub(super) struct RealmText(Realm);
#[derive(Component, Clone, Copy)]
pub(super) struct RealmIcon(Realm);
#[derive(Component, Clone, Copy)]
pub(super) struct Row(usize);
#[derive(Component, Clone, Copy)]
pub(super) struct RowText(usize);
#[derive(Component, Clone, Copy)]
pub(super) struct RowCost(usize);
#[derive(Component, Clone, Copy)]
pub(super) struct Pip(u8);
#[derive(Component)]
pub(super) struct CostLabel;
#[derive(Component)]
pub(super) struct ItemIcon;
/// "Select Item" (retail string 0x77a).
#[derive(Component)]
pub(super) struct Caption;

const ROWS: usize = 7;
const ROW_HEIGHT: f32 = 11.0;

/// Realm buttons in retail order: their rect's top-left, the text's left and
/// width, the icon's place, and whether the button art is the right-hand one.
const REALM_BUTTONS: [(Vec2, f32, f32, Vec2, bool); 6] = [
    (Vec2::new(53.0, 371.0), 53.0, 55.0, Vec2::new(108.0, 373.0), false),
    (Vec2::new(136.0, 371.0), 156.0, 55.0, Vec2::new(138.0, 373.0), true),
    (Vec2::new(41.0, 393.0), 41.0, 55.0, Vec2::new(96.0, 395.0), false),
    (Vec2::new(148.0, 393.0), 168.0, 55.0, Vec2::new(150.0, 395.0), true),
    (Vec2::new(53.0, 415.0), 53.0, 55.0, Vec2::new(108.0, 417.0), false),
    (Vec2::new(136.0, 415.0), 156.0, 55.0, Vec2::new(138.0, 417.0), true),
];
/// Each realm icon's resting frame (retail `g_spell_realm_animations`).
const REALM_FRAMES: [usize; 6] = [3, 8, 8, 1, 14, 11];
/// The chosen realm's icon cycles through this many frames, 50 ms each.
const REALM_CYCLES: [usize; 6] = [22, 18, 14, 21, 16, 24];
/// Power pips: rect top-left and size (`MGSSpellCasting.cpp:176-215`).
const PIPS: [(Vec2, f32); 9] = [
    (Vec2::new(476.0, 360.0), 15.0),
    (Vec2::new(492.0, 362.0), 17.0),
    (Vec2::new(506.0, 374.0), 19.0),
    (Vec2::new(511.0, 393.0), 21.0),
    (Vec2::new(502.0, 414.0), 23.0),
    (Vec2::new(478.0, 423.0), 25.0),
    (Vec2::new(451.0, 416.0), 27.0),
    (Vec2::new(436.0, 389.0), 29.0),
    (Vec2::new(436.0, 389.0), 29.0),
];

fn text(art: &MagicViewArt, at: Vec2, width: f32, justify: bevy::text::Justify) -> BitmapText {
    BitmapText { font: art.font.clone(), at, width, justify, ..default() }
}

/// Open the view for `caster`, replacing one already open.
pub fn open(
    commands: &mut Commands,
    art: &MagicViewArt,
    caster: Entity,
    items: bool,
    requested_attempt: Option<Entity>,
) {
    let art = art.clone();
    commands.queue(move |world: &mut World| {
        use crate::game::world::interaction::{
            DeviceInteraction, Work, current, prepare_device_magic,
        };
        let attempt = match current(world) {
            Some((attempt, _)) => {
                if matches!(
                    world.get::<DeviceInteraction>(attempt).map(|i| &i.work),
                    Some(Work::Looting)
                ) || requested_attempt.is_some_and(|requested| requested != attempt)
                {
                    return;
                }
                Some(attempt)
            }
            None if requested_attempt.is_some() => return,
            None => None,
        };
        if let Some(attempt) = attempt
            && prepare_device_magic(world, caster, attempt).is_err()
        {
            return;
        }
        // Serialize replacements, including multiple opens queued in one frame.
        // A replacement gives focus back to what the first view took it from.
        let previous: Vec<_> = world
            .query::<(Entity, &RestoreFocus)>()
            .iter(world)
            .map(|(view, restore)| (view, restore.0))
            .collect();
        let restore = match previous.first() {
            Some(&(_, restore)) => restore,
            None => world.get_resource::<InputFocus>().and_then(InputFocus::get),
        };
        for (view, _) in previous {
            world.entity_mut(view).remove::<RestoreFocus>();
            world.despawn(view);
        }
        let mut state = bevy::ecs::system::SystemState::<Commands>::new(world);
        let first =
            spawn_view(&mut state.get_mut(world).unwrap(), &art, caster, items, attempt, restore);
        state.apply(world);
        if let Some(mut focus) = world.get_resource_mut::<InputFocus>() {
            focus.set(first, bevy::input_focus::FocusCause::Navigated);
        }
    });
}

fn spawn_view(
    commands: &mut Commands,
    art: &MagicViewArt,
    caster: Entity,
    items: bool,
    attempt: Option<Entity>,
    restore: Option<Entity>,
) -> Entity {
    let root = commands
        .spawn_scene(bsn! {
            MagicKeys
            ContextPriority::<MagicKeys>::new(2)
            Actions<MagicKeys> [
                Action::<CloseMagic>::new()
                ActionSettings { consume_input: true }
                Bindings [ Binding(KeyCode::Escape) ]
            ]
            on(close_by_key)
        })
        .insert((
            RestoreFocus(restore),
            TabGroup::modal(),
            MagicView {
                caster,
                items,
                attempt,
                pool: false,
                realm: None,
                source: None,
                power: None,
                member: None,
            },
            absolute(Vec2::ZERO, crate::presentation::screen_art::SCREEN),
            GlobalZIndex(25),
            Pickable::IGNORE,
            DespawnOnExit(crate::game::screen::Screen::World),
        ))
        .id();
    let picture = |commands: &mut Commands, sheet: &Handle<SpriteSheet>, frame, at| {
        commands
            .spawn((Picture { sheet: sheet.clone(), frame, at, bounds: None }, ChildOf(root)))
            .id()
    };
    let button = |commands: &mut Commands,
                  sheet: &Handle<SpriteSheet>,
                  at: Vec2,
                  size: Vec2,
                  skin: ControlSkin| {
        commands
            .spawn((
                UiButton,
                TabIndex(0),
                FocusRing,
                Hovered::default(),
                skin,
                Picture { sheet: sheet.clone(), frame: skin.normal, at, bounds: None },
                absolute(at, size),
                Pickable::default(),
                ChildOf(root),
            ))
            .id()
    };
    let track = step_scrollbar(
        commands,
        root,
        art.scroll.clone(),
        Vec2::new(if items { 457.0 } else { 400.0 }, 366.0),
        45.0,
        0.0,
    );
    commands.entity(root).insert((MagicScroll(track), ScrollWheel(track))).observe(scroll_wheel);
    let left = bevy::text::Justify::Left;
    let rows_at = if items { Vec2::new(168.0, 366.0) } else { Vec2::new(234.0, 366.0) };
    let rows_width = if items { 284.0 } else { 162.0 };
    // The view's first focus: its cancel button, which every view has.
    let cancel = if items {
        commands
            .spawn((
                UiButton,
                TabIndex(0),
                FocusRing,
                Hovered::default(),
                absolute(Vec2::new(27.0, 430.0), Vec2::new(130.0, 16.0)),
                BackgroundColor(Color::srgb(0.15, 0.12, 0.08)),
                ChildOf(root),
            ))
            .observe(toggle_pool)
            .with_child((
                BitmapText {
                    value: "Carried / Party pool".into(),
                    ..text(art, Vec2::ZERO, 130.0, bevy::text::Justify::Center)
                },
                Pickable::IGNORE,
            ));
        for (frame, x) in [(0, 23.0), (1, 164.0), (2, 476.0)] {
            picture(commands, &art.item_panels, frame, Vec2::new(x, 358.0));
        }
        commands.spawn((
            Caption,
            text(art, Vec2::new(27.0, 365.0), 133.0, bevy::text::Justify::Center),
            ChildOf(root),
        ));
        commands.spawn((
            ItemIcon,
            Picture {
                sheet: Handle::default(),
                frame: 0,
                at: Vec2::new(481.0, 363.0),
                bounds: None,
            },
            Visibility::Hidden,
            ChildOf(root),
        ));
        let cancel = button(
            commands,
            &art.cancel,
            Vec2::new(584.0, 364.0),
            Vec2::splat(27.0),
            ControlSkin::new(4, 5, 6, 6, 7),
        );
        commands.entity(cancel).observe(cancel_view);
        cancel
    } else {
        for (frame, x) in [(0, 23.0), (1, 230.0), (2, 420.0)] {
            picture(commands, &art.spell_panels, frame, Vec2::new(x, 358.0));
        }
        for (i, realm) in EnumSet::<Realm>::all().iter().enumerate() {
            let (at, text_left, text_width, icon, right) = REALM_BUTTONS[i];
            let skin = if right {
                ControlSkin::new(4, 5, 7, 7, 6)
            } else {
                ControlSkin::new(0, 1, 3, 3, 2)
            };
            let realm_button =
                button(commands, &art.realm_buttons, at, Vec2::new(75.0, 22.0), skin);
            commands.entity(realm_button).insert(RealmButton(realm)).observe(choose_realm);
            commands.spawn((
                RealmText(realm),
                text(
                    art,
                    Vec2::new(text_left, at.y + 6.0),
                    text_width,
                    bevy::text::Justify::Center,
                ),
                ChildOf(root),
            ));
            let icon = picture(commands, &art.realms[i], REALM_FRAMES[i], icon);
            commands.entity(icon).insert(RealmIcon(realm));
        }
        for pip in 0..9u8 {
            let (at, size) = PIPS[usize::from(pip)];
            commands
                .spawn((
                    Pip(pip),
                    UiButton,
                    TabIndex(0),
                    FocusRing,
                    Hovered::default(),
                    Picture {
                        sheet: art.pips.clone(),
                        frame: usize::from(pip) * 6 + 3,
                        at,
                        bounds: None,
                    },
                    absolute(at, Vec2::splat(size)),
                    Pickable::default(),
                    ChildOf(root),
                ))
                .observe(choose_power);
        }
        commands.spawn((
            CostLabel,
            text(art, Vec2::new(434.0, 371.0), 34.0, bevy::text::Justify::Center),
            ChildOf(root),
        ));
        let cancel = button(
            commands,
            &art.cancel,
            Vec2::new(570.0, 364.0),
            Vec2::splat(27.0),
            ControlSkin::new(4, 5, 6, 6, 7),
        );
        commands.entity(cancel).observe(cancel_view);
        cancel
    };
    for i in 0..ROWS {
        let at = rows_at + Vec2::Y * ROW_HEIGHT * i as f32;
        commands
            .spawn((
                Row(i),
                UiButton,
                TabIndex(0),
                FocusRing,
                Hovered::default(),
                absolute(at, Vec2::new(rows_width, ROW_HEIGHT)),
                Pickable::default(),
                ChildOf(root),
            ))
            .observe(choose_row);
        commands.spawn((RowText(i), text(art, at, rows_width, left), ChildOf(root)));
        commands.spawn((
            RowCost(i),
            text(art, at, rows_width, bevy::text::Justify::Right),
            ChildOf(root),
        ));
    }
    cancel
}

/// A row's offer: the source, its spell, its label and cost, and whether
/// it can be chosen now.
struct Offer<'a> {
    source: Source,
    spell: &'a SpellDefinition,
    label: String,
    cost: String,
    available: bool,
    /// Barred by silence: shown orange, not red.
    silenced: bool,
}

fn context(
    view: &MagicView,
    fighting: bool,
    camped: bool,
    attempts: &Query<&crate::game::world::interaction::DeviceInteraction>,
) -> Option<UseContext> {
    use crate::game::world::interaction::Work;
    match view.attempt {
        Some(attempt) => {
            let interaction = attempts.get(attempt).ok()?;
            match interaction.work {
                Work::Picking(_) => Some(UseContext::Lock),
                Work::Disarming(_) => Some(UseContext::Trap),
                Work::Looting => None,
            }
        }
        None if fighting => Some(UseContext::Combat),
        None if camped => Some(UseContext::Camp),
        None => Some(UseContext::World),
    }
}

/// The target-type glyph retail puts before a spell's name.
fn glyph(targeting: Targeting) -> char {
    let index = match targeting {
        Targeting::Caster | Targeting::Ally | Targeting::Enemy => 2,
        Targeting::Party | Targeting::EnemyGroup => 3,
        Targeting::Cone => 1,
        Targeting::Radius | Targeting::Point => 0,
        _ => 4,
    };
    char::from_u32(0xfff0 + index).unwrap_or(' ')
}

#[allow(clippy::too_many_arguments)]
fn offers<'a>(
    view: &MagicView,
    character: &Character,
    carried: Option<&Carried>,
    learned: Option<&LearnedSpells>,
    points: Option<&SpellPoints>,
    spells: &'a SpellCatalog,
    items: &ItemCatalog,
    context: UseContext,
    pool: &PartyInventory,
    silenced: bool,
) -> Vec<Offer<'a>> {
    if view.items {
        let sources: Vec<_> = if view.pool {
            pool.pool
                .stacks
                .iter()
                .filter_map(|stack| {
                    let definition = items.get(stack.item)?;
                    let spell = definition.spell?.spell;
                    (can_activate(character, definition, false)
                        && stack.remaining_uses() != Some(0)
                        && (modelled(spell) || matches!(spell.0, 18 | 39)))
                    .then_some((Source::PoolItem(stack.id), stack, spell))
                })
                .collect()
        } else {
            let Some(carried) = carried else { return Vec::new() };
            usable_items(character, carried, items)
                .into_iter()
                .filter_map(|(id, spell)| Some((Source::Item(id), carried.find_stack(id)?, spell)))
                .collect()
        };
        return sources
            .into_iter()
            .filter_map(|(source, stack, id)| {
                let spell = spells.get(id)?;
                let definition = items.get(stack.item)?;
                let silenced =
                    silenced && definition.use_type == crate::content::items::UseType::Instrument;
                let available = usable_now(spell, context) && !silenced;
                if matches!(context, UseContext::Lock | UseContext::Trap | UseContext::Camp)
                    && !available
                {
                    return None;
                }
                let label = match stack.remaining_uses() {
                    Some(_) if !stack.identified => {
                        format!("{} (?)", stack.display_name(definition, false))
                    }
                    Some(uses) => format!("{} ({uses})", stack.display_name(definition, false)),
                    None => stack.display_name(definition, true),
                };
                Some(Offer { source, spell, label, cost: String::new(), available, silenced })
            })
            .collect();
    }
    let (Some(realm), Some(learned)) = (view.realm, learned) else { return Vec::new() };
    let mut offers: Vec<_> = learned
        .0
        .iter()
        .filter_map(|&id| spells.get(id))
        .filter(|s| s.realm == Some(realm))
        .map(|s| {
            let affordable = points.is_some_and(|p| p.affordable_power(s) > 0);
            Offer {
                source: Source::Spell { spell: s.id, power: 1 },
                spell: s,
                label: format!("{} {}", glyph(s.targeting), s.name),
                cost: s.cost.to_string(),
                available: affordable
                    && (modelled(s.id) || matches!(s.id.0, 18 | 39))
                    && usable_now(s, context)
                    && !silenced,
                silenced,
            }
        })
        .collect();
    if matches!(context, UseContext::Lock | UseContext::Trap) {
        offers.retain(|o| usable_now(o.spell, context));
    }
    // Castable first, then the rest (retail's two passes).
    offers.sort_by_key(|o| !o.available);
    offers
}

/// Retail `GetSpellCastRating`: the pip art by failure chance.
fn rating(chance: u32) -> usize {
    match chance {
        0 => 5,
        1..=5 => 4,
        6..=15 => 3,
        16..=40 => 2,
        _ => 1,
    }
}

/// Draw the view from the current state, and commit when the choice is whole.
#[allow(clippy::too_many_arguments)]
pub(super) fn refresh_magic_view(
    mut commands: Commands,
    mut views: Query<(Entity, &mut MagicView, &MagicScroll)>,
    sliders: Query<(&SliderValue, &SliderRange)>,
    (selected, roster): (Res<super::SelectedMember>, Single<&PartyRoster>),
    members: Query<(&Character, &Vitals, Option<&Carried>)>,
    casters: Query<(&LearnedSpells, Option<&SpellPoints>)>,
    (battle, monsters, party): (
        Option<Single<(&Battle, Option<&crate::game::combat::Combatants>)>>,
        Query<(Entity, &Monster, &GlobalTransform)>,
        Query<&GlobalTransform, With<crate::game::party::body::PartyBody>>,
    ),
    (camp, interaction, pool): (
        Option<Res<crate::game::camp::Camp>>,
        Query<&crate::game::world::interaction::DeviceInteraction>,
        Single<&PartyInventory>,
    ),
    data: Option<Res<GameData>>,
    (spells, items, art, assets, tables): (
        Res<Assets<SpellCatalog>>,
        Res<Assets<ItemCatalog>>,
        Res<MagicViewArt>,
        Res<AssetServer>,
        Res<Assets<TextTable>>,
    ),
    mut captions: Query<
        &mut BitmapText,
        (With<Caption>, Without<CostLabel>, Without<RowText>, Without<RowCost>, Without<RealmText>),
    >,
    mut parts: ParamSet<(
        Query<(&RealmButton, Has<Selected>, Entity)>,
        Query<(&RealmText, &mut BitmapText)>,
        Query<(&Row, &mut Visibility)>,
        Query<(&RowText, &mut BitmapText)>,
        Query<(&RowCost, &mut BitmapText)>,
        Query<(&Pip, &Hovered, &mut Picture, &mut Visibility), (Without<Row>, Without<RealmIcon>)>,
        Query<&mut BitmapText, With<CostLabel>>,
        Query<
            (&mut Picture, &mut Visibility),
            (With<ItemIcon>, Without<Pip>, Without<Row>, Without<RealmIcon>),
        >,
    )>,
    mut icons: Query<(&RealmIcon, &mut Picture), (Without<ItemIcon>, Without<Pip>)>,
    time: Res<Time<Real>>,
    rows_hovered: Query<(&Row, &Hovered)>,
    mut requests: MessageWriter<MagicRequest>,
) {
    let Ok((root, mut view, scroll)) = views.single_mut() else { return };
    // The selected member casts; choosing another starts over for them.
    if let Some(member) = selected.member(&roster)
        && member != view.caster
    {
        if let Some(attempt) = view.attempt {
            commands.queue(move |world: &mut World| {
                let _ =
                    crate::game::world::interaction::prepare_device_magic(world, member, attempt);
            });
        }
        *view = MagicView {
            caster: member,
            items: view.items,
            attempt: view.attempt,
            pool: view.pool,
            realm: None,
            source: None,
            power: None,
            member: None,
        };
    }
    if view.attempt.is_some()
        && !members.get(view.caster).is_ok_and(|(_, v, _)| crate::game::camp::can_act(v))
    {
        commands.entity(root).despawn();
        return;
    }
    let (Some(spells), Some(items)) = (
        data.as_ref().and_then(|d| spells.get(&d.spells)),
        data.as_ref().and_then(|d| items.get(&d.items)),
    ) else {
        return;
    };
    let Some(context) = context(&view, battle.is_some(), camp.is_some(), &interaction) else {
        commands.entity(root).despawn();
        return;
    };
    let Ok((character, _, carried)) = members.get(view.caster) else { return };
    let (learned, points) = casters.get(view.caster).map_or((None, None), |(l, p)| (Some(l), p));
    let maximum = learned.map(|l| max_points(character, l, spells)).unwrap_or_default();
    // Realms: their points, the chosen one latched; the first with points
    // chosen when none is.
    if !view.items {
        if view.realm.is_none() {
            view.realm = EnumSet::<Realm>::all().iter().find(|&r| maximum[r] > 0);
        }
        for (button, selected, entity) in &parts.p0() {
            let wanted = view.realm == Some(button.0);
            if wanted != selected {
                if wanted {
                    commands.entity(entity).insert(Selected);
                } else {
                    commands.entity(entity).remove::<Selected>();
                }
            }
            crate::presentation::retail_controls::set_enabled(
                &mut commands,
                entity,
                maximum[button.0] > 0,
                false,
            );
        }
        let step = (time.elapsed_secs() / 0.05) as usize;
        for (icon, mut picture) in &mut icons {
            let index = icon.0 as usize;
            let frame = if view.realm == Some(icon.0) {
                step % REALM_CYCLES[index]
            } else {
                REALM_FRAMES[index]
            };
            if picture.frame != frame {
                picture.frame = frame;
            }
        }
        for (realm, mut label) in &mut parts.p1() {
            let current = points.map_or(0, |p| p.current[realm.0]);
            let value = format!("{current}/{}", maximum[realm.0]);
            if label.value != value {
                label.value = value;
            }
        }
    }
    let silenced = members
        .get(view.caster)
        .is_ok_and(|(_, v, _)| v.has(crate::game::characters::Condition::Silenced));
    let offers =
        offers(&view, character, carried, learned, points, spells, items, context, &pool, silenced);
    let Ok((scroll_value, scroll_range)) = sliders.get(scroll.0) else { return };
    let maximum = offers.len().saturating_sub(ROWS);
    set_scroll_maximum(&mut commands, scroll.0, scroll_value, scroll_range, maximum as f32);
    let offset = (scroll_value.0 as usize).min(maximum);
    let hovered = rows_hovered.iter().find(|(_, h)| h.get()).map(|(r, _)| r.0);
    for (row, mut visibility) in &mut parts.p2() {
        visibility.set_if_neq(if row.0 + offset < offers.len() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    let chosen = |offer: &Offer| match (view.source, offer.source) {
        (Some(Source::Spell { spell: a, .. }), Source::Spell { spell: b, .. }) => a == b,
        (Some(Source::Item(a)), Source::Item(b))
        | (Some(Source::PoolItem(a)), Source::PoolItem(b)) => a == b,
        _ => false,
    };
    let palette = |i: usize, offer: &Offer| {
        if chosen(offer) {
            Some(art.blue.clone())
        } else if offer.silenced {
            Some(art.orange.clone())
        } else if !offer.available {
            Some(art.red.clone())
        } else if hovered == Some(i) {
            Some(art.yellow.clone())
        } else {
            None
        }
    };
    for (row, mut label) in &mut parts.p3() {
        let (value, colour) = offers
            .get(row.0 + offset)
            .map_or((String::new(), None), |o| (o.label.clone(), palette(row.0, o)));
        if label.value != value {
            label.value = value;
        }
        if label.palette != colour {
            label.palette = colour;
        }
    }
    for (row, mut label) in &mut parts.p4() {
        let (value, colour) = offers
            .get(row.0 + offset)
            .map_or((String::new(), None), |o| (o.cost.clone(), palette(row.0, o)));
        if label.value != value {
            label.value = value;
        }
        if label.palette != colour {
            label.palette = colour;
        }
    }
    let spell = offers.iter().find(|o| chosen(o)).map(|o| o.spell);
    // Pips: as many lit as the chosen power, rated by failure chance;
    // auto pips only where retail offers them.
    if !view.items {
        let affordable = spell.zip(points).map_or(0, |(s, p)| p.affordable_power(s));
        for (pip, hovered, mut picture, mut visibility) in &mut parts.p5() {
            let index = usize::from(pip.0);
            let shown = match (pip.0, spell.map(|s| s.power)) {
                (7, Some(PowerChoice::ChosenOrRepeat)) => context != UseContext::Combat,
                (8, Some(PowerChoice::Maximum)) => true,
                (7 | 8, _) => false,
                _ => true,
            };
            visibility.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
            let frame = match spell {
                Some(spell) if pip.0 >= 7 || pip.0 < affordable => {
                    let rated = if pip.0 >= 7 {
                        0
                    } else {
                        rating(crate::game::magic::fail_chance(character, spell, pip.0 + 1))
                    };
                    let lit = match view.power {
                        Some(AUTO_POWER) => pip.0 >= 7,
                        Some(power) => pip.0 < 7 && pip.0 < power,
                        None => false,
                    };
                    let base = (index + rated * 9) * 6;
                    match (lit, hovered.get()) {
                        (true, true) => base + 4,
                        (true, false) => base + 2,
                        (false, true) => base + 1,
                        (false, false) => base,
                    }
                }
                _ => index * 6 + 3,
            };
            if picture.frame != frame {
                picture.frame = frame;
            }
        }
        let cost = match (spell, view.power) {
            (Some(_), Some(AUTO_POWER)) => "?".to_owned(),
            (Some(spell), Some(power)) => spell.cost_at(power).to_string(),
            _ => String::new(),
        };
        for mut label in &mut parts.p6() {
            if label.value != cost {
                label.value.clone_from(&cost);
            }
        }
        // Maximum-power spells latch their auto pip (retail `SC:990-1002`).
        if spell.is_some_and(|s| s.power == PowerChoice::Maximum) && view.power.is_none() {
            view.power = Some(AUTO_POWER);
        }
        if spell.is_some_and(|s| s.power == PowerChoice::Fixed) && view.power.is_none() {
            view.power = Some(1);
        }
    } else {
        let icon = view
            .source
            .and_then(|s| match s {
                Source::Item(id) => carried?.find_stack(id),
                Source::PoolItem(id) => pool.pool.stacks.iter().find(|s| s.id == id),
                Source::Spell { .. } => None,
            })
            .and_then(|stack| items.get(stack.item));
        for (mut picture, mut visibility) in &mut parts.p7() {
            visibility.set_if_neq(if icon.is_some() {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
            if let Some(definition) = icon {
                let sheet = assets.load(format!("retail://{}", definition.image));
                if picture.sheet != sheet {
                    picture.sheet = sheet;
                }
            }
        }
        let caption = if view.pool {
            "Party pool"
        } else {
            tables.get(&art.strings).and_then(|t| t.get(0x77a)).unwrap_or("")
        };
        for mut label in &mut captions {
            if label.value != caption {
                label.value = caption.to_owned();
            }
        }
    }
    // Commit once the source, its power and its target are all chosen.
    let (Some(source), Some(spell)) = (view.source, spell) else { return };
    // An item casts at its own power.
    let power = if view.items { Some(1) } else { view.power };
    let Some(power) = power else { return };
    let aim = if let Some(attempt) = view.attempt {
        Aim::Device(attempt)
    } else {
        match spell.targeting {
            Targeting::Ally => match view.member {
                Some(member) => Aim::Member(member),
                None => return,
            },
            Targeting::Enemy | Targeting::EnemyGroup | Targeting::Cone | Targeting::Radius => {
                // Retail's auto-targeting: the nearest fighting monster ahead.
                let Ok(eye) = party.single() else { return };
                let ahead = eye.forward().with_y(0.0);
                let target = battle
                    .iter()
                    .flat_map(|b| b.1.into_iter().flat_map(|c| c.entities().iter().copied()))
                    .filter_map(|m| {
                        let (_, monster, at) = monsters.get(m).ok()?;
                        let to = (at.translation() - eye.translation()).with_y(0.0);
                        (!monster.is_dead()
                            && to.length() <= spell.reach.metres() + monster.body.radius)
                            .then_some((m, to.angle_between(ahead), to.length()))
                    });
                let nearest = target.min_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)));
                match nearest {
                    Some((monster, ..)) => Aim::Monster(monster),
                    None => Aim::Member(view.caster),
                }
            }
            _ => Aim::Member(view.caster),
        }
    };
    let source = match source {
        Source::Spell { spell, .. } => Source::Spell { spell, power },
        item => item,
    };
    requests.write(MagicRequest { caster: view.caster, choice: MagicChoice { source, aim } });
    commands.entity(root).despawn();
}

fn toggle_pool(_: On<Activate>, mut views: Query<&mut MagicView>) {
    if let Ok(mut view) = views.single_mut() {
        view.pool = !view.pool;
        view.source = None;
        view.power = None;
        view.member = None;
    }
}

fn choose_realm(
    click: On<Activate>,
    realms: Query<&RealmButton>,
    mut views: Query<&mut MagicView>,
) {
    if let (Ok(realm), Ok(mut view)) = (realms.get(click.entity), views.single_mut()) {
        view.realm = Some(realm.0);
        view.source = None;
        view.power = None;
        view.member = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn choose_row(
    click: On<Activate>,
    rows: Query<&Row>,
    mut views: Query<(&mut MagicView, &MagicScroll)>,
    sliders: Query<&SliderValue>,
    members: Query<(&Character, &Vitals, Option<&Carried>)>,
    casters: Query<(&LearnedSpells, Option<&SpellPoints>)>,
    battle: Option<Single<&Battle>>,
    (camp, interaction, pool): (
        Option<Res<crate::game::camp::Camp>>,
        Query<&crate::game::world::interaction::DeviceInteraction>,
        Single<&PartyInventory>,
    ),
    data: Option<Res<GameData>>,
    (spells, items): (Res<Assets<SpellCatalog>>, Res<Assets<ItemCatalog>>),
) {
    let (Ok(row), Ok((mut view, scroll))) = (rows.get(click.entity), views.single_mut()) else {
        return;
    };
    let (Some(spells), Some(items)) = (
        data.as_ref().and_then(|d| spells.get(&d.spells)),
        data.as_ref().and_then(|d| items.get(&d.items)),
    ) else {
        return;
    };
    let Ok((character, _, carried)) = members.get(view.caster) else { return };
    let (learned, points) = casters.get(view.caster).map_or((None, None), |(l, p)| (Some(l), p));
    let silenced = members
        .get(view.caster)
        .is_ok_and(|(_, v, _)| v.has(crate::game::characters::Condition::Silenced));
    let offers = offers(
        &view,
        character,
        carried,
        learned,
        points,
        spells,
        items,
        match context(&view, battle.is_some(), camp.is_some(), &interaction) {
            Some(c) => c,
            None => return,
        },
        &pool,
        silenced,
    );
    // Unavailable rows only explain themselves in retail; here they do nothing.
    if let Some(offer) = offers
        .get(
            row.0
                + sliders
                    .get(scroll.0)
                    .map_or(0, |v| v.0 as usize)
                    .min(offers.len().saturating_sub(ROWS)),
        )
        .filter(|o| o.available)
    {
        view.source = Some(offer.source);
        view.power = None;
        view.member = None;
    }
}

fn choose_power(click: On<Activate>, pips: Query<&Pip>, mut views: Query<&mut MagicView>) {
    let (Ok(pip), Ok(mut view)) = (pips.get(click.entity), views.single_mut()) else { return };
    if view.source.is_none() {
        return;
    }
    let power = if pip.0 >= 7 { AUTO_POWER } else { (pip.0 + 1).min(MAX_POWER) };
    // Clicking the lit pip again takes the choice back.
    view.power = if view.power == Some(power) { None } else { Some(power) };
}

fn cancel_view(_: On<Activate>, views: Query<Entity, With<MagicView>>, mut commands: Commands) {
    for view in &views {
        commands.entity(view).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::test_input::{input_app, tap};

    #[derive(Component, Clone, Default)]
    struct Beneath;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct LeaveBeneath;

    #[derive(Resource, Default)]
    struct Left(u32);

    /// Escape closes the view and nothing beneath it (a camp's Escape ends
    /// the rest), and focus goes back to what had it, such as a device
    /// panel's button, rather than staying on a hidden control.
    #[test]
    fn escape_closes_only_the_view_and_gives_focus_back() {
        let mut app = input_app();
        app.add_plugins((
            magic_keys,
            bevy::input_focus::InputDispatchPlugin,
            bevy::input_focus::tab_navigation::TabNavigationPlugin,
        ))
        .add_input_context::<Beneath>()
        .init_asset::<SpriteSheet>()
        .init_asset::<TextTable>()
        .init_resource::<Left>()
        .insert_resource(MagicViewArt {
            spell_panels: default(),
            realm_buttons: default(),
            realms: default(),
            pips: default(),
            cancel: default(),
            scroll: default(),
            item_panels: default(),
            font: default(),
            red: default(),
            orange: default(),
            blue: default(),
            yellow: default(),
            strings: default(),
        });
        app.finish();
        app.cleanup();
        let window = app.world_mut().spawn((Window::default(), bevy::window::PrimaryWindow)).id();
        app.world_mut().commands().spawn_scene(bsn! {
            Beneath
            ContextPriority::<Beneath>::new(1)
            Actions<Beneath> [
                Action::<LeaveBeneath>::new()
                ActionSettings { consume_input: true }
                Bindings [ Binding(KeyCode::Escape) ]
            ]
            on(|_: On<Start<LeaveBeneath>>, mut left: ResMut<Left>| left.0 += 1)
        });
        let before = app.world_mut().spawn((UiButton, TabIndex(0))).id();
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(before, bevy::input_focus::FocusCause::Navigated);
        let caster = app.world_mut().spawn_empty().id();
        let art = app.world().resource::<MagicViewArt>().clone();
        open(&mut app.world_mut().commands(), &art, caster, false, None);
        app.update();
        app.update();
        let focused = app.world().resource::<InputFocus>().get().unwrap();
        assert_ne!(focused, before, "the view takes focus");
        assert_eq!(app.world_mut().query::<&MagicView>().iter(app.world()).count(), 1);
        tap(&mut app, window, KeyCode::Escape);
        app.update();
        assert_eq!(app.world_mut().query::<&MagicView>().iter(app.world()).count(), 0);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(before));
        assert_eq!(app.world().resource::<Left>().0, 0, "what lies beneath keeps its Escape");
    }
}
