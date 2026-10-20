#[test]
fn empty_population_scores_serialize_as_positive_zero() {
    let state = serde_json::json!({"plants":[],"bugs":[]});
    let parts = terrarium_server::scoring::score_parts(&state).unwrap();
    assert!(!parts["plants"].as_f64().unwrap().is_sign_negative());
    assert!(!parts["bugs"].as_f64().unwrap().is_sign_negative());
    assert!(!parts.to_string().contains("-0"));
}
