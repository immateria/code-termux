use lazy_static::lazy_static;
use serde::Deserialize;
use serde_json::Value;
// Keep JSON insertion order; no need for BTreeMap
use std::sync::LockResult;
use std::sync::RwLock;
use std::sync::RwLockReadGuard;
use std::sync::RwLockWriteGuard;

#[derive(Debug, Clone)]
pub struct Spinner {
    /// Machine name (from JSON key)
    pub name: String,
    /// Human‑readable label (Title Case)
    pub label: String,
    /// Logical group for browsing
    pub group: String,
    pub interval_ms: u64,
    pub frames: Vec<String>,
}

#[derive(Deserialize)]
struct SpinnerJson {
    interval: u64,
    frames: Vec<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    group: Option<String>,
}

// The full upstream set (commit pinned) with our classic "diamond" added.
// Stored as JSON text and parsed on startup; supports two formats:
// 1) Flat map: { name: {interval, frames, label?, group?}, ... }
// 2) Grouped map: { Group: { name: {interval, frames, label?}, ... }, ... }
const SPINNERS_JSON: &str = include_str!("../assets/spinners.json");

lazy_static! {
    static ref FALLBACK_SPINNER: Spinner = Spinner {
        name: "fallback".to_string(),
        label: "Fallback".to_string(),
        group: "Other".to_string(),
        interval_ms: 120,
        frames: vec!["-".into(), "\\".into(), "|".into(), "/".into()],
    };
    static ref ALL_SPINNERS: Vec<Spinner> = {
        let mut list: Vec<Spinner> = Vec::new();
        let val: Value = serde_json::from_str(SPINNERS_JSON).unwrap_or(Value::Object(Default::default()));
        match val {
            Value::Object(map) => {
                // Mixed-mode tolerant parse: for each top-level entry
                for (k, v) in map.into_iter() {
                    // If this is the pointer entry (Default: "name"), skip it;
                    // but allow a group actually named "Default" (object).
                    if k == "Default" {
                        if !v.is_string() { /* fall through to parse group */ } else { continue; }
                    }
                    if let Value::Object(inner) = v {
                        if inner.get("interval").is_some() {
                            // Flat entry
                            if let Ok(sj) = serde_json::from_value::<SpinnerJson>(Value::Object(inner)) {
                                vpush(&mut list, &k, sj, None);
                            }
                        } else {
                            // Group container
                            for (name, val_entry) in inner.into_iter() {
                                if let Ok(sj) = serde_json::from_value::<SpinnerJson>(val_entry) {
                                    vpush(&mut list, &name, sj, Some(k.clone()));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        // Preserve JSON order: no reordering here
        list
    };
    static ref DEFAULT_INDEX: usize = {
        let val: Value = serde_json::from_str(SPINNERS_JSON).unwrap_or(Value::Object(Default::default()));
        let mut idx = 0usize;
        if let Value::Object(map) = val
            && let Some(Value::String(def)) = map.get("Default")
                && let Some(found) = ALL_SPINNERS.iter().position(|s| s.name == *def) {
                    idx = found;
                }
        idx
    };
    static ref CURRENT_INDEX: RwLock<usize> = RwLock::new(*DEFAULT_INDEX);
    static ref CURRENT_NAME: RwLock<String> = RwLock::new(ALL_SPINNERS[*DEFAULT_INDEX].name.clone());
    static ref CUSTOM_SPINNERS: RwLock<Vec<Spinner>> = RwLock::new(Vec::new());
}

fn unwrap_lock<T>(result: LockResult<T>) -> T {
    result.unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    unwrap_lock(lock.read())
}

fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    unwrap_lock(lock.write())
}

pub fn init_spinner(name: &str) { switch_spinner(name); }

pub fn switch_spinner(name: &str) {
    if ALL_SPINNERS.is_empty() { return; }
    let raw = name.trim();
    // Update the canonical current name (custom or built‑in)
    *write_lock(&CURRENT_NAME) = raw.to_string();
    // Keep CURRENT_INDEX aligned when the name is an all‑spinners entry (for fallbacks)
    let mut idx = ALL_SPINNERS.iter().position(|s| s.name == raw);
    if idx.is_none() {
        let needle = raw.to_ascii_lowercase();
        idx = ALL_SPINNERS.iter().position(|s| s.name.to_ascii_lowercase() == needle);
    }
    let idx = idx.unwrap_or(*DEFAULT_INDEX);
    *write_lock(&CURRENT_INDEX) = idx;
}

pub fn current_spinner() -> &'static Spinner {
    if ALL_SPINNERS.is_empty() { return &FALLBACK_SPINNER; }
    // Resolve by current name first (supports custom), then fall back to ALL_SPINNERS by index
    let name = read_lock(&CURRENT_NAME).clone();
    if let Some(s) = find_spinner_by_name(&name) { return s; }
    let idx = *read_lock(&CURRENT_INDEX);
    let idx = idx.min(ALL_SPINNERS.len().saturating_sub(1));
    &ALL_SPINNERS[idx]
}

pub fn find_spinner_by_name(name: &str) -> Option<&'static Spinner> {
    let raw = name.trim();
    // custom first
    if let Some(pos) = read_lock(&CUSTOM_SPINNERS).iter().position(|s| s.name == raw) {
        // Leak to 'static for shared ref (only for custom preview; safe for session lifetime)
        let s = read_lock(&CUSTOM_SPINNERS)[pos].clone();
        let b = Box::leak(Box::new(s));
        return Some(b);
    }
    ALL_SPINNERS
        .iter()
        .find(|s| s.name == raw)
        .or_else(|| {
            let needle = raw.to_ascii_lowercase();
            ALL_SPINNERS.iter().find(|s| s.name.to_ascii_lowercase() == needle)
        })
}

#[allow(dead_code)]
pub fn spinner_names() -> Vec<String> {
    let mut v: Vec<String> = ALL_SPINNERS.iter().map(|s| s.name.clone()).collect();
    v.extend(read_lock(&CUSTOM_SPINNERS).iter().map(|s| s.name.clone()));
    v
}

pub fn spinner_label_for(name: &str) -> String {
    find_spinner_by_name(name)
        .map(|s| s.label.clone())
        .unwrap_or_else(|| humanize(name))
}

#[allow(dead_code)]
pub fn spinner_group_for(name: &str) -> &'static str {
    if let Some(s) = find_spinner_by_name(name) { return &s.group; }
    derive_group(name)
}

pub fn frame_at_time(def: &Spinner, now_ms: u128) -> String {
    if def.frames.is_empty() { return String::new(); }
    let idx = ((now_ms as u64 / def.interval_ms) as usize) % def.frames.len();
    def.frames[idx].clone()
}

fn humanize(name: &str) -> String {
    // Convert kebab or camelCase to Title Case with spaces, keep digits grouped
    let mut out = String::new();
    let mut prev_is_lower = false;
    let mut prev_is_alpha = false;
    for ch in name.chars() {
        if ch == '-' || ch == '_' {
            out.push(' ');
            prev_is_lower = false;
            prev_is_alpha = false;
            continue;
        }
        if ch.is_ascii_uppercase() && prev_is_lower {
            out.push(' ');
        } else if ch.is_ascii_digit() && prev_is_alpha {
            out.push(' ');
        }
        out.push(ch);
        prev_is_lower = ch.is_ascii_lowercase();
        prev_is_alpha = ch.is_ascii_alphabetic();
    }
    // Title case each word
    out.split_whitespace()
        .map(|w| {
            let mut cs = w.chars();
            match cs.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), cs.as_str().to_lowercase()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn derive_group(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    let key = n.as_str();
    if key.contains("dot") { return "Dots"; }
    if key.contains("circle") || key.contains("round") || key.contains("arc") { return "Circles"; }
    if key.contains("line") || key.contains("pipe") || key.contains("bar") || key.contains("pulse") { return "Lines"; }
    if key.contains("bounce") || key.contains("ball") || key.contains("pong") { return "Bouncing"; }
    if key.contains("star") || key.contains("asterisk") { return "Stars"; }
    if key.contains("arrow") || key.contains("triangle") { return "Arrows"; }
    if key.contains("box") || key.contains("square") { return "Boxes"; }
    if key.contains("toggle") { return "Toggles"; }
    if key.contains("monkey") || key.contains("earth") || key.contains("moon") || key.contains("weather") || key.contains("smiley") || key.contains("emoji") { return "Emoji"; }
    "Other"
}

fn vpush(out: &mut Vec<Spinner>, name: &str, sj: SpinnerJson, group_override: Option<String>) {
    let label = sj.label.clone().unwrap_or_else(|| humanize(name));
    let group = group_override.unwrap_or_else(|| sj.group.clone().unwrap_or_else(|| derive_group(name).to_string()));
    out.push(Spinner { name: name.to_string(), label, group, interval_ms: sj.interval, frames: sj.frames });
}

#[allow(dead_code)]
pub fn global_max_frame_len() -> usize {
    let mut maxlen = 0usize;
    for s in ALL_SPINNERS.iter() { for f in &s.frames { maxlen = maxlen.max(f.chars().count()); } }
    for s in read_lock(&CUSTOM_SPINNERS).iter() { for f in &s.frames { maxlen = maxlen.max(f.chars().count()); } }
    maxlen
}

pub fn set_custom_spinners(custom: Vec<Spinner>) { *write_lock(&CUSTOM_SPINNERS) = custom; }

#[allow(dead_code)]
pub fn add_custom_spinner(name: String, label: String, interval_ms: u64, frames: Vec<String>) {
    let mut v = write_lock(&CUSTOM_SPINNERS);
    if let Some(pos) = v.iter().position(|s| s.name == name) {
        v[pos] = Spinner { name, label, group: "Custom".to_string(), interval_ms, frames };
    } else {
        v.push(Spinner { name, label, group: "Custom".to_string(), interval_ms, frames });
    }
}
