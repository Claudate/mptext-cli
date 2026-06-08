use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    #[serde(default)]
    pub auth_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
}

pub fn config_path() -> Result<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("mptext")
    } else {
        dirs_fallback()?.join("mptext")
    };
    Ok(base.join("config.toml"))
}

fn dirs_fallback() -> Result<PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
        return Ok(PathBuf::from(home).join(".config"));
    }
    Ok(PathBuf::from("."))
}

pub fn load_config() -> Result<UserConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(UserConfig::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("读取配置失败: {}", path.display()))?;
    parse_config_toml(&text).with_context(|| format!("解析配置失败: {}", path.display()))
}

pub fn save_config(config: &UserConfig) -> Result<PathBuf> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serialize_config_toml(config)?;
    std::fs::write(&path, text)?;
    Ok(path)
}

fn parse_config_toml(text: &str) -> Result<UserConfig> {
    let mut cfg = UserConfig::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "auth_key" | "token" => cfg.auth_key = strip_quotes(v.trim()).to_string(),
                "base_url" => cfg.base_url = Some(strip_quotes(v.trim()).to_string()),
                _ => {}
            }
        }
    }
    Ok(cfg)
}

fn serialize_config_toml(config: &UserConfig) -> Result<String> {
    let mut lines = vec![
        "# mptext 用户配置".to_string(),
        format!("auth_key = \"{}\"", escape_toml(&config.auth_key)),
    ];
    if let Some(url) = &config.base_url {
        lines.push(format!("base_url = \"{}\"", escape_toml(url)));
    }
    lines.push(String::new());
    Ok(lines.join("\n"))
}

fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn strip_quotes(s: &str) -> &str {
    s.trim_matches('"').trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_auth_key_line() {
        let cfg = parse_config_toml("auth_key = \"abc-123\"\n").unwrap();
        assert_eq!(cfg.auth_key, "abc-123");
    }
}
