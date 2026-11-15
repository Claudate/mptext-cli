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

/// 同一篇文章在不同通道里的去重键。
///
/// 各通道给出的链接并不逐字相同：搜狗那条常带 `chksm` 与 `#rd`，`/mp/homepage`
/// 是 `scene=19#wechat_redirect`，合集又可能是另一种拼法。按完整 URL 去重会把
/// 同一篇算成多篇——2026-10-21 实测「手工川」55 条里有 11 条是这种重复。
///
/// 微信文章的稳定标识是 `__biz` + `mid` + `idx`（`sn` 只是签名，会随渠道变），
/// 三者齐备时用它们拼键；缺任何一个就退回去掉 `chksm`/`scene` 与 fragment 后的
/// 路径，仍比原串稳定。
pub fn article_key(url: &str) -> String {
    let (head, _fragment) = match url.find('#') {
        Some(pos) => (&url[..pos], &url[pos..]),
        None => (url, ""),
    };
    let (path, query) = match head.find('?') {
        Some(pos) => (&head[..pos], &head[pos + 1..]),
        None => (head, ""),
    };

    let mut biz = None;
    let mut mid = None;
    let mut idx = None;
    let mut kept: Vec<(&str, &str)> = Vec::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = match pair.find('=') {
            Some(pos) => (&pair[..pos], &pair[pos + 1..]),
            None => (pair, ""),
        };
        match k {
            "__biz" => biz = Some(v),
            "mid" => mid = Some(v),
            "idx" => idx = Some(v),
            // chksm 是正文防篡改签名，scene 是来源埋点，都不标识文章本身。
            "chksm" | "scene" => {}
            _ => kept.push((k, v)),
        }
    }

    match (biz, mid, idx) {
        (Some(b), Some(m), Some(i)) => format!("{b}/{m}/{i}"),
        _ => {
            // 短链 `s/<code>` 这类没有参数的文章，路径本身就是标识。
            let kept_query = kept
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&");
            if kept_query.is_empty() {
                path.to_string()
            } else {
                format!("{path}?{kept_query}")
            }
        }
    }
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

    #[test]
    fn article_key_ignores_chksm_scene_and_fragment() {
        // 同一篇文章的三种真实拼法：搜狗带 chksm+#rd、主页带 scene+#wechat_redirect、
        // 合集不带 chksm。按 URL 字符串比较会算成三篇。
        let a = "https://mp.weixin.qq.com/s?__biz=Mzg2&mid=2247&idx=1&sn=aa&chksm=ce97#rd";
        let b = "https://mp.weixin.qq.com/s?__biz=Mzg2&mid=2247&idx=1&sn=aa&scene=19#wechat_redirect";
        let c = "https://mp.weixin.qq.com/s?__biz=Mzg2&mid=2247&idx=1&sn=aa";
        assert_eq!(article_key(a), article_key(b));
        assert_eq!(article_key(b), article_key(c));
    }

    #[test]
    fn article_key_separates_different_articles() {
        let first = "https://mp.weixin.qq.com/s?__biz=Mzg2&mid=1&idx=1&sn=aa";
        let second = "https://mp.weixin.qq.com/s?__biz=Mzg2&mid=2&idx=1&sn=aa";
        // 同一 mid 的次条（idx=2）是另一篇文章，不能并成一条。
        let second_in_batch = "https://mp.weixin.qq.com/s?__biz=Mzg2&mid=1&idx=2&sn=aa";
        assert_ne!(article_key(first), article_key(second));
        assert_ne!(article_key(first), article_key(second_in_batch));
    }

    #[test]
    fn article_key_falls_back_to_short_link() {
        let a = "https://mp.weixin.qq.com/s/aW42E-SFYKU8qRmJMGZsNQ";
        let b = "https://mp.weixin.qq.com/s/aW42E-SFYKU8qRmJMGZsNQ#rd";
        assert_eq!(article_key(a), article_key(b));
        assert_ne!(
            article_key(a),
            article_key("https://mp.weixin.qq.com/s/otherCodeHere")
        );
    }
}
