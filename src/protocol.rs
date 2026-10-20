use anyhow::{Result, bail, ensure};
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;
pub const MAX_BYTES: usize = 8_388_608;
pub const MAX_CHANGES: usize = 16_384;
pub fn valid_version(value: &Value) -> bool {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$").unwrap()
    });
    value
        .as_str()
        .is_some_and(|s| s.len() <= 100 && PATTERN.is_match(s))
}
pub fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    (2..=40).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}
pub fn valid_hash(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
/// Matches JSON.stringify with the client's sorted-key replacer, including JS integer-key ordering.
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Number(n) => {
            let v = n.as_f64().unwrap();
            if v == 0.0 {
                "0".into()
            } else {
                ryu_js::Buffer::new().format(v).to_owned()
            }
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical_json).collect::<Vec<_>>().join(",")
        ),
        Value::Object(obj) => {
            let index = |key: &str| {
                key.parse::<u32>()
                    .ok()
                    .filter(|n| *n < u32::MAX && n.to_string() == key)
            };
            let mut keys: Vec<_> = obj.keys().collect();
            keys.sort_by(|a, b| match (index(a), index(b)) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                _ => a.encode_utf16().cmp(b.encode_utf16()),
            });
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|k| format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap(),
                        canonical_json(&obj[k])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        _ => value.to_string(),
    }
}
pub fn state_hash(value: &Value) -> String {
    format!("{:x}", Sha256::digest(canonical_json(value).as_bytes()))
}
pub fn apply_changes(base: &Value, changes: &Value) -> Result<Value> {
    let changes = changes
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid report changes"))?;
    ensure!(changes.len() <= MAX_CHANGES, "Invalid report changes");
    let mut state = base.clone();
    for change in changes {
        let op = change["op"].as_str().unwrap_or("");
        let path = change["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid report path"))?;
        ensure!(
            matches!(op, "add" | "replace" | "remove")
                && (op == "remove" || change.get("value").is_some()),
            "Invalid report change"
        );
        if path.is_empty() {
            ensure!(op == "replace", "Invalid root change");
            state = change["value"].clone();
            continue;
        }
        ensure!(path.starts_with('/'), "Invalid report path");
        let keys = path[1..]
            .split('/')
            .map(|part| -> Result<String> {
                let mut chars = part.chars();
                let mut key = String::new();
                while let Some(c) = chars.next() {
                    if c == '~' {
                        key.push(match chars.next() {
                            Some('0') => '~',
                            Some('1') => '/',
                            _ => bail!("Invalid report path"),
                        })
                    } else {
                        key.push(c)
                    }
                }
                ensure!(
                    !["__proto__", "prototype", "constructor"].contains(&key.as_str()),
                    "Invalid report path"
                );
                Ok(key)
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(keys.len() <= 32, "Report path too deep");
        let mut target = &mut state;
        for key in &keys[..keys.len() - 1] {
            target = match target {
                Value::Object(obj) => obj.get_mut(key),
                Value::Array(a) => array_index(key).and_then(|i| a.get_mut(i)),
                _ => None,
            }
            .ok_or_else(|| anyhow::anyhow!("Missing report path"))?;
        }
        let key = keys.last().unwrap();
        match target {
            Value::Array(a) => {
                let i = array_index(key).ok_or_else(|| anyhow::anyhow!("Invalid array index"))?;
                ensure!(
                    i <= a.len() && (op == "add" || i < a.len()),
                    "Missing report path"
                );
                match op {
                    "add" => a.insert(i, change["value"].clone()),
                    "remove" => {
                        a.remove(i);
                    }
                    _ => a[i] = change["value"].clone(),
                }
            }
            Value::Object(obj) => {
                ensure!(op == "add" || obj.contains_key(key), "Missing report path");
                if op == "remove" {
                    obj.remove(key);
                } else {
                    obj.insert(key.to_owned(), change["value"].clone());
                }
            }
            _ => bail!("Missing report path"),
        }
    }
    ensure!(
        canonical_json(&state).len() <= MAX_BYTES,
        "Report too large"
    );
    Ok(state)
}
fn array_index(key: &str) -> Option<usize> {
    key.parse::<usize>()
        .ok()
        .filter(|i| i.to_string() == key && *i <= 9_007_199_254_740_991)
}
