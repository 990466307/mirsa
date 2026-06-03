use std::collections::HashMap;
use std::fs;
use std::path::Path;

const CONFIG_PATH: &str = "crates/analysis/src/config.toml";

#[derive(Clone, Copy, Debug)]
pub struct EngineConfig {
    pub max_paths: usize,
    pub max_iterations: Option<u32>,
}

fn load_key_value_config(path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(content) = fs::read_to_string(path) else {
        return map;
    };

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        map.insert(key.trim().to_string(), value.trim().to_string());
    }
    map
}

fn parse_u32_config_value(key: &str, value: &str) -> Option<u32> {
    match value.trim().parse::<u32>() {
        Ok(v) => Some(v),
        Err(_) => {
            println!(
                "Warning: invalid config value for `{}`: `{}`; using default.",
                key,
                value.trim()
            );
            None
        }
    }
}

fn parse_optional_u32_config_value(key: &str, value: &str) -> Option<Option<u32>> {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    parse_u32_config_value(key, trimmed).map(Some)
}

fn parse_bool_config_value(key: &str, value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => {
            println!(
                "Warning: invalid config value for `{}`: `{}`; using default.",
                key,
                value.trim()
            );
            None
        }
    }
}

pub fn load_bool_config(path: &Path, key: &str, default: bool) -> bool {
    let map = load_key_value_config(path);
    map.get(key)
        .and_then(|value| parse_bool_config_value(key, value))
        .unwrap_or(default)
}

pub fn load_analysis_bool_config(key: &str, default: bool) -> bool {
    let map = load_key_value_config(Path::new(CONFIG_PATH));
    map.get(key)
        .and_then(|value| parse_bool_config_value(key, value))
        .unwrap_or(default)
}

pub fn load_engine_config(path: &Path) -> EngineConfig {
    let mut cfg = EngineConfig {
        max_paths: 10,
        max_iterations: None,
    };

    let map = load_key_value_config(path);
    if let Some(v) = map
        .get("max_paths")
        .and_then(|x| parse_u32_config_value("max_paths", x))
    {
        cfg.max_paths = usize::try_from(v).unwrap_or(cfg.max_paths);
    }
    if let Some(v) = map
        .get("max_iterations")
        .and_then(|x| parse_optional_u32_config_value("max_iterations", x))
    {
        cfg.max_iterations = v;
    }

    cfg
}

pub fn load_domain_engine_config(domain: &str) -> EngineConfig {
    let mut cfg = EngineConfig {
        max_paths: 10,
        max_iterations: None,
    };

    let map = load_key_value_config(Path::new(CONFIG_PATH));
    let max_paths_key = format!("{domain}.max_paths");
    if let Some(v) = map
        .get(&max_paths_key)
        .and_then(|x| parse_u32_config_value(&max_paths_key, x))
    {
        cfg.max_paths = usize::try_from(v).unwrap_or(cfg.max_paths);
    }

    let max_iterations_key = format!("{domain}.max_iterations");
    if let Some(v) = map
        .get(&max_iterations_key)
        .and_then(|x| parse_optional_u32_config_value(&max_iterations_key, x))
    {
        cfg.max_iterations = v;
    }

    cfg
}
