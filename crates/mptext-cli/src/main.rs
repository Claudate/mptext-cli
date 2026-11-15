//! 命令行入口。搜号和拉列表走本机保存的公众平台扫码会话，不再使用 down.mptext.top 的密钥。
//! 单篇文章是公开页，不登录也能下。

mod platform;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use mptext_core::{
    ArticleItem, WechatClient, config_path, load_config, load_session, safe_filename, save_config,
    session_path, unique_path, write_file,
};
use platform::{pause_before_exit_if_needed, print_welcome};
use tokio::time::{Duration, sleep};

#[derive(Parser)]
#[command(
    name = "mptext",
    about = "微信公众号文章下载（扫码登录公众平台后台）",
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
    /// 微信扫码登录公众号后台，会话保存在本机
    Login,
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
    /// 按公众号名全量导出：登录态全部历史 + 主页栏目 + 搜狗索引合并去重后批量下载
    Export {
        /// 公众号名称，可给多个：mptext export 远方青木 占豪
        #[arg(required_unless_present = "biz")]
        names: Vec<String>,
        /// 直接指定 __biz 跳过登录态搜号。主页栏目与合集都是免登录公开接口，
        /// 登录态失效时仍可导：mptext export --biz MzI1NjA0MDg2Mw== 夜听
        #[arg(long)]
        biz: Option<String>,
        #[arg(long, default_value = "markdown")]
        format: String,
        #[arg(short, long, default_value = "output")]
        output_dir: PathBuf,
        /// 每篇之间的间隔秒数。0.5 实测安全，1.5 更稳
        #[arg(short, long, default_value_t = 1.5)]
        interval: f64,
        /// 每个号最多下载多少篇（0 = 不限）
        #[arg(short, long, default_value_t = 0)]
        limit: u32,
        /// 只列出文章、不下载
        #[arg(long)]
        list_only: bool,
    },
    Config {
        #[command(subcommand)]
        action: ConfigCommands,
    },
    /// 从微信流量里尝试抓一次历史列表凭据（PC 端实测拿不到，见 docs/wechat-pc-channel-findings.md）
    Ticket {
        /// 抓包端口，默认 8080
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
        /// 抓包插件路径
        #[arg(long, default_value = "tools/wxmitm_addon.py")]
        addon: PathBuf,
    },
    /// 作者历史通道：用抓到的票据 + 文章链接，拉全量历史（不受每栏目 30 篇限制）
    Author {
        /// 该公众号任意一篇文章的链接，用来反查 author_id
        url: String,
        /// 最多拉多少页（每页 30 条）。0 = 翻到底
        #[arg(long, default_value_t = 0)]
        max_pages: u32,
        /// 只列出、不下载
        #[arg(long)]
        list_only: bool,
        #[arg(long, default_value = "markdown")]
        format: String,
        #[arg(short, long, default_value = "output")]
        output_dir: PathBuf,
        /// 每篇之间的间隔秒数
        #[arg(short, long, default_value_t = 1.5)]
        interval: f64,
    },
    /// MITM 抓包：从微信客户端流量里抓取 uin/key/pass_ticket
    Mitm {
        #[command(subcommand)]
        action: MitmCommands,
    },
}

#[derive(Subcommand)]
enum MitmCommands {
    /// 生成本机根证书并输出路径
    Ca,
    /// 把根证书装进系统信任列表（需要管理员权限）
    Trust,
    /// 开启/关闭系统代理
    Proxy {
        #[arg(long, default_value = "on")]
        state: String,
        #[arg(short, long, default_value_t = mptext_core::MITM_DEFAULT_PORT)]
        port: u16,
    },
    /// 启动本地抓包代理（前台运行，Ctrl-C 结束）
    Start {
        #[arg(short, long, default_value_t = mptext_core::MITM_DEFAULT_PORT)]
        port: u16,
    },
    /// 查看已抓到的公众号票据
    Status,
    /// 清空已抓票据
    Clear,
}

#[derive(Subcommand)]
enum ConfigCommands {
    SetToken { token: String },
    Show,
}

fn open_session() -> Result<WechatClient> {
    WechatClient::from_saved_session().context(
        "尚未登录微信公众平台。\n\
         请先运行: mptext login\n\
         用微信扫描二维码，登录任意一个你有权限的公众号后台。",
    )
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

    if cli.token.is_some() || cli.base_url.is_some() {
        eprintln!("提示: down.mptext.top 的密钥已停用，搜索和列表改为本机扫码登录（mptext login）");
    }

    // 登录、单篇下载、从链接认号都不依赖后台会话；搜号和拉列表才要。
    match cli.command.expect("checked above") {
        Commands::Login => login().await?,
        Commands::Download { url, format, output } => {
            let client = WechatClient::new(mptext_core::MpSession::default())?;
            let content = client.download_article(&url, &format).await?;
            if let Some(path) = output {
                write_file(&path, &content)?;
                println!("已保存: {}", path.display());
            } else {
                print!("{content}");
            }
        }
        Commands::Account { url } => {
            let client = WechatClient::new(mptext_core::MpSession::default())?;
            let info = client.account_by_url(&url).await?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        Commands::Auth => {
            let mut client = open_session()?;
            if client.verify().await? {
                let nick = client.session().nickname.clone();
                if nick.is_empty() {
                    println!("登录会话有效");
                } else {
                    println!("登录会话有效（{nick}）");
                }
            } else {
                println!("登录已失效，请重新运行 mptext login");
                std::process::exit(1);
            }
        }
        Commands::Search { keyword } => {
            let mut client = open_session()?;
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
            let mut client = open_session()?;
            // 从accounts.json读取nickname
            let nickname = mptext_core::config::load_accounts()
                .iter()
                .find(|acc| acc.fakeid == fakeid)
                .map(|acc| acc.nickname.clone())
                .unwrap_or_default();
            let articles = client
                .list_articles(&fakeid, &nickname, begin, size, keyword.as_deref())
                .await?;
            print_articles(&articles);
        }
        Commands::Fetch {
            fakeid,
            limit,
            format,
            output_dir,
            interval,
        } => {
            let mut client = open_session()?;
            fetch_batch(&mut client, &fakeid, limit, &format, &output_dir, interval).await?;
        }
        Commands::Export {
            names,
            biz,
            format,
            output_dir,
            interval,
            limit,
            list_only,
        } => {
            export_accounts(
                &names,
                biz.as_deref(),
                &format,
                &output_dir,
                interval,
                limit,
                list_only,
            )
            .await?
        }
        Commands::Ticket { port, addon } => ticket(port, &addon).await?,
        Commands::Author {
            url,
            max_pages,
            list_only,
            format,
            output_dir,
            interval,
        } => {
            author_channel(&url, max_pages, list_only, &format, &output_dir, interval).await?
        }
        Commands::Mitm { action } => handle_mitm(action).await?,
        Commands::Config { .. } => unreachable!(),
    }

    Ok(())
}

async fn login() -> Result<()> {
    let mut client = WechatClient::new(mptext_core::MpSession::default())?;
    let qr_shown = std::cell::Cell::new(false);
    let session = client
        .login_qr(
            |path| {
                if qr_shown.replace(true) {
                    println!("二维码已刷新: {}", path.display());
                } else {
                    println!("二维码已保存: {}", path.display());
                }
                reveal_qr(path);
            },
            |status| println!("{status}"),
        )
        .await?;
    let who = if session.nickname.is_empty() {
        "公众号后台".to_string()
    } else {
        session.nickname
    };
    println!("登录成功：{who}");
    println!("会话已保存 → {}", session_path()?.display());
    println!("接着可以: mptext search \"公众号名\"");
    Ok(())
}

fn reveal_qr(path: &std::path::Path) {
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(path).status()
    } else if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &path.display().to_string()])
            .status()
    } else {
        std::process::Command::new("xdg-open").arg(path).status()
    };
    if result.is_err() {
        eprintln!("无法自动打开二维码，请手动打开上面的图片，用微信扫描");
    }
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
            let session = load_session().unwrap_or_default();
            if session.token.is_empty() {
                println!("  微信登录: 未登录（运行 mptext login）");
            } else {
                let who = if session.nickname.is_empty() {
                    "已登录".to_string()
                } else {
                    session.nickname
                };
                println!("  微信登录: {who}");
                println!("  会话文件: {}", session_path()?.display());
            }
        }
    }
    Ok(())
}

async fn fetch_batch(
    client: &mut WechatClient,
    fakeid: &str,
    limit: u32,
    format: &str,
    output_dir: &Path,
    interval_secs: f64,
) -> Result<()> {
    // 从accounts.json读取nickname
    let nickname = mptext_core::config::load_accounts()
        .iter()
        .find(|acc| acc.fakeid == fakeid)
        .map(|acc| acc.nickname.clone())
        .unwrap_or_default();
    
    let page_size = limit.clamp(1, 10);
    let mut collected = Vec::new();
    let mut begin = 0u32;

    while collected.len() < limit as usize {
        let batch = client
            .list_articles(fakeid, &nickname, begin, page_size, None)
            .await?;
        if batch.is_empty() {
            break;
        }
        let short_page = batch.len() < 10;
        let remaining = limit as usize - collected.len();
        collected.extend(batch.into_iter().take(remaining));
        if collected.len() >= limit as usize || short_page {
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

/// 作者历史通道：用一篇文章反查 `author_id`，再用抓到的票据换 Cookie 并翻全量。
///
/// 这条路绕开了主页栏目 30 篇、合集单页 20 篇的限制，是拿到完整历史的通道。
async fn author_channel(
    url: &str,
    max_pages: u32,
    list_only: bool,
    format: &str,
    output_dir: &Path,
    interval_secs: f64,
) -> Result<()> {
    println!("反查作者标识: {url}");
    let (author_id, biz) = mptext_core::resolve_article(url).await?;
    let author_id = author_id.context("文章页里没有 author_id，换一篇该公众号的文章再试")?;
    println!("  author_id = {author_id}");
    if let Some(biz) = &biz {
        println!("  __biz     = {biz}");
    }

    let mut cred = mptext_core::capture_for(biz.as_deref().unwrap_or_default())
        .context(
            "还没有抓到微信票据。\n\
             先运行: mptext mitm start\n\
             然后在微信里打开该公众号的任意一篇文章，让票据落盘后再试。",
        )?;
    if !cred.is_complete() {
        bail!("抓到的票据不完整（缺 uin / key），请重新抓一次");
    }
    if cred.author_id.is_empty() {
        cred.author_id = author_id.clone();
    }
    if let Some(biz) = biz {
        cred.biz = biz;
    }

    let client = mptext_core::AuthorClient::new()?;
    println!("换取作者 Cookie…");
    client.exchange_cookie(&mut cred).await?;
    println!("  Cookie 有效期至 {}（约 24 小时）", cred.cookie_expires_at);

    println!("拉取历史列表（每页 30 条，游标 from_article_id）…");
    let articles = client.list_all(&mut cred, &author_id, max_pages).await?;
    println!("  共 {} 篇", articles.len());
    if articles.is_empty() {
        return Ok(());
    }
    if list_only {
        print_articles(&articles);
        return Ok(());
    }

    let dir = output_dir.join(if cred.nickname.is_empty() {
        "author".to_string()
    } else {
        cred.nickname.clone()
    });
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("创建输出目录失败: {}", dir.display()))?;

    // 下载走的是公开文章页，不需要票据；票据只用于列表。
    let downloader = WechatClient::new(mptext_core::MpSession::default())?;
    let mut saved = 0usize;
    let mut failed = 0usize;
    for (idx, article) in articles.iter().enumerate() {
        if idx > 0 && interval_secs > 0.0 {
            sleep(Duration::from_secs_f64(interval_secs)).await;
        }
        match downloader.download_article(&article.url, format).await {
            Ok(content) => {
                let filename = safe_filename(&article.title, format);
                let path = unique_path(&dir, &filename);
                write_file(&path, &content)?;
                println!("  [{}] {}", idx + 1, path.display());
                saved += 1;
            }
            Err(err) => {
                eprintln!("  [{}] 失败: {} — {err}", idx + 1, article.title);
                failed += 1;
            }
        }
    }
    println!("完成：成功 {saved} 篇，失败 {failed} 篇 → {}", dir.display());
    Ok(())
}

/// 抓包相关子命令。
async fn handle_mitm(action: MitmCommands) -> Result<()> {
    match &action {
        MitmCommands::Ca => {
            let (cert, key) = mptext_core::mitm::ensure_ca()?;
            println!("CA 证书: {}", cert.display());
            println!("CA 私钥: {}", key.display());
            println!("\n把证书装进系统信任列表（需要管理员密码）：");
            println!("  mptext mitm trust");
        }
        MitmCommands::Trust => {
            mptext_core::mitm::trust_ca()?;
            println!("CA 已装进系统钥匙串并标记为信任");
        }
        MitmCommands::Proxy { state, port } => {
            let enabled = state != "off";
            mptext_core::mitm::set_system_proxy(enabled, "127.0.0.1", *port)?;
            println!(
                "系统代理已{}",
                if enabled { format!("指向 127.0.0.1:{port}") } else { "关闭".to_string() }
            );
        }
        MitmCommands::Status => {
            let status = mptext_core::mitm::status();
            println!("代理运行: {}", if status.running { "是" } else { "否" });
            if status.running {
                println!("监听端口: {}", status.port);
            }
            println!("已抓公众号: {}", status.captured_accounts);
            println!("{}", status.message);
            for (biz, cred) in mptext_core::load_captures() {
                println!(
                    "  {biz}  uin={}  key={}…  Cookie={}",
                    if cred.uin.is_empty() { "-" } else { "有" },
                    cred.key.chars().take(6).collect::<String>(),
                    if cred.cookie_valid() { "有效" } else { "无/过期" }
                );
            }
        }
        MitmCommands::Clear => {
            mptext_core::clear_captures()?;
            println!("已清空抓到的票据");
        }
        MitmCommands::Start { port } => {
            let port = *port as u16;
            let (cert, _) = mptext_core::mitm::ensure_ca()?;
            println!("CA 证书: {}", cert.display());
            if !mptext_core::mitm::ca_trusted() {
                println!("⚠ 系统还不信任这个 CA。首次使用需要执行 mptext mitm trust");
            }
            mptext_core::mitm::start(port).await?;
            println!("抓包代理已启动: 127.0.0.1:{port}");
            if std::env::var("MPTEXT_NO_PROXY_SETUP").is_err() {
                println!("正在把系统代理指到这个端口（需要管理员密码，取消则手动设置）…");
                match mptext_core::mitm::set_system_proxy(true, "127.0.0.1", port) {
                    Ok(()) => println!("系统代理已开启"),
                    Err(err) => println!("系统代理未开启：{err}\n  请手动把 HTTP/HTTPS 代理设为 127.0.0.1:{port}"),
                }
            }
            println!("\n现在去微信里打开该公众号的任意一篇文章，票据会自动落盘。");
            println!("抓完按 Ctrl-C 结束（会尝试恢复系统代理）。");
            let _ = tokio::signal::ctrl_c().await;
            let _ = mptext_core::mitm::set_system_proxy(false, "127.0.0.1", port);
            mptext_core::mitm::stop();
            println!("已停止抓包并恢复系统代理");
        }
    }
    Ok(())
}

/// 启动 mitmproxy 抓一次历史列表凭据，并等到票据文件出现后报告结果。
///
/// **PC 端实测无效（2026-10-19）**：macOS 微信 4.1.13 的内置浏览器虽然走系统代理，
/// 但不建立网页登录态（Cookie 里没有 `wap_sid2`），抓到的 `uin`/`key` 换不出列表，
/// `profile_ext` 一律回「请在微信客户端打开」或 `ret:-3 no session`。
/// 依据见 `docs/wechat-pc-channel-findings.md`。
///
/// 保留这个命令是为了移动端会话或将来客户端行为变化时还能用同一套抓包链路，
/// 不要在文档或帮助里把它描述成能拿到全量历史的常规手段。
async fn ticket(port: u16, addon: &Path) -> Result<()> {
    if !addon.exists() {
        bail!(
            "找不到抓包插件 {}。请在仓库根目录运行，或加 --addon 指定路径",
            addon.display()
        );
    }
    let path = mptext_core::ticket_path()?;
    if path.exists() {
        std::fs::remove_file(&path).ok();
    }

    println!("启动 mitmproxy，监听 127.0.0.1:{port}");
    println!("注意：PC 微信实测拿不到可用的历史列表凭据，这一步大概率抓不到东西。");
    println!("      依据见 docs/wechat-pc-channel-findings.md（2026-10-19 实测）。");
    println!("接下来如果仍要试：");
    println!("  1. 让微信的流量走这个代理（127.0.0.1:{port}）");
    println!("  2. 打开任一个公众号的主页 →「全部」");
    println!("  3. 本程序检测到凭据会自动退出，然后运行 mptext export <公众号名>");
    println!("要全量历史，更可靠的是 mptext login 扫码登录公众号后台（仅限你有权限的号）。");
    println!("票据将写入 {}\n", path.display());

    let mut child = std::process::Command::new("mitmdump")
        .args(["-s"])
        .arg(addon)
        .args(["-p", &port.to_string(), "--set", "block_global=false"])
        .spawn()
        .context("启动 mitmdump 失败，请确认已安装 mitmproxy（brew install mitmproxy）")?;

    // 等票据落盘：默认最多 10 分钟，按秒轮询。插件是原子写入，读到就能用。
    let deadline = std::time::Instant::now() + Duration::from_secs(600);
    let mut captured = None;
    while std::time::Instant::now() < deadline {
        sleep(Duration::from_secs(1)).await;
        if let Ok(Some(t)) = mptext_core::load_ticket() {
            captured = Some(t);
            break;
        }
    }

    // 抓到就收工；超时也把 mitmdump 停掉，免得占着端口。
    let _ = child.kill();
    let _ = child.wait();

    match captured {
        Some(t) if !t.url.is_empty() => {
            println!("\n已捕获票据（{}）", t.captured_at);
            println!("写入: {}", path.display());
            let has_key = t.url.contains("key=") && t.url.contains("uin=");
            if has_key {
                println!("包含 uin/key，可以直接运行: mptext export 英国那些事儿");
            } else {
                println!("警告: URL 里没有 uin/key，可能是首页请求而非历史消息，请重新抓一次");
            }
            let _ = t;
        }
        _ => {
            println!("\n10 分钟内没有捕获到凭据。检查：");
            println!("  - 流量是否真的走了 127.0.0.1:{port} 代理");
            println!("  - 是否打开了公众号主页的「全部」列表");
        }
    }
    Ok(())
}

/// 按公众号名全量导出。
///
/// 三条来源合并去重，覆盖范围各不相同：
/// - `profile_ext` 全部历史：登录后自动取凭据，这是唯一能翻到几千篇的通道；
/// - `/mp/homepage`：免登录，每号最多 4 个栏目 × 30 篇；
/// - 搜狗移动搜索：只索引到很浅一层，用来兜住主页完全没栏目的号。
///
/// 有登录会话时全部历史排在最前，后两条只用来补漏。
async fn export_accounts(
    names: &[String],
    biz_override: Option<&str>,
    format: &str,
    output_dir: &Path,
    interval_secs: f64,
    limit: u32,
    list_only: bool,
) -> Result<()> {
    // 公开通道（主页栏目 / 搜狗 / 合集）不读会话，用空会话建客户端即可；
    // 登录会话只决定能不能多走「全部历史」那一条。
    let mut client = WechatClient::new(mptext_core::MpSession::default())?;
    let logged_in = match WechatClient::from_saved_session() {
        Ok(session_client) => {
            client = session_client;
            true
        }
        Err(err) => {
            println!("未登录（{err}）——只走免登录通道");
            false
        }
    };
    // 抓包票据只作兜底：登录会话自己就能换到 key/uin。
    let ticket = mptext_core::load_ticket()?;

    // --biz 给了就直接用，不再依赖登录态搜号（昵称只用于日志和导出目录）。
    let targets: Vec<(String, String)> = match biz_override {
        Some(biz) => vec![(
            names.first().cloned().unwrap_or_else(|| biz.to_string()),
            biz.to_string(),
        )],
        None => {
            if !logged_in {
                bail!("没有登录会话，无法按昵称搜号。请加 --biz 直接指定 __biz");
            }
            let mut resolved = Vec::new();
            for name in names {
                println!("\n===== {name} =====");
                let accounts = client.search_accounts(name).await?;
                // 搜索按关键词匹配，取昵称完全一致的那条，避免抓到同名号。
                match accounts
                    .iter()
                    .find(|a| a.nickname == *name)
                    .or_else(|| accounts.first())
                {
                    Some(account) => resolved.push((account.nickname.clone(), account.fakeid.clone())),
                    None => println!("未找到该公众号，跳过"),
                }
            }
            resolved
        }
    };

    for (nickname, fakeid) in targets {
        println!("\n===== {nickname} =====");
        println!("fakeid = {fakeid}");

        let mut seen = std::collections::HashSet::new();
        let mut articles = Vec::new();
        // 用不捕获环境的局部函数而不是闭包：闭包会一直可变借用 articles，
        // 后面挑合集种子时还要不可变读它，两者不能共存。
        fn merge_push(
            articles: &mut Vec<ArticleItem>,
            seen: &mut std::collections::HashSet<String>,
            list: Vec<ArticleItem>,
            source: &str,
        ) {
            let before = articles.len();
            for article in list {
                if seen.insert(mptext_core::article_key(&article.url)) {
                    articles.push(article);
                }
            }
            println!("  {source}: 新增 {} 篇", articles.len() - before);
        }

        // 三条公开通道都不读登录态，会话过期时照常可用。
        merge_push(
            &mut articles,
            &mut seen,
            client
                .list_articles_homepage(&fakeid, 0, u32::MAX, None)
                .await
                .unwrap_or_default(),
            "主页栏目",
        );
        merge_push(
            &mut articles,
            &mut seen,
            client
                .list_articles_sogou_all(&fakeid, &nickname, 10)
                .await
                .unwrap_or_default(),
            "搜狗索引",
        );
        // 合集通道：免登录、可翻到底。合集 id 只能从已有文章页反查，
        // 一个号常有多个互不相通的合集（实测「官察笔记」：种子只挂合集 A，
        // A 的文章才挂合集 B，B 有 97 篇），所以按轮展开的逻辑放在 core 里，
        // 与 GUI 共用。
        let album_seeds: Vec<String> = articles.iter().map(|a| a.url.clone()).collect();
        let album_articles = client
            .list_all_albums(&fakeid, &album_seeds, |album_id, total| {
                println!("  合集 {album_id}: 累计 {total} 篇");
            })
            .await;
        merge_push(&mut articles, &mut seen, album_articles, "合集");
        // 最后才是登录态：这是唯一能翻到全部历史的通道，覆盖远大于上面几条。
        if logged_in {
            match client.list_articles_full_history(&fakeid, 0).await {
                Ok(list) => merge_push(&mut articles, &mut seen, list, "登录态(全部历史)"),
                Err(err) => {
                    println!("  登录态: 不可用（{err}）");
                    if let Some(ticket) = &ticket {
                        match client.list_articles_profile_ext(&fakeid, ticket, 0).await {
                            Ok(list) => {
                                merge_push(&mut articles, &mut seen, list, "抓包票据(全部历史)")
                            }
                            Err(err) => println!("  抓包票据: 不可用（{err}）"),
                        }
                    }
                }
            }
        } else {
            println!("  登录态: 未登录，跳过全部历史通道");
        }

        if articles.is_empty() {
            println!("  没有拿到任何文章");
            continue;
        }
        // 主页和票据都带发布时间；没有时间的排到最后。
        articles.sort_by_key(|a| {
            std::cmp::Reverse(a.create_time.as_ref().and_then(|v| v.as_i64()).unwrap_or(0))
        });
        if limit > 0 {
            articles.truncate(limit as usize);
        }
        println!("  合计 {} 篇", articles.len());

        if list_only {
            print_articles(&articles);
            continue;
        }

        // 目录名用昵称本身：safe_filename 是给文章文件名加扩展名的，套在目录上会变成「浙江宣传.md」。
        let dir = output_dir.join(&nickname);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("创建输出目录失败: {}", dir.display()))?;

        let mut saved = 0usize;
        let mut failed = 0usize;
        for (idx, article) in articles.iter().enumerate() {
            if idx > 0 && interval_secs > 0.0 {
                sleep(Duration::from_secs_f64(interval_secs)).await;
            }
            match client.download_article(&article.url, format).await {
                Ok(content) => {
                    let filename = safe_filename(&article.title, format);
                    let path = unique_path(&dir, &filename);
                    write_file(&path, &content)?;
                    println!("  [{}] {}", idx + 1, path.display());
                    saved += 1;
                }
                // 单篇失败不该中断整批：记下来最后汇总，剩下的继续下。
                Err(err) => {
                    eprintln!("  [{}] 失败: {} — {err}", idx + 1, article.title);
                    failed += 1;
                }
            }
        }
        println!("完成：成功 {saved} 篇，失败 {failed} 篇 → {}", dir.display());
    }
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
