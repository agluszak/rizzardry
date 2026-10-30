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
    definitions: crate::game::world::level::PlayedLevel,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A door closing on a walker mid-crossing despawns the link, and the
    /// walker goes back to finding its way instead of walking into the door.
    /// A monster put to sleep mid-doorway stops moving at once; waking does
    /// not resume the move its sleep ended, while a new move resumes the
    /// crossing where it was. Asleep, it starts no new traversal.
    #[test]
    fn a_crossing_moves_its_walker_only_while_it_may_walk() {
        use crate::game::characters::{Condition, Lasting};
        use crate::game::world::population::{Monster, ScaledBody};
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.add_observer(end_crossing);
        let link = world.spawn_empty().id();
        let party = world.spawn_empty().id();
        let monster = Monster {
            enchantments: default(),
            location: 0,
            species: 0,
            hp: 10,
            max_hp: 10,
            stamina: 10,
            max_stamina: 10,

            conditions: Default::default(),
            spell_points: 0,
            body: ScaledBody { scale: 1.0, radius: 0.5, eye_height: 1.0, speed: 1.0 },
        };
        let walker = world
            .spawn((
                super::super::Walker { speed: 2.0, float_height: 1.0 },
                AgentTarget3d::Entity(party),
                monster,
                Transform::default(),
                bevy_tnua::prelude::TnuaController::<super::super::WalkerMotion>::default(),
                Crossing { link, to: Vec3::X * 4.0 },
            ))
            .id();
        let motion = |world: &mut World| {
            world.run_system_once(super::super::steer_walkers).unwrap();
            let controller =
                world.get::<bevy_tnua::prelude::TnuaController<super::super::WalkerMotion>>(walker);
            controller.unwrap().basis.desired_motion
        };
        assert!(motion(&mut world).x > 0.0, "crossing toward the far side");
        world
            .get_mut::<Monster>(walker)
            .unwrap()
            .conditions
            .inflict(Condition::Asleep, Lasting::Rounds(2));
        assert_eq!(motion(&mut world), Vec3::ZERO, "asleep, it stops in the doorway");
        // Its combat move ends with its sleep (`close_in`), clearing the target.
        *world.get_mut::<AgentTarget3d>(walker).unwrap() = AgentTarget3d::None;
        world.get_mut::<Monster>(walker).unwrap().conditions = Default::default();
        assert_eq!(motion(&mut world), Vec3::ZERO, "waking does not resume the ended move");
        assert!(world.get::<Crossing>(walker).is_some(), "its progress is kept");
        *world.get_mut::<AgentTarget3d>(walker).unwrap() = AgentTarget3d::Entity(party);
        assert!(motion(&mut world).x > 0.0, "a new move finishes the crossing");
        // Asleep at another link's start, it does not take it.
        world.entity_mut(walker).remove::<Crossing>();
        world.flush();
        world
            .get_mut::<Monster>(walker)
            .unwrap()
            .conditions
            .inflict(Condition::Asleep, Lasting::Rounds(2));
        world.entity_mut(walker).insert(ReachedAnimationLink3d {
            link_entity: link,
            start_point: Vec3::ZERO,
            end_point: Vec3::X * 4.0,
        });
        let door = LevelLink { kind: LinkKind::Door, from: Vec3::ZERO, to: Vec3::X * 4.0 };
        world.entity_mut(link).insert(link_bundle(&door, Entity::PLACEHOLDER, 0.5).link);
        world.run_system_once(use_reached_links).unwrap();
        world.flush();
        assert!(world.get::<Crossing>(walker).is_none());
        // Awake and moving, it does.
        world.get_mut::<Monster>(walker).unwrap().conditions = Default::default();
        world.run_system_once(use_reached_links).unwrap();
        world.flush();
        assert!(world.get::<Crossing>(walker).is_some());
    }

    #[test]
    fn a_crossing_ends_with_its_link() {
        let mut world = World::new();
        world.add_observer(end_crossing);
        let link = world.spawn_empty().id();
        let walker = world.spawn((UsingAnimationLink, Crossing { link, to: Vec3::X })).id();
        world.flush();
        world.despawn(link);
        world.flush();
        assert!(world.get::<Crossing>(walker).is_none());
        assert!(world.get::<UsingAnimationLink>(walker).is_none());
    }

    /// An accepted entry holds navigation too: a walker at the far side of
    /// its crossing does not finish it until the entry is gone.
    #[test]
    fn crossings_wait_out_an_entry() {
        use crate::game::world::WorldPlugin;
        use crate::game::world::level::{Destination, Entry, Level};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), WorldPlugin))
            .init_resource::<Time<avian3d::prelude::Physics>>();
        app.finish();
        app.cleanup();
        app.world_mut().spawn(Level::test(default()));
        let link = app.world_mut().spawn_empty().id();
        let walker = app
            .world_mut()
            .spawn((
                super::super::Walker { speed: 2.0, float_height: 0.0 },
                Transform::default(),
                Crossing { link, to: Vec3::ZERO },
            ))
            .id();
        let entry = app
            .world_mut()
            .spawn(Destination::new(Level::test(default()), default(), Entry::Travel))
            .id();
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get::<Crossing>(walker).is_some(), "held while entering");
        app.world_mut().despawn(entry);
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get::<Crossing>(walker).is_none());
    }

    #[test]
    fn physical_door_pose_and_navigation_agree_before_each_planning_step() {
        use crate::game::world::{WorldPlugin, level::Level};
        assert!(DoorLink::around([]).is_none(), "no corners, no doorway");
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), WorldPlugin))
            .init_asset::<LevelDefinition>();
        app.finish();
        app.cleanup();
        let archipelago = app
            .world_mut()
            .spawn(Archipelago3d::new(ArchipelagoOptions::from_agent_radius(SIZE_CLASSES[0])))
            .id();
        // A door whose one part runs ten frames.
        let definition =
            app.world_mut().resource_mut::<Assets<LevelDefinition>>().add(LevelDefinition {
                props: vec![crate::content::level::PropModel {
                    name: "DOOR".into(),
                    collidable: true,
                    face_camera: false,
                    frames_per_second: 10.0,
                    parts: vec![crate::content::level::PropModelPart {
                        transform: Transform::IDENTITY,
                        keys: vec![Transform::IDENTITY; 11],
                        collision: default(),
                    }],
                    wraps: false,
                    slots: vec![],
                    trigger: None,
                }],
                ..default()
            });
        let level = app
            .world_mut()
            .spawn((Level::test(definition), LevelNavigation { archipelagos: vec![archipelago] }))
            .id();
        let door = app
            .world_mut()
            .spawn((
                Prop { definition: 0 },
                PropMotion { frame: 0.0, target: 10.0 },
                DoorLink::around([Vec3::new(-1.0, 0.0, -0.1), Vec3::new(1.0, 2.0, 0.1)]).unwrap(),
                LevelOwned(level),
            ))
            .id();
        // An authored part is a sibling of the navigation links and must survive closing.
        let part = app.world_mut().spawn(ChildOf(door)).id();
        app.world_mut().resource_mut::<Time>().advance_by(std::time::Duration::from_millis(500));
        app.world_mut().run_schedule(FixedPreUpdate);
        let links: Vec<_> = app
            .world_mut()
            .query_filtered::<Entity, With<AnimationLink3d>>()
            .iter(app.world())
            .collect();
        assert_eq!(links.len(), 1, "the halfway pose opens navigation in this same step");
        app.world_mut().get_mut::<PropMotion>(door).unwrap().target = 0.0;
        app.world_mut().run_schedule(FixedPreUpdate);
        assert!(app.world().get_entity(links[0]).is_err());
        assert!(app.world().get_entity(part).is_ok());
        app.world_mut().get_mut::<PropMotion>(door).unwrap().target = 10.0;
        app.world_mut().run_schedule(FixedPreUpdate);
        let link = app
            .world_mut()
            .query_filtered::<Entity, With<AnimationLink3d>>()
            .single(app.world())
            .unwrap();
        app.world_mut().despawn(door);
        assert!(app.world().get_entity(link).is_err());
    }
}
