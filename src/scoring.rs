use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::HashMap;

pub const SCORE_VERSION: i64 = 3;

pub fn curved_score(base: f64) -> f64 {
    if base <= 200.0 {
        base
    } else {
        200.0 + 200.0 * (base / 200.0).ln()
    }
    .round()
}

pub(crate) fn curve_parts(parts: Value, keys: &[&str]) -> Value {
    let base: f64 = keys.iter().map(|key| parts[key].as_f64().unwrap()).sum();
    if base <= 200.0 {
        return parts;
    }
    let total = curved_score(base);
    let mut shares: Vec<_> = keys
        .iter()
        .enumerate()
        .map(|(index, key)| {
            let exact = parts[key].as_f64().unwrap() * total / base;
            (index, exact.fract(), exact.floor())
        })
        .collect();
    let remainder = (total - shares.iter().map(|s| s.2).sum::<f64>()) as usize;
    shares.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    for share in shares.iter_mut().take(remainder) {
        share.2 += 1.0;
    }
    let mut result = serde_json::Map::new();
    for (index, _, points) in shares {
        result.insert(keys[index].into(), json!(points));
    }
    Value::Object(result)
}

fn number(value: &Value, key: &str) -> Result<f64> {
    value[key]
        .as_f64()
        .filter(|n| n.is_finite())
        .with_context(|| format!("Invalid numeric field: {key}"))
}
fn species_score(items: &Value, value: impl Fn(&Value) -> Result<f64>) -> Result<f64> {
    let mut indices: HashMap<&str, usize> = HashMap::new();
    let mut species: Vec<Vec<f64>> = Vec::new();
    for item in items.as_array().context("Missing organism array")? {
        let name = item["species"].as_str().context("Missing species")?;
        let index = *indices.entry(name).or_insert_with(|| {
            species.push(Vec::new());
            species.len() - 1
        });
        species[index].push(value(item)?);
    }
    let total: f64 = species
        .iter_mut()
        .map(|values| {
            values.sort_by(|a, b| b.total_cmp(a));
            values
                .iter()
                .zip([1.0, 0.5, 0.25])
                .map(|(v, w)| v * w)
                .fold(0.0, |sum, value| sum + value)
        })
        .fold(0.0, |sum, value| sum + value);
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
        Ok(curve_parts(
            json!({"plants":plants,"fish":fish,"water":(40.0*oxygen.min(cleanliness)/100.0*1.0_f64.min(plants/40.0).min(fish/40.0)).round()}),
            &["plants", "fish", "water"],
        ))
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
        Ok(curve_parts(
            json!({"plants":plants,"bugs":bugs,"seeds":0}),
            &["plants", "bugs", "seeds"],
        ))
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
