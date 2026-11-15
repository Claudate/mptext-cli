use std::path::PathBuf;
use std::sync::Mutex;

use mptext_core::{
    AccountItem, ArticleItem, MpSession, WechatClient, clear_accounts, clear_session, config_path,
    load_accounts, load_session, safe_filename, save_accounts, session_path, unique_path,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    /// 同一时刻只跑一次扫码，避免两个登录把同一张二维码互相覆盖。
    logging_in: Mutex<bool>,
}

#[derive(Debug, Serialize)]
struct SettingsInfo {
    config_path: String,
    session_path: String,
    logged_in: bool,
    nickname: String,
}

#[derive(Clone, Serialize)]
struct QrPayload {
    image: String,
}

#[derive(Debug, Deserialize)]
struct ListArticlesRequest {
    fakeid: String,
    begin: u32,
    size: u32,
}

#[derive(Debug, Deserialize)]
struct DownloadRequest {
    articles: Vec<ArticleItem>,
    format: String,
    output_dir: String,
    interval_secs: f64,
}

#[derive(Debug, Serialize, Clone)]
struct DownloadProgress {
    current: usize,
    total: usize,
    title: String,
    /// 当前文章链接，前端据此精确定位行（避免靠标题匹配）
    url: String,
    status: String,
    saved_path: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct FailedItem {
    title: String,
    url: String,
    error: String,
}

#[derive(Debug, Serialize, Clone)]
struct DownloadResult {
    saved: usize,
    failed: Vec<FailedItem>,
}

fn map_err(err: impl ToString) -> String {
    err.to_string()
}

fn current_settings() -> Result<SettingsInfo, String> {
    let cfg_path = config_path().map_err(map_err)?;
    let sess_path = session_path().map_err(map_err)?;
    let session = load_session().map_err(map_err)?;
    let logged_in = !session.token.trim().is_empty() && !session.cookies.is_empty();
    Ok(SettingsInfo {
        config_path: cfg_path.display().to_string(),
        session_path: sess_path.display().to_string(),
        logged_in,
        nickname: session.nickname,
    })
}

/// 搜索和拉列表要用本机已保存的公众号后台会话。
fn open_wechat() -> Result<WechatClient, String> {
    let session = load_session().map_err(map_err)?;
    if session.token.trim().is_empty() || session.cookies.is_empty() {
        return Err("尚未登录。请到设置页用微信扫码登录一个你有权限的公众号后台".to_string());
    }
    WechatClient::new(session).map_err(map_err)
}

/// 单篇文章是公开页，下正文不需要后台会话。
fn open_public() -> Result<WechatClient, String> {
    WechatClient::new(MpSession::default()).map_err(map_err)
}

/// 抓取与批量下载用：有登录态就带上（能翻全部历史），没有就退成免登录。
/// 主页栏目与搜狗这两条通道本身不需要会话，没登录也应该能抓。
fn open_any() -> Result<WechatClient, String> {
    match load_session() {
        Ok(session) if !session.token.trim().is_empty() && !session.cookies.is_empty() => {
            WechatClient::new(session).map_err(map_err)
        }
        _ => WechatClient::new(MpSession::default()).map_err(map_err),
    }
}

#[tauri::command]
fn get_settings() -> Result<SettingsInfo, String> {
    current_settings()
}

/// 向微信要二维码并等待手机确认。二维码和状态用事件推给界面，成功后会话写入本机。
#[tauri::command]
async fn start_login(app: AppHandle, state: State<'_, AppState>) -> Result<SettingsInfo, String> {
    {
        let mut guard = state
            .logging_in
            .lock()
            .map_err(|_| "登录状态锁定失败".to_string())?;
        if *guard {
            return Err("正在等待扫码，请先完成这次登录或等它超时".to_string());
        }
        *guard = true;
    }

    let outcome = run_qr_login(app).await;
    if let Ok(mut guard) = state.logging_in.lock() {
        *guard = false;
    }
    outcome
}

async fn run_qr_login(app: AppHandle) -> Result<SettingsInfo, String> {
    let mut client = WechatClient::new(MpSession::default()).map_err(map_err)?;
    let app_qr = app.clone();
    let app_status = app.clone();
    client
        .login_qr(
            move |path| {
                let Ok(bytes) = std::fs::read(path) else {
                    return;
                };
                let _ = app_qr.emit(
                    "login-qr",
                    QrPayload {
                        image: png_data_url(&bytes),
                    },
                );
            },
            move |status| {
                let _ = app_status.emit("login-status", status.to_string());
            },
        )
        .await
        .map_err(map_err)?;
    current_settings()
}

#[tauri::command]
fn logout() -> Result<SettingsInfo, String> {
    clear_session().map_err(map_err)?;
    current_settings()
}

#[tauri::command]
async fn verify_session() -> Result<bool, String> {
    let mut client = open_wechat()?;
    client.verify().await.map_err(map_err)
}

fn png_data_url(bytes: &[u8]) -> String {
    format!("data:image/png;base64,{}", b64_encode(bytes))
}

fn b64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | data[i + 2] as u32;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
        i += 3;
    }
    if i < data.len() {
        let rem = data.len() - i;
        let mut n = (data[i] as u32) << 16;
        if rem == 2 {
            n |= (data[i + 1] as u32) << 8;
        }
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if rem == 2 {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
            out.push('=');
        } else {
            out.push('=');
            out.push('=');
        }
    }
    out
}

#[tauri::command]
async fn search_accounts(keyword: String) -> Result<Vec<AccountItem>, String> {
    if keyword.trim().is_empty() {
        return Err("请输入搜索关键词".to_string());
    }
    let mut client = open_wechat()?;
    let results = client
        .search_accounts(keyword.trim())
        .await
        .map_err(map_err)?;

    // 自动记忆：把本次结果合并进已存列表（按 fakeid 去重，新结果置顶）
    merge_and_save_accounts(&results);

    Ok(results)
}

/// 合并新搜索结果到记忆列表：按 fakeid 去重、保留旧的收藏标记、收藏项置顶。
fn merge_and_save_accounts(fresh: &[AccountItem]) {
    if fresh.is_empty() {
        return;
    }
    let old = load_accounts();
    // 旧记录中已收藏的 fakeid 集合，用于在新结果上恢复收藏标记
    let fav_ids: std::collections::HashSet<String> = old
        .iter()
        .filter(|a| a.favorite)
        .map(|a| a.fakeid.clone())
        .collect();

    let fresh_ids: std::collections::HashSet<&str> =
        fresh.iter().map(|a| a.fakeid.as_str()).collect();

    let mut merged: Vec<AccountItem> = fresh
        .iter()
        .cloned()
        .map(|mut a| {
            if fav_ids.contains(&a.fakeid) {
                a.favorite = true;
            }
            a
        })
        .collect();

    for o in old {
        if !fresh_ids.contains(o.fakeid.as_str()) {
            merged.push(o);
        }
    }

    merged = sort_accounts(merged);
    merged.truncate(200);
    if let Err(e) = save_accounts(&merged) {
        eprintln!("保存公众号记忆失败: {e}");
    }
}

/// 收藏的公众号排在前面（稳定排序，保持各自相对顺序）
fn sort_accounts(mut list: Vec<AccountItem>) -> Vec<AccountItem> {
    list.sort_by_key(|item| std::cmp::Reverse(item.favorite));
    list
}

#[tauri::command]
fn get_saved_accounts() -> Result<Vec<AccountItem>, String> {
    Ok(sort_accounts(load_accounts()))
}

/// 切换某公众号的收藏状态，返回更新后的完整列表（收藏置顶）
#[tauri::command]
fn toggle_favorite(fakeid: String) -> Result<Vec<AccountItem>, String> {
    let mut list = load_accounts();
    let mut found = false;
    for a in &mut list {
        if a.fakeid == fakeid {
            a.favorite = !a.favorite;
            found = true;
            break;
        }
    }
    if !found {
        return Err("未找到该公众号".to_string());
    }
    list = sort_accounts(list);
    save_accounts(&list).map_err(map_err)?;
    Ok(list)
}

/// 从记忆中删除单个公众号，返回更新后的列表
#[tauri::command]
fn remove_account(fakeid: String) -> Result<Vec<AccountItem>, String> {
    let mut list = load_accounts();
    let before = list.len();
    list.retain(|a| a.fakeid != fakeid);
    if list.len() == before {
        return Err("未找到该公众号".to_string());
    }
    save_accounts(&list).map_err(map_err)?;
    Ok(sort_accounts(list))
}

#[tauri::command]
fn clear_saved_accounts() -> Result<(), String> {
    clear_accounts().map_err(map_err)
}

#[tauri::command]
async fn list_articles(payload: ListArticlesRequest) -> Result<Vec<ArticleItem>, String> {
    // 与抓取/批量下载一致：没登录态时 list_articles 内部会自动回退到主页栏目 + 搜狗，
    // 这样用户在未登录状态下点开某个号也能翻页，不会直接吃一句「尚未登录」。
    let mut client = open_any()?;
    // 从accounts.json读取nickname
    let nickname = mptext_core::config::load_accounts()
        .iter()
        .find(|acc| acc.fakeid == payload.fakeid)
        .map(|acc| acc.nickname.clone())
        .unwrap_or_default();
    let result = client
        .list_articles(&payload.fakeid, &nickname, payload.begin, payload.size, None)
        .await;
    if let Err(err) = &result {
        eprintln!(
            "[list_articles] fakeid={} begin={} size={} 失败: {err}",
            payload.fakeid, payload.begin, payload.size
        );
    }
    result
        .map_err(map_err)
}

#[derive(Debug, Serialize, Clone)]
struct FetchProgress {
    fetched: usize,
    page: usize,
    status: String,
}

/// 抓取某公众号全部可用文章的核心逻辑：主页栏目 + 搜狗索引 + 合集 + 登录态全量历史
/// 多路合并去重，与 CLI `export` 保持一致。
///
/// 只靠 `list_articles` 按 begin 翻页会漏掉大半个号：后台接口对别人的号固定
/// 200013，回退到主页栏目后每栏目最多 30 篇（实测「胡锡进观察」只有 74 篇），
/// 而三路合并能到 167 篇；登录态可用时还能再往上一个量级。
///
/// `event_name` 指定进度事件通道，便于「仅抓取」与「一键抓取+下载」复用同一逻辑。
async fn fetch_all_inner(
    app: &AppHandle,
    client: &mut WechatClient,
    fakeid: &str,
    event_name: &str,
) -> Result<Vec<ArticleItem>, String> {
    // 昵称既是搜狗查询词，也是从 accounts.json 反查的结果
    let nickname = mptext_core::config::load_accounts()
        .iter()
        .find(|acc| acc.fakeid == fakeid)
        .map(|acc| acc.nickname.clone())
        .unwrap_or_default();
    fetch_by_biz(app, client, fakeid, &nickname, &[], event_name).await
}

/// 按 `__biz` 抓取某公众号的全部可用文章，不依赖后台登录态。
///
/// 「全量历史」页只知道文章链接、不知道昵称，所以昵称由调用方给
/// （空字符串时搜狗那一路自动跳过）。各通道行为与 [`fetch_all_inner`] 完全一致，
/// 只是把「从 accounts.json 查昵称」这一步留给调用方决定。
///
/// `seed_urls` 是调用方手里的额外种子（比如用户贴的那篇文章本身）：
/// 合集 id 只能从文章页反查，而主页栏目通道已全局失效（所有号都返回
/// `ret=5`），搜狗对新号又常常是 0 条 —— 此时手上的那篇就是唯一种子。
async fn fetch_by_biz(
    app: &AppHandle,
    client: &mut WechatClient,
    fakeid: &str,
    nickname: &str,
    seed_urls_extra: &[String],
    event_name: &str,
) -> Result<Vec<ArticleItem>, String> {
    let mut articles: Vec<ArticleItem> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut page: usize = 0;

    // 每合并完一路就上报一次，界面上能看到「主页栏目 74 → 搜狗 167」的推进。
    // 用宏而不是闭包：闭包会把 articles/seen 的可变借用挂到整个函数尾，
    // 后面就再也不能读 articles 去找合集种子了。
    macro_rules! merge {
        ($list:expr, $source:expr) => {{
            page += 1;
            for art in $list {
                if art.url.trim().is_empty() || art.title.trim().is_empty() {
                    continue;
                }
                if seen.insert(mptext_core::article_key(&art.url)) {
                    articles.push(art);
                }
            }
            let _ = app.emit(
                event_name,
                &FetchProgress {
                    fetched: articles.len(),
                    page,
                    status: $source.to_string(),
                },
            );
        }};
    }

    // 免登录的通道：无论有没有登录态都先铺满，登录态只做增量
    merge!(
        client
            .list_articles_homepage(fakeid, 0, u32::MAX, None)
            .await
            .unwrap_or_default(),
        "主页栏目"
    );
    // 昵称未知时搜狗那一路没有查询词，直接跳过（它按昵称搜，没有别的入口）。
    if !nickname.trim().is_empty() {
        merge!(
            client
                .list_articles_sogou_all(fakeid, nickname, 10)
                .await
                .unwrap_or_default(),
            "搜狗索引"
        );
    }

    // 合集通道：免登录、可翻到底，覆盖通常远大于前两条。合集 id 只能从
    // 已有文章页反查，一个号又常有多个互不相通的合集（实测「官察笔记」：
    // 种子只挂合集 A，A 的文章才挂合集 B，B 有 97 篇），所以要按轮展开。
    // 这段逻辑放在 core 里，CLI 与 GUI 共用一份。
    let album_seeds: Vec<String> = seed_urls_extra
        .iter()
        .cloned()
        .chain(articles.iter().map(|a| a.url.clone()))
        .collect();
    let album_articles = client
        .list_all_albums(fakeid, &album_seeds, |album_id, total| {
            let _ = app.emit(
                event_name,
                &FetchProgress {
                    fetched: total,
                    page,
                    status: format!("合集 {album_id}"),
                },
            );
        })
        .await;
    merge!(album_articles, "合集");

    // 登录态是唯一能翻到全部历史的通道，覆盖远大于上面几条；
    // 没登录时它会返回一个可读的错误，这里降级而不是让整个抓取失败。
    // 登录态不可用时再试抓包票据（同样是全量历史），与 CLI `export` 一致。
    match client.list_articles_full_history(fakeid, 0).await {
        Ok(list) => merge!(list, "登录态(全部历史)"),
        Err(_) => {
            let ticket = mptext_core::load_ticket().ok().flatten();
            let fetched_by_ticket = match &ticket {
                Some(t) => match client.list_articles_profile_ext(fakeid, t, 0).await {
                    Ok(list) => {
                        merge!(list, "抓包票据(全部历史)");
                        true
                    }
                    Err(_) => false,
                },
                None => false,
            };
            if !fetched_by_ticket {
                let _ = app.emit(
                    event_name,
                    &FetchProgress {
                        fetched: articles.len(),
                        page,
                        status: "登录态不可用（只用了免登录通道）".to_string(),
                    },
                );
            }
        }
    }

    // 最新的排前面；没有发布时间的沉到底部
    articles.sort_by_key(|a| {
        std::cmp::Reverse(a.create_time.as_ref().and_then(|v| v.as_i64()).unwrap_or(0))
    });

    let _ = app.emit(
        event_name,
        &FetchProgress {
            fetched: articles.len(),
            page,
            status: "done".to_string(),
        },
    );

    Ok(articles)
}

/// 抓取某公众号的全部历史文章（自动翻页直到无更多）。
/// 边抓边通过 fetch-articles-progress 事件上报进度，返回去重后的完整列表。
#[tauri::command]
async fn fetch_all_articles(
    app: AppHandle,
    fakeid: String,
) -> Result<Vec<ArticleItem>, String> {
    if fakeid.trim().is_empty() {
        return Err("请先选择公众号".to_string());
    }
    let mut client = open_any()?;
    fetch_all_inner(&app, &mut client, &fakeid, "fetch-articles-progress").await
}

#[tauri::command]
async fn pick_output_dir(app: AppHandle) -> Result<Option<String>, String> {
    let path = app
        .dialog()
        .file()
        .set_title("选择文章保存目录")
        .blocking_pick_folder();
    Ok(path.map(|p| p.to_string()))
}

/// 批量下载文章并实时上报进度。被「下载选中」与「自动抓取下载」共用。
async fn run_download(
    app: &AppHandle,
    client: &WechatClient,
    articles: &[ArticleItem],
    format: &str,
    output_dir: &PathBuf,
    interval_secs: f64,
) -> Result<DownloadResult, String> {
    std::fs::create_dir_all(output_dir).map_err(map_err)?;
    let total = articles.len();
    let mut saved = 0usize;
    let mut failed: Vec<FailedItem> = Vec::new();

    for (idx, article) in articles.iter().enumerate() {
        let progress = DownloadProgress {
            current: idx + 1,
            total,
            title: article.title.clone(),
            url: article.url.clone(),
            status: "downloading".to_string(),
            saved_path: None,
        };
        let _ = app.emit("download-progress", &progress);

        if idx > 0 && interval_secs > 0.0 {
            tokio::time::sleep(tokio::time::Duration::from_secs_f64(interval_secs)).await;
        }

        // 单篇失败不中断整体：记录失败明细并继续
        let result: Result<PathBuf, String> = async {
            let content = client
                .download_article(&article.url, format)
                .await
                .map_err(map_err)?;
            let filename = safe_filename(&article.title, format);
            let path = unique_path(output_dir, &filename);
            std::fs::write(&path, content).map_err(map_err)?;
            Ok(path)
        }
        .await;

        match result {
            Ok(path) => {
                saved += 1;
                let done = DownloadProgress {
                    current: idx + 1,
                    total,
                    title: article.title.clone(),
                    url: article.url.clone(),
                    status: "done".to_string(),
                    saved_path: Some(path.display().to_string()),
                };
                let _ = app.emit("download-progress", &done);
            }
            Err(err) => {
                failed.push(FailedItem {
                    title: article.title.clone(),
                    url: article.url.clone(),
                    error: err.clone(),
                });
                let fail = DownloadProgress {
                    current: idx + 1,
                    total,
                    title: article.title.clone(),
                    url: article.url.clone(),
                    status: "failed".to_string(),
                    saved_path: None,
                };
                let _ = app.emit("download-progress", &fail);
            }
        }
    }

    Ok(DownloadResult { saved, failed })
}

#[tauri::command]
async fn download_articles(
    app: AppHandle,
    payload: DownloadRequest,
) -> Result<DownloadResult, String> {
    if payload.articles.is_empty() {
        return Err("请选择要下载的文章".to_string());
    }

    let output_dir = PathBuf::from(&payload.output_dir);
    let client = open_public()?;

    run_download(
        &app,
        &client,
        &payload.articles,
        &payload.format,
        &output_dir,
        payload.interval_secs,
    )
    .await
}

#[derive(Debug, Deserialize)]
struct AutoDownloadRequest {
    fakeid: String,
    format: String,
    output_dir: String,
    interval_secs: f64,
}

#[derive(Debug, Serialize, Clone)]
struct AutoDownloadResult {
    fetched: usize,
    saved: usize,
    failed: Vec<FailedItem>,
}

/// 一键全自动：抓取该公众号全部历史文章 → 逐篇下载。
/// 抓取阶段走 `fetch-articles-progress`，列表就绪后通过 `auto-articles-ready`
/// 把完整文章列表推给前端立即渲染并全选，再进入下载阶段（`download-progress`）。
#[tauri::command]
async fn auto_download_all(
    app: AppHandle,
    payload: AutoDownloadRequest,
) -> Result<AutoDownloadResult, String> {
    if payload.fakeid.trim().is_empty() {
        return Err("请先选择公众号".to_string());
    }
    if payload.output_dir.trim().is_empty() {
        return Err("请先选择保存目录".to_string());
    }

    let output_dir = PathBuf::from(&payload.output_dir);
    let mut client = open_any()?;

    // 阶段一：抓取全部文章
    let articles =
        fetch_all_inner(&app, &mut client, &payload.fakeid, "fetch-articles-progress").await?;

    // 把抓取结果推给前端，前端立即渲染列表并默认全选（抓取失败/空条目已在抓取阶段过滤）
    let _ = app.emit("auto-articles-ready", &articles);

    if articles.is_empty() {
        return Ok(AutoDownloadResult {
            fetched: 0,
            saved: 0,
            failed: Vec::new(),
        });
    }

    // 阶段二：逐篇下载
    let result = run_download(
        &app,
        &client,
        &articles,
        &payload.format,
        &output_dir,
        payload.interval_secs,
    )
    .await?;

    Ok(AutoDownloadResult {
        fetched: articles.len(),
        saved: result.saved,
        failed: result.failed,
    })
}

// ---------------------------------------------------------------------------
// 微信客户端抓包 + 作者历史通道
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct MitmState {
    running: bool,
    port: u16,
    ca_trusted: bool,
    ca_path: String,
    captured_accounts: usize,
    message: String,
}

fn mitm_state() -> MitmState {
    let status = mptext_core::mitm::status();
    let ca_path = mptext_core::mitm::ensure_ca()
        .map(|(c, _)| c.display().to_string())
        .unwrap_or_default();
    MitmState {
        running: status.running,
        port: status.port,
        ca_trusted: mptext_core::mitm::ca_trusted(),
        ca_path,
        captured_accounts: status.captured_accounts,
        message: status.message,
    }
}

#[tauri::command]
fn mitm_status() -> MitmState {
    mitm_state()
}

/// 启动抓包代理。证书信任和系统代理是两步独立的、需要管理员授权的动作，
/// 这里不代做，只把状态返回给界面，由用户自己点。
#[tauri::command]
async fn mitm_start(port: u16) -> Result<MitmState, String> {
    mptext_core::mitm::start(port).await.map_err(map_err)?;
    Ok(mitm_state())
}

#[tauri::command]
fn mitm_stop() -> MitmState {
    mptext_core::mitm::stop();
    mitm_state()
}

#[tauri::command]
fn mitm_trust_ca() -> Result<MitmState, String> {
    mptext_core::mitm::trust_ca().map_err(map_err)?;
    Ok(mitm_state())
}

#[tauri::command]
fn mitm_set_proxy(enabled: bool, port: u16) -> Result<MitmState, String> {
    mptext_core::mitm::set_system_proxy(enabled, "127.0.0.1", port).map_err(map_err)?;
    Ok(mitm_state())
}

#[derive(Debug, Serialize)]
struct CapturedAccount {
    biz: String,
    nickname: String,
    author_id: String,
    has_ticket: bool,
    cookie_valid: bool,
    captured_at: i64,
}

#[tauri::command]
fn list_captured_accounts() -> Vec<CapturedAccount> {
    let mut list: Vec<CapturedAccount> = mptext_core::load_captures()
        .into_iter()
        .map(|(biz, cred)| {
            let has_ticket = cred.is_complete();
            let cookie_valid = cred.cookie_valid();
            CapturedAccount {
                biz,
                nickname: cred.nickname,
                author_id: cred.author_id,
                has_ticket,
                cookie_valid,
                captured_at: cred.captured_at,
            }
        })
        .collect();
    list.sort_by_key(|a| -a.captured_at);
    list
}

#[tauri::command]
fn clear_captured_accounts() -> Result<(), String> {
    mptext_core::clear_captures().map_err(map_err)
}

#[derive(Debug, Deserialize)]
struct AuthorListRequest {
    /// 该公众号任意一篇文章的链接，用来反查 author_id
    url: String,
    /// 0 = 翻到底
    max_pages: u32,
}

#[derive(Debug, Serialize)]
struct AuthorListResult {
    author_id: String,
    biz: String,
    nickname: String,
    articles: Vec<ArticleItem>,
    /// 哪些通道参与合并，界面上直接展示，避免用户以为抓住了全部。
    channels: Vec<String>,
    /// 全量历史（需要微信客户端票据）是否可用；不可用不影响免登录通道。
    ticket_used: bool,
}

/// 只拉列表、不下载。下载复用界面上已有的 download_articles。
///
/// 主体走**免登录**通道（主页栏目 + 搜狗 + 合集），与「搜索下载」页的
/// `fetch_all_articles` 同一套逻辑；微信客户端票据只是附加的一路：
/// 它需要 MITM 抓到 uin/key，而 macOS 上微信走 Clash 的 TUN 不经系统代理，
/// 拿不到票据也能正常用。
#[tauri::command]
async fn list_articles_by_author(
    app: AppHandle,
    payload: AuthorListRequest,
) -> Result<AuthorListResult, String> {
    let (author_id, biz, nickname) =
        mptext_core::resolve_article_full(&payload.url)
            .await
            .map_err(map_err)?;
    let biz = biz.ok_or_else(|| {
        "这篇不是公众号文章，或者链接已失效（页里没有 __biz）。\
         请用 mp.weixin.qq.com/s/... 形式的文章链接"
            .to_string()
    })?;
    let nickname = nickname.unwrap_or_default();

    let mut client = open_any()?;
    // 把用户贴的这篇本身也当种子：合集 id 只能从文章页反查，
    // 主页栏目通道已全局失效、搜狗对新号常为 0 条时，它就是唯一种子。
    let seeds = vec![payload.url.clone()];
    let mut articles = fetch_by_biz(
        &app,
        &mut client,
        &biz,
        &nickname,
        &seeds,
        "author-fetch-progress",
    )
    .await?;

    let mut channels = vec!["主页栏目".to_string()];
    if !nickname.is_empty() {
        channels.push("搜狗索引".to_string());
    }
    channels.push("合集".to_string());

    // 票据路线是锦上添花：抓到 uin/key 就再叠一层全量历史，没抓到不影响结果。
    let mut ticket_used = false;
    if let Some(mut cred) = mptext_core::capture_for(&biz) {
        if cred.is_complete() {
            if cred.author_id.is_empty() {
                cred.author_id = author_id.clone().unwrap_or_default();
            }
            cred.biz = biz.clone();
            let author = mptext_core::AuthorClient::new().map_err(map_err)?;
            if author.exchange_cookie(&mut cred).await.is_ok() {
                let cred_author = cred.author_id.clone();
                if let Ok(list) = author
                    .list_all(&mut cred, &cred_author, payload.max_pages)
                    .await
                {
                    let before = articles.len();
                    let mut seen: std::collections::HashSet<String> =
                        articles.iter().map(|a| mptext_core::article_key(&a.url)).collect();
                    for article in list {
                        if seen.insert(mptext_core::article_key(&article.url)) {
                            articles.push(article);
                        }
                    }
                    if articles.len() > before {
                        ticket_used = true;
                        channels.push("客户端票据(全部历史)".to_string());
                    }
                }
            }
        }
    }

    articles.sort_by_key(|a| {
        std::cmp::Reverse(a.create_time.as_ref().and_then(|v| v.as_i64()).unwrap_or(0))
    });

    Ok(AuthorListResult {
        author_id: author_id.unwrap_or_default(),
        biz,
        nickname,
        articles,
        channels,
        ticket_used,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            logging_in: Mutex::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            start_login,
            logout,
            verify_session,
            search_accounts,
            get_saved_accounts,
            clear_saved_accounts,
            toggle_favorite,
            remove_account,
            list_articles,
            fetch_all_articles,
            pick_output_dir,
            download_articles,
            auto_download_all,
            mitm_status,
            mitm_start,
            mitm_stop,
            mitm_trust_ca,
            mitm_set_proxy,
            list_captured_accounts,
            clear_captured_accounts,
            list_articles_by_author,
        ])
        .run(tauri::generate_context!())
        .expect("启动 mptext 桌面应用失败");
}
