//! Navigation links and party teleports from a level's triggers.

use std::collections::HashMap;

use bevy::math::Vec3;

use super::sections::LevelSections;
use super::space::{retail_position, retail_yaw};
use super::trigger::{ItemTrigger, TriggerKind, TriggerPlacement, action};
use crate::content::level::{Arrival, Entrance, LevelLink, LinkKind};

/// The eye above the feet, in retail units.
const EYE_HEIGHT: f32 = crate::content::EYE_HEIGHT * crate::content::UNITS_PER_METRE;

fn feet(eye: [f32; 3]) -> Vec3 {
    retail_position([eye[0], eye[1] - EYE_HEIGHT, eye[2]])
}

/// Every positioned trigger as a place to enter the level.
pub fn level_entrances(sections: &LevelSections) -> Vec<Entrance> {
    sections
        .all_triggers()
        .filter_map(|t| match &t.kind {
            TriggerKind::Item(item) => item.placement.map(|placement| {
                let [x, y, z] = placement.position;
                Entrance {
                    name: t.name.clone(),
                    eye: retail_position([x, y + EYE_HEIGHT, z]),
                    yaw: retail_yaw(placement.angle, placement.axis),
                }
            }),
            _ => None,
        })
        .collect()
}

/// Where a teleport trigger (action 0x34) takes the party: a trigger
/// in this level, or another level's entrance named by location code and
/// number, e.g. `MO201` (Whizzardry `RunDestination`). Named positions are
/// not handled. Random teleports decode all destinations with `random_arrivals`.
pub fn teleport_arrival(sections: &LevelSections, item: &ItemTrigger) -> Option<Arrival> {
    let name = item.recipients().next()?;
    let here = item.recipients().find_map(|name| match &sections.trigger(name)?.kind {
        TriggerKind::Item(item) => item.placement,
        _ => None,
    });
    match here {
        Some(destination) => Some(Arrival::Here {
            eye: retail_position(destination.position),
            yaw: retail_yaw(destination.angle, destination.axis),
        }),
        None if name.len() >= 5 => Some(Arrival::Entrance {
            entrance: name.to_uppercase(),
            // Retail confirms the level change when the first datum is set.
            ask: item.data[0] > 0,
        }),
        None => None,
    }
}

/// Decode each destination without dropping invalid entries or making a
/// random selection during content loading.
pub fn random_arrivals(sections: &LevelSections, item: &ItemTrigger) -> Option<Vec<Arrival>> {
    let arrivals: Option<Vec<_>> = item
        .recipients()
        .map(|name| {
            if let Some(TriggerKind::Item(target)) = sections.trigger(name).map(|t| &t.kind) {
                let place = target.placement?;
                Some(Arrival::Here {
                    eye: retail_position(place.position),
                    yaw: retail_yaw(place.angle, place.axis),
                })
            } else {
                crate::content::levels::LevelInfo::for_entrance(name)?;
                Some(Arrival::Entrance { entrance: name.to_uppercase(), ask: item.data[0] > 0 })
            }
        })
        .collect();
    arrivals.filter(|arrivals| !arrivals.is_empty())
}

/// The links walkers use: teleports to a trigger in this level, and climbs
/// (action 0x09 naming a camera path). A random teleport becomes one link per
/// destination.
pub fn level_links(sections: &LevelSections) -> Vec<LevelLink> {
    let positioned: HashMap<&str, TriggerPlacement> = sections
        .all_triggers()
        .filter_map(|t| match &t.kind {
            TriggerKind::Item(item) => Some((t.name.as_str(), item.placement?)),
            _ => None,
        })
        .collect();
    let mut links = Vec::new();
    for trigger in sections.all_triggers() {
        let TriggerKind::Item(item) = &trigger.kind else { continue };
        match item.initial_action {
            action::TELEPORT | action::RANDOM_TELEPORT => {
                let Some(from) = item.placement else { continue };
                for destination in item.recipients().filter_map(|name| positioned.get(name)) {
                    links.push(LevelLink {
                        kind: LinkKind::Teleport,
                        from: feet(from.position),
                        to: feet(destination.position),
                    });
                }
            }
            action::CAMERA_PATH => {
                for name in item.recipients() {
                    let Some(camera) = sections.cameras.iter().find(|c| c.name == name) else {
                        continue;
                    };
                    let (Some(first), Some(last)) = (camera.path.first(), camera.path.last())
                    else {
                        continue;
                    };
                    links.push(LevelLink {
                        kind: LinkKind::Climb,
                        from: feet(first.position),
                        to: feet(last.position),
                    });
                }
            }
            _ => {}
        }
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::Bytes;
    use crate::retail::pvl::compiled_level_end;
    use crate::retail::sections::read_sections;

    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_levels_have_teleports_and_climbs() {
        let files = crate::retail::files::retail_files();
        let (mut teleports, mut climbs) = (0, 0);
        let mut entrances = std::collections::HashMap::new();
        let mut travel = Vec::new();
        for level in files.names().filter(|n| n.ends_with(".pvl")) {
            let data = files.read(level).unwrap().unwrap();
            let start = compiled_level_end(&data).unwrap();
            let sections = read_sections(&mut Bytes::new(&data[start..])).unwrap();
            let links = level_links(&sections);
            teleports += links.iter().filter(|l| l.kind == LinkKind::Teleport).count();
            climbs += links.iter().filter(|l| l.kind == LinkKind::Climb).count();
            let names: Vec<String> =
                level_entrances(&sections).into_iter().map(|e| e.name.to_uppercase()).collect();
            entrances.insert(level.to_owned(), names);
            for trigger in sections.all_triggers() {
                let TriggerKind::Item(item) = &trigger.kind else { continue };
                if item.enabled
                    && item.initial_action == action::TELEPORT
                    && let Some(Arrival::Entrance { entrance: name, .. }) =
                        teleport_arrival(&sections, item)
                {
                    travel.push((level.to_owned(), name));
                }
            }
        }
        // Every exit to another level names a level and one of its entrances.
        assert!(travel.len() > 20, "{} level exits", travel.len());
        for (from, entrance) in &travel {
            let to = crate::content::levels::LevelInfo::for_entrance(entrance)
                .unwrap_or_else(|| panic!("{from}: {entrance} names no level"))
                .path();
            assert!(entrances[&to].contains(entrance), "{from}: {entrance} is not in {to}");
        }
        // 43 triggers name a destination in their own level; random ones
        // name several.
        assert!(teleports >= 43, "{teleports} teleport links");
        assert!(climbs > 0, "{climbs} climb links");
    }
}
