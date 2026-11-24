use ferriscraft_towerwars::domain::LaneId;
use ferriscraft_towerwars::map::{Map, MapError};
use serde_json::{Value, json};

fn valid_json() -> Value {
    serde_json::from_str(&Map::test_map().to_json()).expect("test map json is valid")
}

fn load(value: &Value) -> Result<Map, MapError> {
    Map::from_json(&value.to_string())
}

fn lane_reason(result: Result<Map, MapError>, expected_lane: u8) -> String {
    match result {
        Err(MapError::InvalidLane { lane, reason }) => {
            assert_eq!(lane, expected_lane);
            reason
        }
        other => panic!("expected InvalidLane, got {other:?}"),
    }
}

fn point(x: f64, z: f64) -> Value {
    json!({"x": x, "y": 64.0, "z": z})
}

#[test]
fn test_map_has_six_lanes_within_length_and_turn_bounds() {
    let map = Map::test_map();
    assert_eq!(map.lanes.len(), 6);
    for lane in LaneId::ALL {
        let path = map.path_of(lane).expect("path builds");
        assert!(
            (75.0..=95.0).contains(&path.length()),
            "lane {lane:?} length {}",
            path.length()
        );
        assert!(
            (4..=6).contains(&path.turns()),
            "lane {lane:?} turns {}",
            path.turns()
        );
        assert!(map.lane(lane).is_some());
    }
}

#[test]
fn test_map_is_deterministic() {
    assert_eq!(Map::test_map(), Map::test_map());
}

#[test]
fn test_map_round_trips_through_json() {
    let map = Map::test_map();
    let loaded = Map::from_json(&map.to_json()).expect("test map loads");
    assert_eq!(loaded, map);
}

#[test]
fn lane_areas_never_overlap_between_lanes() {
    let map = Map::test_map();
    for (index, first) in map.lanes.iter().enumerate() {
        for second in &map.lanes[index + 1..] {
            for a in &first.build_bounds {
                for b in &second.build_bounds {
                    let apart = a.max.x < b.min.x
                        || b.max.x < a.min.x
                        || a.max.z < b.min.z
                        || b.max.z < a.min.z;
                    assert!(
                        apart,
                        "lanes {} and {} overlap",
                        first.lane_id, second.lane_id
                    );
                }
            }
            for p in &first.waypoints {
                for q in &second.waypoints {
                    assert!(p.distance_xz(*q) > 1.0);
                }
            }
        }
    }
}

#[test]
fn malformed_json_is_a_parse_error() {
    assert!(matches!(
        Map::from_json("{ not json"),
        Err(MapError::Parse(_))
    ));
    assert!(matches!(
        Map::from_json(r#"{"lanes": 3}"#),
        Err(MapError::Parse(_))
    ));
}

#[test]
fn missing_lane_is_refused() {
    let mut value = valid_json();
    value["lanes"].as_array_mut().expect("lanes").pop();
    assert_eq!(load(&value), Err(MapError::LaneCount));
}

#[test]
fn duplicate_lane_id_is_refused() {
    let mut value = valid_json();
    value["lanes"][5]["lane_id"] = json!(4);
    assert_eq!(load(&value), Err(MapError::LaneCount));
}

#[test]
fn out_of_range_lane_id_is_refused() {
    let mut value = valid_json();
    value["lanes"][5]["lane_id"] = json!(6);
    assert_eq!(load(&value), Err(MapError::LaneCount));
}

#[test]
fn too_short_path_is_refused() {
    let mut value = valid_json();
    value["lanes"][2]["waypoints"] = json!([
        point(0.5, 0.5),
        point(10.5, 0.5),
        point(10.5, 4.5),
        point(0.5, 4.5),
        point(0.5, 8.5)
    ]);
    value["lanes"][2]["spawn_point"] = point(1.5, 0.5);
    let reason = lane_reason(load(&value), 2);
    assert!(reason.contains("longueur"), "{reason}");
}

#[test]
fn too_many_turns_is_refused() {
    let mut value = valid_json();
    let mut waypoints = Vec::new();
    for step in 1..=8 {
        let x = 0.5 + 10.0 * f64::from((step + 1) / 2);
        let z = 0.5 + 10.0 * f64::from(step / 2);
        waypoints.push(point(x, z));
    }
    value["lanes"][1]["spawn_point"] = point(0.5, 0.5);
    value["lanes"][1]["waypoints"] = Value::Array(waypoints);
    let reason = lane_reason(load(&value), 1);
    assert!(reason.contains("virages"), "{reason}");
}

#[test]
fn unbuildable_path_is_refused() {
    let mut value = valid_json();
    let spawn = value["lanes"][0]["spawn_point"].clone();
    value["lanes"][0]["waypoints"] = json!([spawn]);
    let reason = lane_reason(load(&value), 0);
    assert!(reason.contains("chemin"), "{reason}");
}

#[test]
fn empty_build_bounds_are_refused() {
    let mut value = valid_json();
    value["lanes"][3]["build_bounds"] = json!([]);
    let reason = lane_reason(load(&value), 3);
    assert!(reason.contains("build_bounds"), "{reason}");
}

#[test]
fn inverted_build_box_is_refused() {
    let mut value = valid_json();
    let bounds = value["lanes"][4]["build_bounds"][0].clone();
    value["lanes"][4]["build_bounds"][0] = json!({"min": bounds["max"], "max": bounds["min"]});
    let reason = lane_reason(load(&value), 4);
    assert!(reason.contains("min"), "{reason}");
}

#[test]
fn build_cell_on_the_path_is_refused() {
    let mut value = valid_json();
    let spawn = value["lanes"][5]["spawn_point"].clone();
    let x = spawn["x"].as_f64().expect("x") + 5.0;
    let z = spawn["z"].as_f64().expect("z");
    value["lanes"][5]["build_bounds"] = json!([{
        "min": {"x": x - 0.5, "y": 64.0, "z": z - 0.5},
        "max": {"x": x + 0.5, "y": 65.0, "z": z + 0.5}
    }]);
    let reason = lane_reason(load(&value), 5);
    assert!(reason.contains("chemin"), "{reason}");
}
