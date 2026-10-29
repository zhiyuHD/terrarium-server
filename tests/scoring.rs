#[test]
fn empty_population_scores_serialize_as_positive_zero() {
    let state = serde_json::json!({"plants":[],"bugs":[]});
    let parts = terrarium_server::scoring::score_parts(&state).unwrap();
    assert!(!parts["plants"].as_f64().unwrap().is_sign_negative());
    assert!(!parts["bugs"].as_f64().unwrap().is_sign_negative());
    assert!(!parts.to_string().contains("-0"));
}

#[test]
fn curve_preserves_early_scores_has_diminishing_returns_and_no_fixed_ceiling() {
    use terrarium_server::scoring::curved_score;
    for base in 0..=200 {
        assert_eq!(curved_score(base as f64), base as f64);
    }
    for base in 0..10000 {
        assert!(curved_score((base + 1) as f64) >= curved_score(base as f64));
    }
    let values: Vec<_> = [200., 400., 600., 800., 1000., 1200.]
        .into_iter()
        .map(curved_score)
        .collect();
    let gains: Vec<_> = values.windows(2).map(|v| v[1] - v[0]).collect();
    assert!(gains.windows(2).all(|v| v[0] > v[1] && v[1] > 0.));
    assert!(curved_score(1e9) > curved_score(1e6));
}

#[test]
fn land_and_water_scores_match_typescript_fixture_totals_and_allocations() {
    use terrarium_server::scoring::{score, score_parts};
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/scores.json")).unwrap();
    let curved: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/score-curve.json")).unwrap();
    for (index, case) in cases.iter().chain(curved.iter()).enumerate() {
        assert_eq!(
            score(&case["state"]).unwrap(),
            case["score"].as_f64().unwrap(),
            "case {index}"
        );
        let parts = score_parts(&case["state"]).unwrap();
        for (key, expected) in case["parts"].as_object().unwrap() {
            assert_eq!(
                parts[key].as_f64(),
                expected.as_f64(),
                "case {index}, {key}"
            );
        }
    }
}
