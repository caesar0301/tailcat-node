//! Output formatting helpers.

use serde_json::Value;

/// Print a JSON value pretty-printed.
pub fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

/// Print a key-value pair.
pub fn print_kv(key: &str, value: &str) {
    println!("  {:<12} {}", format!("{}:", key), value);
}
