//! Authored map cuts: numbered level clip planes, then MapFilters.txt entries.
//! Both references' AutomapScreen select the filter section by level-table
//! index, not by its bracketed label. Filter heights are authored metres.

use super::sections::ClipPlane;
use crate::content::UNITS_PER_METRE;

pub fn layer_heights(
    planes: &[ClipPlane],
    filters: &str,
    level_index: Option<usize>,
) -> Result<Vec<f32>, String> {
    let mut heights = Vec::new();
    for index in 1.. {
        let name = format!("LAYER {index}");
        let Some(plane) = planes.iter().find(|plane| plane.name == name) else { break };
        heights.push(plane.height / UNITS_PER_METRE);
    }
    if let Some(level_index) = level_index {
        let mut section = None;
        for line in filters.lines().map(str::trim) {
            if line.starts_with('[') {
                section = Some(section.map_or(0, |index| index + 1));
            } else if section == Some(level_index)
                && let Some(value) = line.strip_prefix("LAYER=")
            {
                heights.push(value.parse::<f32>().map_err(|_| format!("invalid height: {line}"))?);
            }
        }
    }
    if heights.iter().any(|height| !height.is_finite()) {
        return Err("layer heights must be finite".into());
    }
    Ok(heights)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_planes_precede_the_indexed_filter_section() {
        let planes = vec![
            ClipPlane { name: "LAYER 2".into(), height: 5000.0 },
            ClipPlane { name: "OTHER".into(), height: 9000.0 },
            ClipPlane { name: "LAYER 1".into(), height: 10000.0 },
            ClipPlane { name: "LAYER 4".into(), height: -9000.0 },
        ];
        let filters = "[first]\r\nLAYER=30\r\n[second]\r\ntexture.tga\r\nLAYER=3.5\r\nLAYER=-1\r\n[third]\r\nLAYER=-30";
        assert_eq!(layer_heights(&planes, filters, Some(1)).unwrap(), [20.0, 10.0, 3.5, -1.0]);
        assert_eq!(layer_heights(&planes, filters, None).unwrap(), [20.0, 10.0]);
        assert!(layer_heights(&[], "[first]\nLAYER=NaN", Some(0)).is_err());
    }

    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_layers_and_cursor_hotspots_match_authored_data() {
        use crate::bytes::Bytes;
        use crate::retail::{
            files::RetailFiles, pvl::read_level_geometry, sections::read_sections,
        };
        let files = RetailFiles::open(std::env::var("RIZZARDRY_RETAIL").expect("RIZZARDRY_RETAIL"))
            .unwrap();
        let filters = files.read("data/automap/mapfilters.txt").unwrap().unwrap();
        let filters = String::from_utf8_lossy(&filters);
        let cursor_data = files.read("data/automap/map_cursors.sti").unwrap().unwrap();
        let cursors = crate::retail::stci::read_stci(&cursor_data).unwrap();
        for (index, (x, y)) in [(0, 0), (8, 7), (1, 24), (1, 24), (8, 7)].into_iter().enumerate() {
            assert!(
                x < cursors[index].width && y < cursors[index].height,
                "cursor hotspot must fit frame {index}"
            );
        }
        let mut checked = 0;
        for (index, level) in crate::content::levels::LEVELS.iter().enumerate() {
            let Some(data) = files.read(&level.path()) else { continue };
            let data = data.unwrap();
            checked += 1;
            let geometry = read_level_geometry(&data).unwrap();
            let sections = read_sections(&mut Bytes::new(&data[geometry.end..])).unwrap();
            let layers = layer_heights(&sections.clip_planes, &filters, Some(index)).unwrap();
            assert!(
                layers.windows(2).all(|pair| pair[0] > pair[1]),
                "{}: {layers:?}",
                level.path()
            );
            if level.location == "MO1" {
                assert_eq!(layers, [41.0, 26.5, 15.5]);
            }
        }
        assert_eq!(checked, 34, "all shipped levels must be checked");
    }
}
