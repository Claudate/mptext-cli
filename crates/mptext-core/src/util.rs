use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// 在目录下为 filename 生成不冲突的路径。
/// 若已存在同名文件，自动追加 `_1`、`_2`… 后缀，避免批量下载时同名文章互相覆盖。
pub fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }

    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let ext = path.extension().and_then(|s| s.to_str());

    let mut counter = 1usize;
    loop {
        let new_name = match ext {
            Some(e) => format!("{stem}_{counter}.{e}"),
            None => format!("{stem}_{counter}"),
        };
        let candidate = dir.join(&new_name);
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_path_dedupes_same_name() {
        let dir = std::env::temp_dir().join(format!("mptext_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let p1 = unique_path(&dir, "观音诞.md");
        assert_eq!(p1.file_name().unwrap(), "观音诞.md");
        std::fs::write(&p1, "a").unwrap();

        let p2 = unique_path(&dir, "观音诞.md");
        assert_eq!(p2.file_name().unwrap(), "观音诞_1.md");
        std::fs::write(&p2, "b").unwrap();

        let p3 = unique_path(&dir, "观音诞.md");
        assert_eq!(p3.file_name().unwrap(), "观音诞_2.md");

        std::fs::remove_dir_all(&dir).ok();
    }
}
