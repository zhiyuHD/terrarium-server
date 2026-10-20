use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::HashMap;

fn number(value: &Value, key: &str) -> Result<f64> {
    value[key]
        .as_f64()
        .filter(|n| n.is_finite())
        .with_context(|| format!("Invalid numeric field: {key}"))
}
fn species_score(items: &Value, value: impl Fn(&Value) -> Result<f64>) -> Result<f64> {
    let mut species: HashMap<&str, Vec<f64>> = HashMap::new();
    for item in items.as_array().context("Missing organism array")? {
        species
            .entry(item["species"].as_str().context("Missing species")?)
            .or_default()
            .push(value(item)?);
    }
    let total: f64 = species
        .values_mut()
        .map(|values| {
            values.sort_by(|a, b| b.total_cmp(a));
            values
                .iter()
                .zip([1.0, 0.5, 0.25])
                .map(|(v, w)| v * w)
                .sum::<f64>()
        })
        .sum();
    ensure!(total.is_finite(), "Invalid score");
    Ok(total.round())
}
fn maturity(value: &Value) -> Result<f64> {
    Ok(0.25 + 0.75 * (number(value, "ageHours")? / 24.0).min(1.0))
}
pub fn score_parts(state: &Value) -> Result<Value> {
    let plants = species_score(&state["plants"], |p| {
        Ok((number(p, "stage")? + 1.0) * number(p, "health")? / 10.0)
    })?;
    if state.get("fish").is_some() {
        let oxygen = number(state, "oxygen")?;
        let cleanliness = number(state, "cleanliness")?;
        let fish = species_score(&state["fish"], |f| {
            Ok(
                40.0 * number(f, "health")? / 100.0 * number(f, "energy")? / 100.0
                    * maturity(f)?
                    * 1.0_f64.min(oxygen / 50.0).min(cleanliness / 50.0),
            )
        })?;
        Ok(
            json!({"plants":plants,"fish":fish,"water":(40.0*oxygen.min(cleanliness)/100.0*1.0_f64.min(plants/40.0).min(fish/40.0)).round()}),
        )
    } else {
        let bugs = species_score(&state["bugs"], |b| {
            Ok(40.0 * number(b, "energy")? / 100.0
                * (1.0
                    - b.get("hunger")
                        .map(|_| number(b, "hunger"))
                        .transpose()?
                        .unwrap_or(0.0)
                        / 100.0)
                * maturity(b)?)
        })?;
        Ok(json!({"plants":plants,"bugs":bugs,"seeds":0}))
    }
}
pub fn score(state: &Value) -> Result<f64> {
    Ok(score_parts(state)?
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_f64().unwrap())
        .sum())
}
