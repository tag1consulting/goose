//! Groundwork for Gaggle, distributed load tests: a manager coordinating
//! workers that each run a share of the users.
//!
//! A worker's metrics processor keeps what changed since its last delta
//! beside its own state ([`worker`]) and sends it once a second as a
//! [`delta::GaggleDelta`], postcard encoded; the manager adds every worker's
//! deltas up ([`merge`]) into the metrics, graph data, dashboard snapshot
//! and reports one process running the whole load test would produce. No
//! network code: the transport comes later. `--gaggle-shadow-merge` runs
//! both sides in one process and checks the result ([`shadow`]).

pub(crate) mod compare;
pub(crate) mod delta;
pub(crate) mod merge;
pub(crate) mod shadow;
pub(crate) mod worker;

#[cfg(test)]
mod tests;

/// Whether two f32 averages agree to f32 rounding. A per second average
/// response time is an f32 moving average whose last digits depend on the
/// order its times arrive in, even in one process.
pub(crate) fn f32_close(a: f32, b: f32) -> bool {
    a == b || (a - b).abs() <= 1e-5 * a.abs().max(b.abs()).max(1e-3)
}

/// Every difference between two JSON values, as `path: a != b`, numbers
/// that are not integers matching to f32 rounding (see [`f32_close`]).
pub(crate) fn json_diff(
    path: &str,
    a: &serde_json::Value,
    b: &serde_json::Value,
    out: &mut Vec<String>,
) {
    use serde_json::Value;
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for key in keys {
                match (x.get(key), y.get(key)) {
                    (Some(x), Some(y)) => json_diff(&format!("{path}.{key}"), x, y, out),
                    (x, y) => out.push(format!("{path}.{key}: {x:?} != {y:?}")),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (index, (x, y)) in x.iter().zip(y).enumerate() {
                json_diff(&format!("{path}[{index}]"), x, y, out);
            }
        }
        (Value::Number(x), Value::Number(y)) if x.is_f64() || y.is_f64() => {
            let (x, y) = (x.as_f64().unwrap_or(0.0), y.as_f64().unwrap_or(0.0));
            if !f32_close(x as f32, y as f32) {
                out.push(format!("{path}: {x} != {y}"));
            }
        }
        _ if a == b => {}
        _ => {
            let shown = |value: &Value| {
                let text = value.to_string();
                match text.char_indices().nth(400) {
                    Some((end, _)) => format!("{}...", &text[..end]),
                    None => text,
                }
            };
            out.push(format!("{path}: {} != {}", shown(a), shown(b)));
        }
    }
}

/// Every difference between two texts, as their first differing lines,
/// numbers matching to f32 rounding: the text between numbers must be
/// equal, and each number close.
pub(crate) fn text_diff(name: &str, a: &str, b: &str, out: &mut Vec<String>) {
    fn tokens(text: &str) -> Vec<(bool, &str)> {
        let mut tokens = Vec::new();
        let bytes = text.as_bytes();
        let mut start = 0;
        let mut at = 0;
        let is_number = |c: u8| c.is_ascii_digit() || c == b'.';
        while at < bytes.len() {
            let number = is_number(bytes[at]);
            let mut end = at + 1;
            while end < bytes.len() && is_number(bytes[end]) == number {
                end += 1;
            }
            if start != at {
                tokens.push((false, &text[start..at]));
            }
            tokens.push((number, &text[at..end]));
            start = end;
            at = end;
        }
        tokens
    }
    let (x, y) = (tokens(a), tokens(b));
    let same = x.len() == y.len()
        && x.iter().zip(&y).all(|(x, y)| {
            x == y
                || (x.0
                    && y.0
                    && matches!(
                        (x.1.parse::<f64>(), y.1.parse::<f64>()),
                        (Ok(p), Ok(q)) if f32_close(p as f32, q as f32)
                    ))
        });
    if !same {
        let line = a
            .lines()
            .zip(b.lines())
            .position(|(p, q)| p != q)
            .unwrap_or(0);
        out.push(format!(
            "{name}: line {}: {:?} != {:?}",
            line + 1,
            a.lines().nth(line).unwrap_or(""),
            b.lines().nth(line).unwrap_or("")
        ));
    }
}
