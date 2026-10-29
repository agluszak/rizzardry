//! The radar (retail `MGSRadarMap.cpp`): a disc and view cone 91 pixels
//! square from (30, 359), the blips of monsters and lying items, and the
//! compass rim on top, turned with the party. Clicking it opens the
//! automap.

use crate::game::world::obstacles::WorldObstacles;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use super::{MainScreenArt, layer, loaded};
use crate::content::ui::SpriteSheet;
use crate::presentation::automap::OpenAutomap;
use crate::presentation::party_view::PartyCamera;
use crate::presentation::screen_art::{absolute, sprite};

pub fn panel(art: &MainScreenArt, sheets: &Assets<SpriteSheet>) -> impl SceneList {
    let at = Vec2::new(30.0, 359.0);
    let disc = sprite(loaded(sheets, &art.radar_disc), 0, at);
    let cone = sprite(loaded(sheets, &art.view_cone), 0, at);
    let rim = sprite(loaded(sheets, &art.radar_rim), 0, at);
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
/// far clip: hostile red, others yellow, brighter above and dimmer below;
/// and the items once seen, grey to white.
pub fn update_radar(
    mut commands: Commands,
    party: Query<(&avian3d::prelude::Position, &crate::game::party::body::PartyBody)>,
    camera: Query<(&Projection, &GlobalTransform), With<PartyCamera>>,
    mut rims: Query<&mut UiTransform, With<RadarRim>>,
    layers: Query<Entity, With<RadarBlips>>,
    monsters: Query<(Entity, &crate::game::world::population::Monster, &GlobalTransform)>,
    views: Query<&RadarView>,
    mut blips: Query<(Entity, &mut Node, &mut BackgroundColor, &mut Visibility), With<RadarBlip>>,
    (mut items, obstacles): (
        Query<(Entity, &crate::game::world::items::WorldItem, &GlobalTransform, &mut RadarItem)>,
        WorldObstacles,
    ),
    content: crate::game::data::Content,
) {
    let (Ok((position, body)), Ok(layer)) = (party.single(), layers.single()) else { return };
    for mut rim in &mut rims {
        // North stays put while the view turns.
        let rotation = Rot2::radians(body.yaw);
        if rim.rotation != rotation {
            rim.rotation = rotation;
        }
    }
    let (far, eye) = match camera.single() {
        Ok((Projection::Perspective(p), eye)) => (p.far, eye.translation()),
        _ => (0.0, position.0),
    };
    let catalog = content.species();
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
    for (entity, monster, at) in &monsters {
        let delta = at.translation() - position.0;
        if monster.is_dead() || delta.length() > far {
            continue;
        }
        let hostile =
            catalog.and_then(|c| c.kind(monster.species)).is_some_and(|s| s.npc.is_none());
        let class = if hostile { HOSTILE_BLIPS } else { NEUTRAL_BLIPS };
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
            let in_sight = Dir3::new(to - eye).is_ok_and(|direction| {
                obstacles.first_hit(eye, direction, eye.distance(to)).is_none()
            });
            if !in_sight {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_glows_even_when_the_hud_has_not_been_built() {
        use crate::game::inventory::{ItemIds, make_item};
        use crate::game::rng::GameRng;
        use crate::game::world::{items::WorldItem, search::SearchNews};
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .register_required_components::<WorldItem, RadarItem>()
            .add_message::<SearchNews>()
            .add_systems(Update, track_radar_items);
        let item =
            make_item(&default(), &mut ItemIds::default(), &mut GameRng::default(), true, true);
        let target = app.world_mut().spawn(WorldItem::loose(item, false)).id();
        assert!(app.world().get::<RadarItem>(target).is_some(), "attached at insertion");
        app.world_mut().get_mut::<RadarItem>(target).unwrap().seen = true;
        // Replacing the item must not overwrite its presentation history.
        let replacement = app.world().get::<WorldItem>(target).unwrap().clone();
        app.world_mut().entity_mut(target).insert(replacement);
        assert!(app.world().get::<RadarItem>(target).unwrap().seen);
        app.world_mut().write_message(SearchNews::Spotted {
            target,
            member: "Finder".into(),
            item: "Dagger".into(),
        });
        app.update();
        assert!(app.world().get::<RadarItem>(target).unwrap().glow.is_some());
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs(11),
        ));
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .set_max_delta(std::time::Duration::from_secs(11));
        app.update();
        assert!(app.world().get::<RadarItem>(target).unwrap().glow.is_none());
    }

    #[test]
    fn blips_sit_ahead_and_beyond_on_the_rim() {
        // An item exactly at the party lies at the centre.
        let centre = blip_position(Vec3::ZERO, 0.0, true);
        // Ahead of a party facing -Z, 10 m off: straight up, inside the rim.
        let ahead = blip_position(Vec3::new(0.0, 0.0, -10.0), 0.0, false) - centre;
        assert_eq!(ahead.x, 0.0);
        assert!(ahead.y < 0.0 && ahead.length() < RIM);
        // Far beyond extreme range: on the rim, to the right.
        assert_eq!(blip_position(Vec3::new(200.0, 0.0, 0.0), 0.0, false) - centre, RIM * Vec2::X);
        // Halfway inside the party's radius a monster sits on its edge, an
        // item where it is.
        let near = Vec3::new(0.5, 0.0, 0.0);
        assert_eq!(blip_position(near, 0.0, false) - centre, PARTY_EDGE * Vec2::X);
        assert_eq!(blip_position(near, 0.0, true) - centre, PARTY_EDGE / 2.0 * Vec2::X);
    }
}
