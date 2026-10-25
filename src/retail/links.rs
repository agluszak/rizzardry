//! Navigation links and party teleports from a level's triggers.

use std::collections::HashMap;

use bevy::math::Vec3;

use super::sections::LevelSections;
use super::space::{UNITS_PER_METRE, retail_position, retail_yaw};
use super::trigger::{ItemTrigger, TriggerKind, TriggerPlacement, action};
use crate::content::level::{LevelLink, LinkKind, TeleportTrigger};

/// The eye above the feet, in retail units.
const EYE_HEIGHT: f32 = 1000.0;

fn feet(eye: [f32; 3]) -> Vec3 {
    retail_position([eye[0], eye[1] - EYE_HEIGHT, eye[2]])
}

fn recipients(trigger: &ItemTrigger) -> impl Iterator<Item = &str> {
    trigger.recipients.split(',').map(str::trim).filter(|r| !r.is_empty())
}

/// Same-level teleports (action 0x34/0x4b naming a trigger in this level) and
/// climbs (action 0x09 naming a camera path).
///
/// Destinations on other levels and named positions are not links. A
/// random teleport becomes one link per destination; the party takes the
/// first until the game has a random source for it.
pub fn level_links(sections: &LevelSections) -> (Vec<LevelLink>, Vec<TeleportTrigger>) {
    let positioned: HashMap<&str, (&ItemTrigger, TriggerPlacement)> = sections
        .all_triggers()
        .filter_map(|t| match &t.kind {
            TriggerKind::Item(item) => Some((t.name.as_str(), (item, item.placement?))),
            _ => None,
        })
        .collect();
    let mut links = Vec::new();
    let mut teleports = Vec::new();
    for trigger in sections.all_triggers() {
        let TriggerKind::Item(item) = &trigger.kind else { continue };
        match item.initial_action {
            action::TELEPORT | action::RANDOM_TELEPORT => {
                let Some(from) = item.placement else { continue };
                let destinations: Vec<TriggerPlacement> = recipients(item)
                    .filter_map(|name| positioned.get(name).map(|(_, placement)| *placement))
                    .collect();
                for destination in &destinations {
                    links.push(LevelLink {
                        kind: LinkKind::Teleport,
                        from: feet(from.position),
                        to: feet(destination.position),
                    });
                }
                if let Some(destination) = destinations.first() {
                    teleports.push(TeleportTrigger {
                        name: trigger.name.clone(),
                        centre: retail_position(from.position),
                        range: (item.range[0] / UNITS_PER_METRE, item.range[1] / UNITS_PER_METRE),
                        destination: retail_position(destination.position),
                        yaw: retail_yaw(destination.angle, destination.axis),
                    });
                }
            }
            action::CAMERA_PATH => {
                for name in recipients(item) {
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
    (links, teleports)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retail::bytes::Bytes;
    use crate::retail::pvl::read_level_geometry;
    use crate::retail::sections::read_sections;

    /// Runs against a real installation when `RIZZARDRY_RETAIL` names one.
    #[test]
    fn retail_levels_have_teleports_and_climbs() {
        let Ok(root) = std::env::var("RIZZARDRY_RETAIL") else { return };
        let files = crate::retail::files::RetailFiles::open(root).unwrap();
        let (mut teleports, mut climbs) = (0, 0);
        for level in files.names().filter(|n| n.ends_with(".pvl")) {
            let data = files.read(level).unwrap().unwrap();
            let start = read_level_geometry(&data).unwrap().end;
            let sections = read_sections(&mut Bytes::new(&data[start..])).unwrap();
            let (links, _) = level_links(&sections);
            teleports += links.iter().filter(|l| l.kind == LinkKind::Teleport).count();
            climbs += links.iter().filter(|l| l.kind == LinkKind::Climb).count();
        }
        // 43 triggers name a destination in their own level; random ones
        // name several.
        assert!(teleports >= 43, "{teleports} teleport links");
        assert!(climbs > 0, "{climbs} climb links");
    }
}
