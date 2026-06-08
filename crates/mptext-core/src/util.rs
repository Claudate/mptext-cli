use std::path::Path;

use anyhow::{Context, Result};

pub fn safe_filename(title: &str, format: &str) -> String {
    let ext = match format {
        "html" => "html",
        "text" => "txt",
        "json" => "json",
        _ => "md",
    };
    let mut name: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    name = name.trim().to_string();
    if name.chars().count() > 80 {
        name = name.chars().take(80).collect();
    }
    if name.is_empty() {
        name = "untitled".to_string();
    }
    format!("{name}.{ext}")
}

pub fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content).with_context(|| format!("写入文件失败: {}", path.display()))
}
