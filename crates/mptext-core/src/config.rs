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

/// 配置目录（跨平台）：Win=%APPDATA%/mptext，Mac=~/.config/mptext
fn config_dir() -> Result<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("mptext")
    } else {
        dirs_fallback()?.join("mptext")
    };
    Ok(base)
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

/// 公众号记忆缓存文件路径（独立 JSON，不影响 token 配置）
pub fn accounts_cache_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("accounts.json"))
}

/// 读取已记忆的公众号列表（文件不存在或损坏时返回空列表，保证健壮）
pub fn load_accounts() -> Vec<crate::client::AccountItem> {
    let path = match accounts_cache_path() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    if !path.exists() {
        return Vec::new();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// 保存公众号记忆列表到本地
pub fn save_accounts(accounts: &[crate::client::AccountItem]) -> Result<PathBuf> {
    let path = accounts_cache_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(accounts)
        .context("序列化公众号缓存失败")?;
    std::fs::write(&path, text)
        .with_context(|| format!("写入公众号缓存失败: {}", path.display()))?;
    Ok(path)
}

/// 清空公众号记忆
pub fn clear_accounts() -> Result<()> {
    let path = accounts_cache_path()?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("删除公众号缓存失败: {}", path.display()))?;
    }
    Ok(())
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
