//! The radar (retail `MGSRadarMap.cpp`): a disc and view cone 91 pixels
//! square from (30, 359), the blips of monsters and lying items, and the
//! compass rim on top, turned with the party. Clicking it opens the
//! automap.

use crate::game::world::obstacles::WorldObstacles;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use super::{MainScreenArt, layer};
use crate::presentation::automap::OpenAutomap;
use crate::presentation::party_view::PartyCamera;
use crate::presentation::screen_art::{absolute, picture};

pub fn panel(art: &MainScreenArt) -> impl SceneList {
    let at = Vec2::new(30.0, 359.0);
    let disc = picture(art.radar_disc.clone(), 0, at);
    let cone = picture(art.view_cone.clone(), 0, at);
    let rim = picture(art.radar_rim.clone(), 0, at);
    // Left-clicking the radar opens the automap (retail region 0x62, a
    // circle of radius 46 about (75, 402)).
    let disc_area = Node {
        border_radius: BorderRadius::all(Val::Px(46.0)),
        ..absolute(Vec2::new(29.0, 356.0), Vec2::splat(92.0))
    };
    bsn_list! {
        @{disc}
        -- @{cone}
        -- @layer() RadarBlips
        -- @{rim} RadarRim UiTransform
        -- disc_area bevy::ui_widgets::Button bevy::input_focus::tab_navigation::TabIndex(0) crate::presentation::retail_controls::FocusRing
        on(|_: On<Activate>, mut open: MessageWriter<OpenAutomap>| {
                open.write(OpenAutomap);
            })
    }
}

/// The radar's compass rim, turned with the party.
#[derive(Component, Clone, Default)]
pub struct RadarRim;

/// Holds the radar's blips.
#[derive(Component, Clone, Default)]
pub struct RadarBlips;

/// The UI hierarchy owns placement; the target relationship owns validity.
/// Despawning either the screen or the target removes its blip automatically.
#[derive(Component)]
#[relationship(relationship_target = RadarView)]
pub(super) struct RadarBlip(Entity);

#[derive(Component)]
#[relationship_target(relationship = RadarBlip, linked_spawn)]
pub(super) struct RadarView(Entity);

/// One class of radar blip's colours (retail `g_radar_blip_colors`); retail
/// has two more classes (unseen, missile) not drawn yet.
#[derive(Clone, Copy)]
struct BlipColours {
    /// More than 3 m below the party.
    below: Color,
    /// Within 3 m of the party's height.
    level: Color,
    /// More than 3 m above the party.
    above: Color,
}

impl BlipColours {
    /// The colour for a blip `height` metres above the party.
    fn at(self, height: f32) -> Color {
        match height {
            h if h.abs() < 3.0 => self.level,
            h if h > 0.0 => self.above,
            _ => self.below,
        }
    }
}

const NEUTRAL_BLIPS: BlipColours = BlipColours {
    below: Color::srgb(0.5, 0.5, 0.0),
    level: Color::srgb(0.7, 0.7, 0.0),
    above: Color::srgb(1.0, 1.0, 0.0),
};
const HOSTILE_BLIPS: BlipColours = BlipColours {
    below: Color::srgb(0.5, 0.0, 0.0),
    level: Color::srgb(0.7, 0.0, 0.0),
    above: Color::srgb(1.0, 0.0, 0.0),
};
const FRIENDLY_BLIPS: BlipColours = BlipColours {
    below: Color::srgb(0.0, 0.5, 0.0),
    level: Color::srgb(0.0, 0.7, 0.0),
    above: Color::srgb(0.0, 1.0, 0.0),
};
const ITEM_BLIPS: BlipColours = BlipColours {
    below: Color::srgb(0.5, 0.5, 0.5),
    level: Color::srgb(0.7, 0.7, 0.7),
    above: Color::srgb(1.0, 1.0, 1.0),
};

/// A monster's blip class by its own stance (retail
/// `g_radar_disposition_class`).
fn blip_class(stance: Option<crate::game::world::hostility::Stance>) -> BlipColours {
    use crate::game::world::hostility::Stance;
    match stance.unwrap_or_default() {
        Stance::Neutral => NEUTRAL_BLIPS,
        Stance::Hostile => HOSTILE_BLIPS,
        Stance::Friendly => FRIENDLY_BLIPS,
    }
}

/// Pixels from the radar's centre to the party's radius.
const PARTY_EDGE: f32 = 2.0;
/// Pixels from the radar's centre to its rim, at extreme range.
const RIM: f32 = 38.0;

/// Where a blip `delta` away from the party goes on the radar, for a party
/// facing `yaw` (retail `PlaceRadarBlip`, zoomed out: 2 pixels at the
/// party's radius, growing to the 38-pixel rim at extreme range). Inside
/// the party's radius monsters sit on its edge and items (`spread`) are
/// spread across it.
fn blip_position(delta: Vec3, yaw: f32, spread: bool) -> Vec2 {
    let inner = 1.0;
    let outer = 50.0 + inner;
    // Right and forward: in the party's frame forward is -Z.
    let local = Quat::from_rotation_y(-yaw) * delta;
    let flat = Vec2::new(local.x, -local.z);
    let distance = flat.length();
    let pixels = if distance <= inner {
        if spread { PARTY_EDGE * distance / inner } else { PARTY_EDGE }
    } else {
        (PARTY_EDGE + (distance - inner) * (RIM - PARTY_EDGE) / (outer - inner)).min(RIM)
    };
    let unit = flat.normalize_or_zero();
    Vec2::new(75.0 + unit.x * pixels, 404.0 - unit.y * pixels)
}

/// What the radar knows of lying items (retail's `RADAR_SEEN` flag, cleared
/// whenever a level is entered, and `LightRadarBlip`'s 10-second glow for
/// a find).
#[derive(Component, Default)]
pub struct RadarItem {
    seen: bool,
    glow: Option<Timer>,
}

const ITEM_GLOW_SECONDS: f32 = 10.0;

/// Presentation memory belongs to the item and survives rebuilding the HUD,
/// but disappears with the item or level. It is never part of a save record.
pub fn track_radar_items(
    time: Res<Time>,
    mut items: Query<&mut RadarItem, With<crate::game::world::items::WorldItem>>,
    mut news: MessageReader<crate::game::world::search::SearchNews>,
) {
    for mut known in &mut items {
        if known.glow.is_none() {
            continue;
        }
        if known.glow.as_mut().is_some_and(|glow| glow.tick(time.delta()).is_finished()) {
            known.glow = None;
        }
    }
    for notice in news.read() {
        let crate::game::world::search::SearchNews::Spotted { target, .. } = notice else {
            continue;
        };
        let Ok(mut known) = items.get_mut(*target) else { continue };
        known.glow = Some(Timer::from_seconds(ITEM_GLOW_SECONDS, TimerMode::Once));
    }
}

/// Turn the compass rim with the party, and mark the monsters within the
/// far clip by how each stands now: hostile red, friendly green, neutral
/// yellow, brighter above and dimmer below;
/// and the items once seen, grey to white.
pub fn update_radar(
    mut commands: Commands,
    party: Query<(
        &avian3d::prelude::Position,
        &crate::game::party::body::PartyBody,
        Option<&crate::game::magic::enchantments::PartyEffects>,
        Option<&crate::game::party::roster::PartyRoster>,
    )>,
    camera: Query<(&Projection, &GlobalTransform), With<PartyCamera>>,
    mut rims: Query<&mut UiTransform, With<RadarRim>>,
    layers: Query<Entity, With<RadarBlips>>,
    members: Query<(&crate::game::characters::Character, &crate::game::characters::Vitals)>,
    monsters: Query<(
        Entity,
        &crate::game::world::population::Monster,
        crate::game::world::ActorPose,
        Option<&crate::game::world::navigation::Walker>,
        Has<crate::game::combat::Combatant>,
        &crate::game::world::level::LevelOwned,
    )>,
    views: Query<&RadarView>,
    mut blips: Query<(Entity, &mut Node, &mut BackgroundColor, &mut Visibility), With<RadarBlip>>,
    (mut items, obstacles): (
        Query<(Entity, &crate::game::world::items::WorldItem, &GlobalTransform, &mut RadarItem)>,
        WorldObstacles,
    ),
    (content, stances, clock): (
        If<Res<crate::game::data::Catalogs>>,
        crate::game::world::hostility::Stances,
        Res<crate::game::time::GameClock>,
    ),
    (search, battle, levels): (
        Option<Res<crate::game::world::search::Search>>,
        Query<(), With<crate::game::combat::Battle>>,
        Query<(), With<crate::game::world::level::Level>>,
    ),
) {
    let (Ok((position, body, effects, roster)), Ok(layer)) = (party.single(), layers.single())
    else {
        return;
    };
    for mut rim in &mut rims {
        // North stays put while the view turns.
        let rotation = Rot2::radians(body.yaw);
        if rim.rotation != rotation {
            rim.rotation = rotation;
        }
    }
    let (far, eye) = match camera.single() {
        Ok((p, eye)) => (bevy::camera::CameraProjection::far(&**p), eye.translation()),
        _ => (0.0, position.0),
    };
    let catalog = &content.species;
    let xray = effects.is_some_and(|effects| effects.0.has(crate::content::spells::SpellId::X_RAY));
    // Each blip's visibility is decided once, so one that stays in view
    // stays clean.
    let mut shown = bevy::ecs::entity::EntityHashSet::default();
    let mut show = |target: Entity, at: Vec2, colour: Color| {
        if let Ok(view) = views.get(target)
            && let Ok((_, mut node, mut color, mut visibility)) = blips.get_mut(view.0)
        {
            if (node.left, node.top) != (px(at.x), px(at.y)) {
                (node.left, node.top) = (px(at.x), px(at.y));
            }
            color.set_if_neq(BackgroundColor(colour));
            visibility.set_if_neq(Visibility::Inherited);
            shown.insert(view.0);
        } else {
            commands.spawn((
                RadarBlip(target),
                absolute(at, Vec2::splat(2.0)),
                BackgroundColor(colour),
                Pickable::IGNORE,
                ChildOf(layer),
            ));
        }
    };
    let view = crate::game::world::perception::View {
        eye,
        forward: body.forward(),
        far,
        searching: search.as_ref().is_some_and(|s| s.on(!battle.is_empty())),
    };
    let shadow_hound =
        effects.is_some_and(|effects| effects.0.has(crate::content::spells::SpellId::SHADOW_HOUND));
    for (entity, monster, at, walker, in_combat, owner) in &monsters {
        let Some(roster) = roster else { continue };
        let species = &catalog[monster.species];
        let feet = crate::game::world::navigation::feet(at.translation(), walker);
        if !levels.contains(owner.0) || !view.in_range(monster, feet) {
            continue;
        }
        if !xray
            && !view.sees(
                members.iter_many(roster.iter()).matched(),
                shadow_hound,
                monster,
                species,
                feet,
                in_combat,
                &obstacles,
            )
        {
            continue;
        }
        let delta = at.translation() - position.0;
        let class = blip_class(stances.own(entity, catalog, clock.ms));
        let at = blip_position(delta, body.yaw, false) - Vec2::ONE;
        show(entity, at, class.at(delta.y));
    }
    // Items: once the eye has had them in sight, within the far clip.
    for (entity, item, at, mut known) in &mut items {
        if item.hidden {
            continue;
        }
        let delta = at.translation() - position.0;
        if delta.length() > far {
            continue;
        }
        if !known.seen {
            let to = at.translation();
            let in_sight = obstacles.clear(eye, to);
            if !xray && !in_sight {
                continue;
            }
            known.seen = true;
        }
        // Retail makes a find's blip glow between `below` and `above`;
        // here it shows `above`.
        let colour = if known.glow.is_some() { ITEM_BLIPS.above } else { ITEM_BLIPS.at(delta.y) };
        let at = blip_position(delta, body.yaw, true) - Vec2::ONE;
        show(entity, at, colour);
    }
    for (blip, _, _, mut visible) in &mut blips {
        if !shown.contains(&blip) {
            visible.set_if_neq(Visibility::Hidden);
        }
    }
}
