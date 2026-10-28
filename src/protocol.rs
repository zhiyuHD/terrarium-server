use anyhow::{Result, bail, ensure};
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{self, BufWriter, Write},
    sync::LazyLock,
};
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
pub fn valid_client_system(value: &Value) -> bool {
    value.as_object().is_some_and(|system| {
        system.keys().all(|key| key == "name" || key == "version")
            && ["name", "version"].iter().all(|key| {
                system
                    .get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(|part| {
                        !part.is_empty()
                            && part.chars().count() <= 200
                            && part.trim() == part
                            && !part.chars().any(|c| c <= '\u{001f}' || c == '\u{007f}')
                    })
            })
    })
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
fn write_canonical(value: &Value, out: &mut impl Write) -> io::Result<()> {
    match value {
        Value::Null => out.write_all(b"null"),
        Value::Bool(value) => out.write_all(if *value { b"true" } else { b"false" }),
        Value::Number(number) => {
            let n = number.as_f64().unwrap();
            if n == 0.0 {
                out.write_all(b"0")
            } else {
                out.write_all(ryu_js::Buffer::new().format(n).as_bytes())
            }
        }
        Value::String(_) => serde_json::to_writer(out, value).map_err(io::Error::other),
        Value::Array(items) => {
            out.write_all(b"[")?;
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    out.write_all(b",")?;
                }
                write_canonical(item, out)?;
            }
            out.write_all(b"]")
        }
        Value::Object(object) => {
            out.write_all(b"{")?;
            let index = |key: &str| {
                key.parse::<u32>()
                    .ok()
                    .filter(|n| *n < u32::MAX && n.to_string() == key)
            };
            let mut keys = Vec::new();
            if object
                .keys()
                .any(|key| !key.is_ascii() || index(key).is_some())
            {
                keys.extend(object.keys());
                keys.sort_by(|a, b| match (index(a), index(b)) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    _ => a.encode_utf16().cmp(b.encode_utf16()),
                });
            }
            let mut field = |position: usize, key: &String| -> io::Result<()> {
                if position != 0 {
                    out.write_all(b",")?;
                }
                serde_json::to_writer(&mut *out, key).map_err(io::Error::other)?;
                out.write_all(b":")?;
                write_canonical(&object[key], out)
            };
            if keys.is_empty() {
                for (position, key) in object.keys().enumerate() {
                    field(position, key)?;
                }
            } else {
                for (position, key) in keys.into_iter().enumerate() {
                    field(position, key)?;
                }
            }
            out.write_all(b"}")
        }
    }
}
pub fn canonical_json(value: &Value) -> String {
    let mut bytes = Vec::new();
    write_canonical(value, &mut bytes).unwrap();
    String::from_utf8(bytes).unwrap()
}
struct HashWriter(Sha256);
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub fn state_hash(value: &Value) -> String {
    let mut out = BufWriter::with_capacity(8192, HashWriter(Sha256::new()));
    write_canonical(value, &mut out).unwrap();
    out.flush().unwrap();
    format!(
        "{:x}",
        out.into_inner()
            .unwrap_or_else(|_| unreachable!())
            .0
            .finalize()
    )
}
struct SizeWriter(usize);
impl Write for SizeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        if self.0 > MAX_BYTES {
            return Err(io::Error::other("Report too large"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
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
    write_canonical(&state, &mut SizeWriter(0))?;
    Ok(state)
}
fn array_index(key: &str) -> Option<usize> {
    key.parse::<usize>()
        .ok()
        .filter(|i| i.to_string() == key && *i <= 9_007_199_254_740_991)
}
