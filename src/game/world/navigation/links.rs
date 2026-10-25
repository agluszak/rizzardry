//! Off-mesh links: teleports, climbs and doorways, and how walkers use them.

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::UsingAnimationLink;
use bevy_landmass::prelude::*;

use super::LevelNavigation;
use super::bake::SIZE_CLASSES;
use crate::content::level::{LevelGeometry, LevelLink, LinkKind};
use crate::game::world::level::{AwaitingLinks, Level};
use crate::game::world::props::{Prop, PropMotion};

/// A short edge across `at`, perpendicular to `direction`: Landmass links
/// join edges, not points.
fn edge_across(at: Vec3, direction: Vec3, half_width: f32) -> (Vec3, Vec3) {
    let side = direction.with_y(0.0).normalize_or(Vec3::X).cross(Vec3::Y) * half_width;
    (at - side, at + side)
}

fn link_bundle(link: &LevelLink, archipelago: Entity, half_width: f32) -> AnimationLink3dBundle {
    let direction = link.to - link.from;
    AnimationLink3dBundle {
        link: AnimationLink3d {
            start_edge: edge_across(link.from, direction, half_width),
            end_edge: edge_across(link.to, direction, half_width),
            kind: link.kind.index(),
            cost: 1.0,
            bidirectional: link.kind == LinkKind::Door,
        },
        archipelago_ref: ArchipelagoRef3d::new(archipelago),
    }
}

/// Spawn a level's teleport and climb links once it has navigation.
pub fn spawn_level_links(
    mut commands: Commands,
    levels: Query<(Entity, &Level, &LevelNavigation), With<AwaitingLinks>>,
    geometry: Res<Assets<LevelGeometry>>,
) {
    for (root, level, navigation) in &levels {
        let Some(geometry) = geometry.get(&level.geometry) else { continue };
        commands.entity(root).remove::<AwaitingLinks>();
        for link in &geometry.links {
            for (class, &archipelago) in navigation.archipelagos.iter().enumerate() {
                commands.spawn((
                    Name::new(format!("{:?} link", link.kind)),
                    link_bundle(link, archipelago, SIZE_CLASSES[class]),
                    ChildOf(root),
                ));
            }
        }
        info!("{} navigation links", geometry.links.len());
    }
}

/// The links through a door's doorway, one per size class, present only
/// while the door is open.
#[derive(Component, Debug)]
pub struct DoorLink {
    /// The doorway's floor centre.
    centre: Vec3,
    /// From the centre to one face of the closed door.
    half_thickness: Vec3,
    spawned: Vec<Entity>,
}

impl DoorLink {
    /// The doorway crossing for walkers of `radius`: from a radius (and a
    /// margin) beyond one face to the same beyond the other.
    fn link(&self, radius: f32) -> LevelLink {
        let offset = self.half_thickness + self.half_thickness.normalize() * (radius + 0.3);
        LevelLink { kind: LinkKind::Door, from: self.centre + offset, to: self.centre - offset }
    }
}

/// Find each door's doorway once its closed collider exists. The doorway is
/// crossed along the door's thinnest horizontal axis.
pub fn find_door_links(
    mut commands: Commands,
    doors: Query<(Entity, &Prop, &Children), Without<DoorLink>>,
    bounds: Query<&ColliderAabb>,
) {
    for (entity, prop, children) in &doors {
        if !prop.door {
            continue;
        }
        let Some(aabb) = children.iter().find_map(|c| bounds.get(c).ok()) else { continue };
        if aabb.min.x > aabb.max.x {
            continue; // not computed yet
        }
        let (min, max) = (aabb.min, aabb.max);
        let size = max - min;
        let across = if size.x < size.z { Vec3::X * size.x } else { Vec3::Z * size.z };
        let centre = ((min + max) / 2.0).with_y(min.y);
        commands.entity(entity).insert(DoorLink {
            centre,
            half_thickness: across / 2.0,
            spawned: vec![],
        });
    }
}

/// Keep each door's link present exactly while the door is open.
pub fn sync_door_links(
    mut commands: Commands,
    mut doors: Query<(Entity, &Prop, &PropMotion, &mut DoorLink)>,
    navigation: Query<&LevelNavigation>,
) {
    let Ok(navigation) = navigation.single() else { return };
    for (entity, prop, motion, mut door) in &mut doors {
        let open = prop.last_frame > 0 && motion.frame >= prop.last_frame as f32 * 0.5;
        if open && door.spawned.is_empty() {
            for (class, &archipelago) in navigation.archipelagos.iter().enumerate() {
                let radius = SIZE_CLASSES[class];
                let link = commands
                    .spawn((
                        Name::new("Door link"),
                        link_bundle(&door.link(radius), archipelago, radius),
                        ChildOf(entity),
                    ))
                    .id();
                door.spawned.push(link);
            }
        } else if !open {
            for link in door.spawned.drain(..) {
                commands.entity(link).despawn();
            }
        }
    }
}

/// A walker crossing a link.
#[derive(Component, Debug)]
pub struct Crossing {
    pub to: Vec3,
}

/// Start crossing a link the walker has reached: teleports and climbs
/// arrive at once (retail flies the party along a climb's camera path;
/// monsters do not climb in retail at all, so this is provisional), doors
/// are walked through.
pub fn use_reached_links(
    mut commands: Commands,
    walkers: Query<
        (Entity, &ReachedAnimationLink3d, &super::Walker),
        (Without<UsingAnimationLink>, Without<Crossing>),
    >,
    links: Query<&AnimationLink3d>,
    mut bodies: Query<(&mut Position, &mut Transform)>,
) {
    for (entity, reached, walker) in &walkers {
        let Some(kind) =
            links.get(reached.link_entity).ok().and_then(|l| LinkKind::from_index(l.kind))
        else {
            continue;
        };
        match kind {
            LinkKind::Teleport | LinkKind::Climb => {
                if let Ok((mut position, mut transform)) = bodies.get_mut(entity) {
                    position.0 = reached.end_point + Vec3::Y * walker.float_height;
                    transform.translation = position.0;
                }
            }
            LinkKind::Door => {
                commands
                    .entity(entity)
                    .insert((UsingAnimationLink, Crossing { to: reached.end_point }));
            }
        }
    }
}

/// Finish a crossing once the walker reaches the far side.
pub fn finish_crossings(
    mut commands: Commands,
    walkers: Query<(Entity, &Crossing, &Transform, &super::Walker)>,
) {
    for (entity, crossing, transform, walker) in &walkers {
        let feet = transform.translation - Vec3::Y * walker.float_height;
        if feet.with_y(0.0).distance(crossing.to.with_y(0.0)) < 0.4 {
            commands.entity(entity).remove::<(Crossing, UsingAnimationLink)>();
        }
    }
}
