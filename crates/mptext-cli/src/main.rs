mod platform;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mptext_core::{
    ArticleItem, MptextClient, config_path, default_base_url, load_config, safe_filename,
    save_config, unique_path, write_file,
};
use platform::{pause_before_exit_if_needed, print_welcome};
use tokio::time::{Duration, sleep};

#[derive(Parser)]
#[command(
    name = "mptext",
    about = "mptext.top 公众号文章 API 命令行工具",
    version,
    arg_required_else_help = false
)]
struct Cli {
    #[arg(short, long, env = "MPTEXT_AUTH_KEY", global = true)]
    token: Option<String>,

    #[arg(long, global = true)]
    base_url: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Auth,
    Search { keyword: String },
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
    Download {
        url: String,
        #[arg(short, long, default_value = "markdown")]
        format: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    Account { url: String },
    Fetch {
        #[arg(short, long)]
        fakeid: String,
        #[arg(short, long, default_value_t = 5)]
        limit: u32,
        #[arg(long, default_value = "markdown")]
        format: String,
        #[arg(short, long, default_value = "output")]
        output_dir: PathBuf,
        #[arg(short, long, default_value_t = 1.0)]
        interval: f64,
    },
    Config {
        #[command(subcommand)]
        action: ConfigCommands,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    SetToken { token: String },
    Show,
}

fn resolve_credentials(cli: &Cli) -> Result<(String, String)> {
    let file_cfg = load_config().unwrap_or_default();

    let token = cli
        .token
        .clone()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| {
            if !file_cfg.auth_key.trim().is_empty() {
                Some(file_cfg.auth_key.clone())
            } else {
                None
            }
        })
        .context(
            "缺少 API token。\n\
             请任选一种方式配置：\n\
             1) mptext config set-token <密钥>\n\
             2) set MPTEXT_AUTH_KEY=密钥（Windows CMD）\n\
             3) mptext --token 密钥 auth",
        )?;

    let base_url = cli
        .base_url
        .clone()
        .filter(|u| !u.trim().is_empty())
        .or(file_cfg.base_url)
        .unwrap_or_else(|| default_base_url().to_string());

    Ok((token, base_url))
}

#[tokio::main]
async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.as_ref() {
        None => {
            print_welcome();
            return Ok(());
        }
        Some(Commands::Config { action }) => return handle_config(action).await,
        _ => {}
    }

    let (token, base_url) = resolve_credentials(&cli)?;
    let client = MptextClient::new(base_url, token)?;

    match cli.command.expect("checked above") {
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
                let alias = acc.alias.as_deref().unwrap_or("-");
                println!("{}\t{}\t{}", acc.nickname, acc.fakeid, alias);
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
        Commands::Config { .. } => unreachable!(),
    }

    Ok(())
}

async fn handle_config(action: &ConfigCommands) -> Result<()> {
    match action {
        ConfigCommands::SetToken { token } => {
            if token.trim().is_empty() {
                anyhow::bail!("token 不能为空");
            }
            let mut cfg = load_config().unwrap_or_default();
            cfg.auth_key = token.trim().to_string();
            let path = save_config(&cfg)?;
            println!("已保存 API 密钥 → {}", path.display());
            println!("运行 mptext auth 验证是否有效");
        }
        ConfigCommands::Show => {
            let path = config_path()?;
            let cfg = load_config().unwrap_or_default();
            println!("配置文件: {}", path.display());
            println!(
                "  auth_key: {}",
                if cfg.auth_key.is_empty() {
                    "未设置".to_string()
                } else {
                    format!("已设置 ({} 字符)", cfg.auth_key.len())
                }
            );
            if let Some(url) = cfg.base_url {
                println!("  base_url: {url}");
            }
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
    let page_size = limit.clamp(1, 20);
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
        let path = unique_path(output_dir, &filename);
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

fn main() {
    let exit_code = match run() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("错误: {err:#}");
            1
        }
    };
    pause_before_exit_if_needed(exit_code);
    std::process::exit(exit_code);
}
