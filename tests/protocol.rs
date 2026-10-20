use serde_json::{Value, json};
use terrarium_server::{
    legacy_validation::LegacyValidator,
    policy::meets_minimum,
    protocol::{MAX_CHANGES, apply_changes, state_hash, valid_version},
};
#[test]
fn hashes_match_typescript_states_unicode_integer_keys_and_ecmascript_numbers() {
    let states: Vec<Value> = serde_json::from_str(include_str!("fixtures/scores.json")).unwrap();
    let extra: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/hash-edge-values.json")).unwrap();
    let expected: Vec<Value> = serde_json::from_str(include_str!("fixtures/hashes.json")).unwrap();
    for (i, value) in states
        .into_iter()
        .map(|v| v["state"].clone())
        .chain(extra)
        .enumerate()
    {
        assert_eq!(state_hash(&value), expected[i]["hash"], "case {i}");
    }
}
#[test]
fn patches_match_client_diff_and_reject_invalid_paths_without_mutating_baseline() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/patch.json")).unwrap();
    let base = &fixture["before"];
    assert_eq!(
        apply_changes(base, &fixture["changes"]).unwrap(),
        fixture["after"]
    );
    assert_eq!(
        apply_changes(base, &json!([{"op":"replace","path":"","value":null}])).unwrap(),
        Value::Null
    );
    for changes in [
        json!([{"op":"add","path":"/__proto__","value":{}}]),
        json!([{"op":"remove","path":"/missing"}]),
        json!([{"op":"remove","path":"/a~1b/~/0"}]),
        json!([{"op":"replace","path":"/a~1b/~0/01","value":0}]),
        json!([{"op":"add","path":"/a~1b/~0/4","value":0}]),
        json!([{"op":"add","path":"","value":0}]),
        json!([{"op":"replace","path":"/old"}]),
        json!([{"op":"replace","path":"/old/constructor","value":0}]),
        json!([{"op":"add","path":format!("/{}",vec!["a";33].join("/")),"value":0}]),
    ] {
        assert!(apply_changes(base, &changes).is_err(), "{changes}");
    }
    assert!(
        apply_changes(
            base,
            &json!(vec![
                json!({"op":"replace","path":"/old","value":true});
                MAX_CHANGES + 1
            ])
        )
        .is_err()
    );
    assert_eq!(base["old"], true);
    assert_eq!(base["a/b"]["~"], json!([1, 2, 3]));
}
#[test]
fn version_policy_compares_large_components_and_prereleases_without_integer_overflow() {
    for (client, minimum, ok) in [
        ("0.7.9", "0.7.9", true),
        ("0.7.9+build", "0.7.9", true),
        ("0.7.9-beta", "0.7.9", false),
        ("0.8.0-beta", "0.7.9", true),
        ("0.7.03", "0.7.4", false),
        ("99999999999999999999999.0.0", "9.0.0", true),
        ("0.7.3", "0.7.9", false),
    ] {
        assert_eq!(meets_minimum(&json!(client), &json!(minimum)), ok);
    }
    assert!(!valid_version(&json!("０.7.9")));
    assert!(!valid_version(&json!("0.7.9\n")));
    assert!(meets_minimum(&Value::Null, &Value::Null));
    assert!(!meets_minimum(&Value::Null, &json!("0.7.9")));
}
#[tokio::test]
async fn validation_fallback_preserves_unknown_fields_rejects_invalid_collections_and_recovers_after_invalid_input()
 {
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/reports.json")).unwrap();
    let validator = LegacyValidator::default();
    let mut land = fixtures["land"].clone();
    land["futureField"] = json!({"hello":"水"});
    assert_eq!(validator.validate(&land).await.unwrap(), land);
    assert_eq!(
        validator.validate(&fixtures["water"]).await.unwrap(),
        fixtures["water"]
    );
    let mut invalid = land.clone();
    invalid["collection"] = json!([{"kind":"fish","specimen":{}}]);
    assert!(validator.validate(&invalid).await.is_err());
    let mut invalid = fixtures["water"].clone();
    invalid["fish"] = json!([{"id":"invalid"}]);
    assert!(validator.validate(&invalid).await.is_err());
    assert_eq!(validator.validate(&land).await.unwrap(), land);
}

#[tokio::test]
async fn legacy_land_versions_and_historical_timestamps_migrate_before_hash_validation() {
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/reports.json")).unwrap();
    let land = fixtures["land"].clone();
    let validator = LegacyValidator::default();
    let old = json!({"version":1,"updatedAtMs":land["updatedAtMs"],"plants":land["plants"],"items":land["items"]});
    let migrated = validator.validate(&old).await.unwrap();
    assert_eq!(migrated["version"], 3);
    assert_eq!(migrated["soil"].as_array().unwrap().len(), 25);
    assert_eq!(migrated["plants"].as_array().unwrap().len(), 2);
    let mut old = land.clone();
    old["version"] = json!(2);
    old.as_object_mut().unwrap().remove("soil");
    old.as_object_mut().unwrap().remove("history");
    let migrated = validator.validate(&old).await.unwrap();
    assert_eq!(migrated["version"], 3);
    assert_eq!(migrated["soil"].as_array().unwrap().len(), 25);
    assert!(!migrated["history"].as_array().unwrap().is_empty());
    let mut old = land.clone();
    for sample in old["history"].as_array_mut().unwrap() {
        let time = sample
            .as_object_mut()
            .unwrap()
            .remove("recordedAtMs")
            .unwrap();
        sample["atMs"] = time;
    }
    let migrated = validator.validate(&old).await.unwrap();
    assert_eq!(migrated, land);
}
