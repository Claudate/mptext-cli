mod client;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use client::{ArticleItem, MptextClient, default_base_url};
use tokio::time::{Duration, sleep};

#[derive(Parser)]
#[command(
    name = "mptext",
    about = "mptext.top 公众号文章 API 命令行工具",
    version
)]
struct Cli {
    /// API 密钥（也可设环境变量 MPTEXT_AUTH_KEY）
    #[arg(short, long, env = "MPTEXT_AUTH_KEY", global = true)]
    token: Option<String>,

    /// API 基础地址
    #[arg(long, default_value = default_base_url(), global = true)]
    base_url: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 验证 API 密钥是否有效
    Auth,
    /// 按关键词搜索公众号
    Search {
        keyword: String,
    },
    /// 获取公众号文章列表
    Articles {
        #[arg(short, long)]
        fakeid: String,
        #[arg(long, default_value_t = 0)]
        begin: u32,
        #[arg(short, long, default_value_t = 20)]
        size: u32,
        #[arg(short, long)]
        keyword: Option<String>,
    },
    /// 下载单篇文章
    Download {
        url: String,
        #[arg(short, long, default_value = "markdown")]
        format: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// 通过文章 URL 反查公众号信息
    Account {
        url: String,
    },
    /// 批量抓取：拉列表 + 逐篇下载到目录
    Fetch {
        #[arg(short, long)]
        fakeid: String,
        #[arg(short, long, default_value_t = 5)]
        limit: u32,
        #[arg(short, long, default_value = "markdown")]
        format: String,
        #[arg(short, long, default_value = "output")]
        output_dir: PathBuf,
        #[arg(short, long, default_value_t = 1.0)]
        interval: f64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let token = cli
        .token
        .filter(|t| !t.trim().is_empty())
        .context("缺少 API token：使用 --token 或设置 MPTEXT_AUTH_KEY")?;

    let client = MptextClient::new(cli.base_url, token)?;

    match cli.command {
        Commands::Auth => {
            let ok = client.verify_auth().await?;
            if ok {
                println!("API 密钥有效");
            } else {
                println!("API 密钥已过期，请重新登录 mptext.top 获取新密钥");
                std::process::exit(1);
            }
        }
        Commands::Search { keyword } => {
            let accounts = client.search_accounts(&keyword).await?;
            if accounts.is_empty() {
                println!("未找到匹配的公众号");
                return Ok(());
            }
            for acc in accounts {
                println!("{}\t{}", acc.nickname, acc.fakeid);
            }
        }
        Commands::Articles {
            fakeid,
            begin,
            size,
            keyword,
        } => {
            let articles = client
                .list_articles(&fakeid, begin, size, keyword.as_deref())
                .await?;
            print_articles(&articles);
        }
        Commands::Account { url } => {
            let info = client.account_by_url(&url).await?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        Commands::Download { url, format, output } => {
            let content = client.download_article(&url, &format).await?;
            if let Some(path) = output {
                write_file(&path, &content)?;
                println!("已保存: {}", path.display());
            } else {
                print!("{content}");
            }
        }
        Commands::Fetch {
            fakeid,
            limit,
            format,
            output_dir,
            interval,
        } => {
            fetch_batch(&client, &fakeid, limit, &format, &output_dir, interval).await?;
        }
    }

    Ok(())
}

async fn fetch_batch(
    client: &MptextClient,
    fakeid: &str,
    limit: u32,
    format: &str,
    output_dir: &Path,
    interval_secs: f64,
) -> Result<()> {
    let page_size = limit.min(20).max(1);
    let mut collected = Vec::new();
    let mut begin = 0u32;

    while collected.len() < limit as usize {
        let batch = client
            .list_articles(fakeid, begin, page_size, None)
            .await?;
        if batch.is_empty() {
            break;
        }
        let remaining = limit as usize - collected.len();
        collected.extend(batch.into_iter().take(remaining));
        if collected.len() >= limit as usize {
            break;
        }
        begin += page_size;
        if begin > 10_000 {
            break;
        }
    }

    if collected.is_empty() {
        println!("未获取到文章");
        return Ok(());
    }

    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;

    let mut saved = 0usize;
    for (idx, article) in collected.iter().enumerate() {
        if idx > 0 && interval_secs > 0.0 {
            sleep(Duration::from_secs_f64(interval_secs)).await;
        }

        let content = client.download_article(&article.url, format).await?;
        let filename = safe_filename(&article.title, format);
        let path = output_dir.join(filename);
        write_file(&path, &content)?;
        println!("[{}] {}", idx + 1, path.display());
        saved += 1;
    }

    println!("完成：共保存 {saved} 篇 → {}", output_dir.display());
    Ok(())
}

fn print_articles(articles: &[ArticleItem]) {
    for (i, article) in articles.iter().enumerate() {
        let ts = article
            .create_time
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string());
        println!("{}. [{}] {}\n   {}", i + 1, ts, article.title, article.url);
    }
}

fn safe_filename(title: &str, format: &str) -> String {
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

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content).with_context(|| format!("写入文件失败: {}", path.display()))
}
