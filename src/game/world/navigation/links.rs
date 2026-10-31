//! Off-mesh links: teleports, climbs and doorways, and how walkers use them.

use crate::game::world::level::LevelOwned;
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::UsingAnimationLink;
use bevy_landmass::prelude::*;

use super::LevelNavigation;
use super::bake::SIZE_CLASSES;
use crate::content::level::{LevelDefinition, LevelLink, LinkKind, PropModel};

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
            kind: link.kind.into(),
            cost: 1.0,
            bidirectional: link.kind == LinkKind::Door,
        },
        archipelago_ref: ArchipelagoRef3d::new(archipelago),
    }
}

/// Spawn a level's teleport and climb links in each of its archipelagos.
pub fn spawn_level_links(
    commands: &mut Commands,
    root: Entity,
    geometry: &LevelDefinition,
    archipelagos: &[Entity],
) {
    for link in &geometry.links {
        for (class, &archipelago) in archipelagos.iter().enumerate() {
            commands.spawn((
                Name::new(format!("{:?} link", link.kind)),
                link_bundle(link, archipelago, SIZE_CLASSES[class]),
                LevelOwned(root),
            ));
        }
    }
    info!("{} navigation links", geometry.links.len());
}

/// The links through a door's doorway, one per size class, present only
/// while the door is open.
#[derive(Component, Debug)]
pub struct DoorLink {
    /// The doorway's floor centre.
    centre: Vec3,
    /// From the centre to one face of the closed door.
    half_thickness: Vec3,
}

impl DoorLink {
    /// The doorway of a door whose closed shape covers `points` (world
    /// space): crossed along its thinnest horizontal axis, at its floor.
    pub fn around(points: impl IntoIterator<Item = Vec3>) -> Option<Self> {
        let (min, max) = points.into_iter().fold(None, |bounds: Option<(Vec3, Vec3)>, p| {
            Some(bounds.map_or((p, p), |(min, max)| (min.min(p), max.max(p))))
        })?;
        let size = max - min;
        let across = if size.x < size.z { Vec3::X * size.x } else { Vec3::Z * size.z };
        let centre = ((min + max) / 2.0).with_y(min.y);
        Some(DoorLink { centre, half_thickness: across / 2.0 })
    }

    /// The doorway crossing for walkers of `radius`: from a radius (and a
    /// margin) beyond one face to the same beyond the other.
    fn link(&self, radius: f32) -> LevelLink {
        let offset = self.half_thickness + self.half_thickness.normalize() * (radius + 0.3);
        LevelLink { kind: LinkKind::Door, from: self.centre + offset, to: self.centre - offset }
    }
}

/// Keep each door's link present exactly while the door is open.
pub fn sync_door_links(
    mut commands: Commands,
    doors: Query<
        (Entity, &Prop, &LevelOwned, &PropMotion, &DoorLink, Option<&Children>),
        Or<(Changed<PropMotion>, Added<DoorLink>)>,
    >,
    definitions: crate::game::world::level::OwnedDefinitions,
    links: Query<(), With<AnimationLink3d>>,
    navigation: Query<&LevelNavigation>,
) {
    let Ok(navigation) = navigation.single() else { return };
    for (entity, prop, owner, motion, door, children) in &doors {
        let last = prop.model(owner, &definitions).map_or(0, PropModel::last_frame);
        let open = last > 0 && motion.frame >= last as f32 * 0.5;
        let existing =
            children.into_iter().flatten().copied().filter(|&child| links.contains(child));
        if open && existing.clone().next().is_none() {
            for (class, &archipelago) in navigation.archipelagos.iter().enumerate() {
                let radius = SIZE_CLASSES[class];
                commands.spawn((
                    Name::new("Door link"),
                    link_bundle(&door.link(radius), archipelago, radius),
                    ChildOf(entity),
                ));
            }
        } else if !open {
            for link in existing {
                commands.entity(link).despawn();
            }
        }
    }
}

/// A walker crossing a door's link, toward its far side, paused by
/// navigation while it does. It ends when the walker gets there, and with
/// the link: a door that closes despawns its links, and the walker then
/// finds its way again.
#[derive(Component, Debug)]
#[require(UsingAnimationLink)]
#[relationship(relationship_target = Crossers)]
pub struct Crossing {
    #[relationship]
    pub link: Entity,
    pub to: Vec3,
}

/// The walkers crossing a link.
#[derive(Component, Debug)]
#[relationship_target(relationship = Crossing)]
pub struct Crossers(Vec<Entity>);

/// A crossing that ends, however it ends, hands the walker back to its
/// navigation.
pub fn end_crossing(remove: On<Remove<Crossing>>, mut commands: Commands) {
    if let Ok(mut walker) = commands.get_entity(remove.entity) {
        walker.try_remove::<UsingAnimationLink>();
    }
}

/// Start crossing a link the walker has reached: teleports and climbs
/// arrive at once (retail flies the party along a climb's camera path;
/// monsters do not climb in retail at all, so this is provisional), doors
/// are walked through.
pub fn use_reached_links(
    mut commands: Commands,
    walkers: Query<
        (
            Entity,
            &ReachedAnimationLink3d,
            &super::Walker,
            &bevy_landmass::prelude::AgentTarget3d,
            Option<&crate::game::world::population::Monster>,
        ),
        (Without<UsingAnimationLink>, Without<Crossing>),
    >,
    links: Query<&AnimationLink3d>,
    mut bodies: Query<(&mut Position, &mut Transform)>,
) {
    for (entity, reached, walker, target, monster) in &walkers {
        if !super::walking(target, monster) {
            continue;
        }
        let Some(kind) =
            links.get(reached.link_entity).ok().and_then(|l| LinkKind::try_from(l.kind).ok())
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
                    .insert(Crossing { link: reached.link_entity, to: reached.end_point });
            }
        }
    }
}

/// Finish a crossing once the walker reaches the far side.
pub fn finish_crossings(
    mut commands: Commands,
    walkers: Query<(
        Entity,
        &Crossing,
        crate::game::world::ActorPose,
        &super::Walker,
        Option<&crate::game::world::population::Monster>,
    )>,
) {
    for (entity, crossing, pose, walker, monster) in &walkers {
        let feet = super::feet(pose.translation(), Some(walker));
        if monster.is_some_and(|m| m.is_dead())
            || feet.with_y(0.0).distance(crossing.to.with_y(0.0)) < 0.4
        {
            commands.entity(entity).remove_with_requires::<Crossing>();
        }
    }
}
