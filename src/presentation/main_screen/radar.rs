//! The radar (retail `MGSRadarMap.cpp`): a disc and view cone 91 pixels
//! square from (30, 359), the blips of monsters and lying items, and the
//! compass rim on top, turned with the party. Clicking it opens the
//! automap.

use bevy::ecs::entity::{EntityHashMap, EntityHashSet};
use bevy::prelude::*;

use super::{MainScreenArt, layer, loaded, put};
use crate::content::ui::SpriteSheet;
use crate::game::data::GameData;
use crate::game::party::body::PartyCamera;
use crate::presentation::automap::OpenAutomap;
use crate::presentation::screen_art::{absolute, sprite};

pub fn spawn(
    commands: &mut Commands,
    frame: Entity,
    art: &MainScreenArt,
    sheets: &Assets<SpriteSheet>,
) {
    let at = Vec2::new(30.0, 359.0);
    put(commands, frame, sprite(loaded(sheets, &art.radar_disc), 0, at));
    put(commands, frame, sprite(loaded(sheets, &art.view_cone), 0, at));
    layer(commands, frame, RadarBlips);
    if let Some(mut rim) = put(commands, frame, sprite(loaded(sheets, &art.radar_rim), 0, at)) {
        rim.insert((RadarRim, UiTransform::default()));
    }
    // Left-clicking the radar opens the automap (retail region 0x62, a
    // circle of radius 46 about (75, 402)).
    commands
        .spawn((
            Node {
                border_radius: BorderRadius::all(Val::Px(46.0)),
                ..absolute(Vec2::new(29.0, 356.0), Vec2::splat(92.0))
            },
            ChildOf(frame),
        ))
        .observe(|_: On<PointerClick>, mut open: MessageWriter<OpenAutomap>| {
            open.write(OpenAutomap);
        });
}

/// The radar's compass rim, turned with the party.
#[derive(Component)]
pub struct RadarRim;

/// Holds the radar's blips.
#[derive(Component)]
pub struct RadarBlips;

/// One class of radar blip's colours (retail `g_radar_blip_colors`); retail
/// has three more classes (friendly, unseen, missile) not drawn yet.
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
const ITEM_BLIPS: BlipColours = BlipColours {
    below: Color::srgb(0.5, 0.5, 0.5),
    level: Color::srgb(0.7, 0.7, 0.7),
    above: Color::srgb(1.0, 1.0, 1.0),
};

/// Where a blip `delta` away from the party goes on the radar, for a party
/// facing `yaw` (retail `PlaceRadarBlip`, zoomed out: 2 pixels at the
/// party's radius, growing to the 38-pixel rim at extreme range). Inside
/// the party's radius monsters sit on its edge and items (`spread`) are
/// spread across it.
fn blip_position(delta: Vec3, yaw: f32, spread: bool) -> Vec2 {
    const SCALE: f32 = 2.0;
    const RIM: f32 = 38.0;
    let inner = 1.0;
    let outer = 50.0 + inner;
    // Right and forward: in the party's frame forward is -Z.
    let local = Quat::from_rotation_y(-yaw) * delta;
    let flat = Vec2::new(local.x, -local.z);
    let distance = flat.length();
    let pixels = if distance <= inner {
        if spread { SCALE * distance / inner } else { SCALE }
    } else {
        (SCALE + (distance - inner) * (RIM - SCALE) / (outer - inner)).min(RIM)
    };
    let unit = flat.normalize_or_zero();
    Vec2::new(75.0 + unit.x * pixels, 404.0 - unit.y * pixels)
}

/// What the radar knows of lying items (retail's `RADAR_SEEN` flag, cleared
/// whenever a level is entered, and `LightRadarBlip`'s 10-second glow for
/// a find).
#[derive(Default)]
pub struct RadarItems {
    seen: EntityHashSet,
    hidden: EntityHashSet,
    /// Glowing while its timer runs.
    lit: EntityHashMap<Timer>,
}

const ITEM_GLOW_SECONDS: f32 = 10.0;

/// Turn the compass rim with the party, and mark the monsters within the
/// far clip: hostile red, others yellow, brighter above and dimmer below;
/// and the items once seen, grey to white.
pub fn update_radar(
    mut commands: Commands,
    party: Query<(&avian3d::prelude::Position, &crate::game::party::body::PartyBody)>,
    camera: Query<(&Projection, &GlobalTransform), With<PartyCamera>>,
    mut rims: Query<&mut UiTransform, With<RadarRim>>,
    layers: Query<Entity, With<RadarBlips>>,
    monsters: Query<(&crate::game::world::population::Monster, &GlobalTransform)>,
    (items, time, spatial, bodies): (
        Query<(Entity, &crate::game::world::items::WorldItem, &GlobalTransform)>,
        Res<Time>,
        avian3d::prelude::SpatialQuery,
        Query<&avian3d::prelude::RigidBody>,
    ),
    data: Option<Res<GameData>>,
    species: Res<Assets<crate::content::species::MonsterCatalog>>,
    mut known: Local<RadarItems>,
) {
    let (Ok((position, body)), Ok(layer)) = (party.single(), layers.single()) else { return };
    for mut rim in &mut rims {
        // North stays put while the view turns.
        rim.rotation = Rot2::radians(body.yaw);
    }
    let (far, eye) = match camera.single() {
        Ok((Projection::Perspective(p), eye)) => (p.far, eye.translation()),
        _ => (0.0, position.0),
    };
    let catalog = data.and_then(|d| species.get(&d.species));
    commands.entity(layer).despawn_related::<Children>();
    for (monster, at) in &monsters {
        let delta = at.translation() - position.0;
        if monster.is_dead() || delta.length() > far {
            continue;
        }
        let hostile =
            catalog.and_then(|c| c.0.get(monster.species as usize)).is_some_and(|s| !s.npc);
        let class = if hostile { HOSTILE_BLIPS } else { NEUTRAL_BLIPS };
        commands.spawn((
            absolute(blip_position(delta, body.yaw, false) - Vec2::ONE, Vec2::splat(2.0)),
            BackgroundColor(class.at(delta.y)),
            Pickable::IGNORE,
            ChildOf(layer),
        ));
    }
    // Items: once the eye has had them in sight, within the far clip.
    let level = |entity| bodies.get(entity).is_ok_and(avian3d::prelude::RigidBody::is_static);
    let filter = avian3d::prelude::SpatialQueryFilter::default();
    known.seen.retain(|e| items.contains(*e));
    known.lit.retain(|e, glow| items.contains(*e) && !glow.tick(time.delta()).is_finished());
    for (entity, item, at) in &items {
        if item.hidden {
            known.hidden.insert(entity);
            continue;
        }
        if known.hidden.remove(&entity) {
            known.lit.insert(entity, Timer::from_seconds(ITEM_GLOW_SECONDS, TimerMode::Once));
        }
        let delta = at.translation() - position.0;
        if delta.length() > far {
            continue;
        }
        if !known.seen.contains(&entity) {
            let to = at.translation();
            let in_sight = Dir3::new(to - eye).is_ok_and(|direction| {
                spatial
                    .cast_ray_predicate(eye, direction, eye.distance(to), true, &filter, &level)
                    .is_none()
            });
            if !in_sight {
                continue;
            }
            known.seen.insert(entity);
        }
        // Retail makes a find's blip glow between `below` and `above`;
        // here it shows `above`.
        let colour =
            if known.lit.contains_key(&entity) { ITEM_BLIPS.above } else { ITEM_BLIPS.at(delta.y) };
        commands.spawn((
            absolute(blip_position(delta, body.yaw, true) - Vec2::ONE, Vec2::splat(2.0)),
            BackgroundColor(colour),
            Pickable::IGNORE,
            ChildOf(layer),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blips_sit_ahead_and_beyond_on_the_rim() {
        // Ahead of a party facing -Z, 10 m off: straight up from the centre.
        let ahead = blip_position(Vec3::new(0.0, 0.0, -10.0), 0.0, false);
        assert_eq!(ahead.x, 75.0);
        assert!(ahead.y < 404.0 && ahead.y > 404.0 - 38.0);
        // Far beyond extreme range: on the 38-pixel rim, to the right.
        assert_eq!(blip_position(Vec3::new(200.0, 0.0, 0.0), 0.0, false), Vec2::new(113.0, 404.0));
        // Within the party's radius a monster sits on its edge, an item
        // where it is.
        let near = Vec3::new(0.5, 0.0, 0.0);
        assert_eq!(blip_position(near, 0.0, false), Vec2::new(77.0, 404.0));
        assert_eq!(blip_position(near, 0.0, true), Vec2::new(76.0, 404.0));
    }
}
