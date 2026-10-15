//! Token pace from `omp usage --json`, free memory and memory pressure, and the resulting capacity.
//!
//! For each provider window, pace = remaining fraction / remaining time fraction. Above 1 the quota
//! would go unused before the window resets, so parallelism widens; below 1 (or near empty) it narrows.

use std::collections::BTreeMap;
use std::process::{Command, Stdio};

use indexmap::IndexMap;
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::config::Config;
use crate::registry::LEVELS;

const LOW_REMAINING: f64 = 0.1;

#[derive(Clone, Debug, Serialize)]
pub struct Window {
    pub provider: String,
    pub window: String,
    pub remaining: f64,
    pub time_left: f64,
    pub pace: f64,
    pub resets_at: i64,
    pub status: Option<String>,
}

/// `omp usage --json`; an empty report when omp is missing or fails (capacity then ignores tokens).
pub fn usage_report() -> Value {
    let empty = json!({"reports": []});
    let Ok(out) = Command::new("omp").args(["usage", "--json"]).stdin(Stdio::null()).output() else { return empty };
    if !out.status.success() {
        return empty;
    }
    serde_json::from_slice(&out.stdout).unwrap_or(empty)
}

fn round_to(value: f64, digits: i32) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let scale = 10f64.powi(digits);
    (value * scale).round_ties_even() / scale
}

/// Remaining fraction, remaining time fraction and pace per provider window.
pub fn windows(report: &Value, now_ms: i64) -> Vec<Window> {
    let mut out = Vec::new();
    let Some(reports) = report.get("reports").and_then(Value::as_array) else { return out };
    for item in reports {
        let Some(limits) = item.get("limits").and_then(Value::as_array) else { continue };
        for limit in limits {
            let window = limit.get("window").filter(|w| w.is_object()).cloned().unwrap_or(Value::Null);
            let duration = window.get("durationMs").and_then(Value::as_f64).filter(|d| *d != 0.0);
            let resets = window.get("resetsAt").and_then(Value::as_f64).filter(|r| *r != 0.0);
            let remaining = limit.get("amount").and_then(|a| a.get("remainingFraction")).and_then(Value::as_f64);
            let (Some(duration), Some(resets), Some(remaining)) = (duration, resets, remaining) else { continue };
            let time_left = ((resets - now_ms as f64) / duration).clamp(0.0, 1.0);
            let pace = if time_left > 0.0 { remaining / time_left } else { f64::INFINITY };
            let label = limit.get("label").and_then(Value::as_str).filter(|l| !l.is_empty());
            let name = match label {
                Some(l) => l.to_string(),
                None => match window.get("id") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(other) => other.to_string(),
                },
            };
            out.push(Window {
                provider: item.get("provider").and_then(Value::as_str).unwrap_or_default().to_string(),
                window: name,
                remaining: round_to(remaining, 3),
                time_left: round_to(time_left, 3),
                pace: round_to(pace, 2),
                resets_at: resets as i64,
                status: limit.get("status").and_then(Value::as_str).map(str::to_string),
            });
        }
    }
    out
}

/// Recommended workers per provider, judged by its tightest window (lowest pace).
pub fn recommend(windows: &[Window], max_workers: usize) -> IndexMap<String, Value> {
    let mut by_provider: IndexMap<&str, Vec<&Window>> = IndexMap::new();
    for w in windows {
        by_provider.entry(w.provider.as_str()).or_default().push(w);
    }
    let mut result = IndexMap::new();
    for (provider, items) in by_provider {
        let tight = items.iter().copied().fold(items[0], |best, w| if w.pace < best.pace { w } else { best });
        let status_ok = tight.status.as_deref().is_none_or(|s| s == "ok");
        let (workers, mode) = if tight.remaining <= LOW_REMAINING || !status_ok {
            (1, "throttle")
        } else if tight.pace >= 1.5 {
            (max_workers, "maxing")
        } else if tight.pace >= 1.0 {
            (2.max(max_workers / 2), "steady")
        } else {
            (1.max((max_workers as f64 * tight.pace / 2.0).round_ties_even() as usize), "conserve")
        };
        result.insert(provider.to_string(), json!({"workers": workers, "mode": mode, "tightest": tight}));
    }
    result
}

/// Reusable memory from `vm_stat`: free + inactive + speculative pages, in GB (one decimal).
pub fn free_ram_gb() -> f64 {
    let Ok(out) = Command::new("vm_stat").output() else { return 0.0 };
    parse_vm_stat(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_vm_stat(out: &str) -> f64 {
    let number =
        |pattern: &str| Regex::new(pattern).ok().and_then(|re| re.captures(out)).and_then(|c| c[1].parse::<u64>().ok());
    let Some(page) = number(r"page size of (\d+) bytes") else { return 0.0 };
    let pages: u64 = ["Pages free", "Pages inactive", "Pages speculative"]
        .iter()
        .map(|key| number(&format!(r"{key}:\s+(\d+)")).unwrap_or(0))
        .sum();
    round_to((pages * page) as f64 / 1024f64.powi(3), 1)
}

/// macOS kernel memory pressure (`normal`, `warning`, `critical`), or `unknown`.
pub fn memory_pressure() -> String {
    let level = Command::new("sysctl")
        .args(["-n", "kern.memorystatus_vm_pressure_level"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<i64>().ok());
    match level {
        Some(1) => "normal",
        Some(2) => "warning",
        Some(4) => "critical",
        _ => "unknown",
    }
    .to_string()
}

/// Capacity with measured memory and the live usage report.
pub fn capacity(config: &Config, running: &BTreeMap<u8, usize>) -> Value {
    capacity_with(config, running, &usage_report(), free_ram_gb(), &memory_pressure())
}

/// Whole-number floats print as integers, as the Python config's ints did.
fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 1e15 { json!(value as i64) } else { json!(value) }
}

/// Levels 1-5 in order, then any others (level 0 counts entries without a level).
fn level_map<T: Copy + Into<Value>>(map: &BTreeMap<u8, T>) -> Map<String, Value> {
    let mut keys: Vec<u8> = LEVELS.iter().copied().filter(|l| map.contains_key(l)).collect();
    keys.extend(map.keys().copied().filter(|k| !LEVELS.contains(k)));
    keys.into_iter().map(|k| (k.to_string(), map[&k].into())).collect()
}

/// Capacity from explicit inputs (used by `capacity` and by tests).
/// New sessions overall (`slots`) and per level, plus recommended workers inside a session. Memory
/// budget = available minus the user's reserve, divided by each level's session size; memory
/// pressure at `warning` or above, or a throttled token window, blocks new sessions.
pub fn capacity_with(
    config: &Config,
    running: &BTreeMap<u8, usize>,
    report: &Value,
    available_gb: f64,
    pressure: &str,
) -> Value {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let rec = recommend(&windows(report, now_ms), config.max_workers);
    let budget_gb = round_to((available_gb - config.ram_reserve_gb).max(0.0), 1);
    let total: usize = running.values().sum();
    let by_count = config.max_running.saturating_sub(total);
    let primary = rec.get(&config.primary_provider);
    let basis: Vec<&Value> = match primary {
        Some(p) => vec![p],
        None => rec.values().collect(),
    };
    let token_workers =
        basis.iter().filter_map(|r| r["workers"].as_u64()).map(|w| w as usize).min().unwrap_or(config.max_workers);
    let throttled = basis.iter().any(|r| r["mode"] == "throttle");
    let blocked = throttled || pressure == "warning" || pressure == "critical";
    let fits = |cost: f64| if cost > 0.0 { (budget_gb / cost).floor() as usize } else { usize::MAX };
    let costs = &config.ram_gb_by_level;
    let caps = &config.max_running_by_level;
    let mut slots_by_level = Map::new();
    for level in LEVELS {
        let slots = if blocked {
            0
        } else {
            let cap = caps.get(&level).copied().unwrap_or(0).saturating_sub(running.get(&level).copied().unwrap_or(0));
            by_count.min(cap).min(fits(costs.get(&level).copied().unwrap_or(f64::INFINITY)))
        };
        slots_by_level.insert(level.to_string(), json!(slots));
    }
    let cheapest = costs.values().copied().fold(f64::INFINITY, f64::min);
    let slots = if blocked { 0 } else { by_count.min(fits(cheapest)) };
    let by_memory = 1.max(fits(config.ram_gb_per_worker));
    let basis_name = if rec.is_empty() {
        "unknown".to_string()
    } else if primary.is_some() {
        config.primary_provider.clone()
    } else {
        "all".to_string()
    };
    let costs_json: Map<String, Value> = costs.iter().map(|(k, v)| (k.to_string(), number(*v))).collect();
    json!({
        "slots": slots,
        "slots_by_level": slots_by_level,
        "recommended_workers": token_workers.min(by_memory),
        "running": total,
        "running_by_level": level_map(running),
        "ram": {"available_gb": available_gb, "reserve_gb": number(config.ram_reserve_gb), "budget_gb": budget_gb, "pressure": pressure},
        "limits": {
            "by_count": by_count,
            "throttled": throttled,
            "token_workers": token_workers,
            "max_running": config.max_running,
            "max_running_by_level": level_map(caps),
            "ram_gb_by_level": costs_json,
            "basis": basis_name,
        },
        "providers": rec,
    })
}
