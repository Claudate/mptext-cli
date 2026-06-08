use std::path::PathBuf;
use std::sync::Mutex;

use mptext_core::{
    AccountItem, ArticleItem, MptextClient, UserConfig, clear_accounts, config_path, load_accounts,
    load_config, safe_filename, save_accounts, save_config, unique_path,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    config: Mutex<UserConfig>,
}

#[derive(Debug, Serialize)]
struct SettingsInfo {
    config_path: String,
    auth_key_set: bool,
    auth_key_len: usize,
    base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SaveAuthRequest {
    auth_key: String,
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

fn build_client(config: &UserConfig) -> Result<MptextClient, String> {
    if config.auth_key.trim().is_empty() {
        return Err("请先在设置页配置 API 密钥".to_string());
    }
    MptextClient::from_config(config).map_err(map_err)
}

fn read_config(state: &State<'_, AppState>) -> Result<UserConfig, String> {
    state
        .config
        .lock()
        .map_err(|_| "配置状态锁定失败".to_string())
        .map(|c| c.clone())
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Result<SettingsInfo, String> {
    let cfg = read_config(&state)?;
    let path = config_path().map_err(map_err)?;
    Ok(SettingsInfo {
        config_path: path.display().to_string(),
        auth_key_set: !cfg.auth_key.is_empty(),
        auth_key_len: cfg.auth_key.len(),
        base_url: cfg.base_url.clone(),
    })
}

#[tauri::command]
fn save_auth_key(
    state: State<'_, AppState>,
    payload: SaveAuthRequest,
) -> Result<SettingsInfo, String> {
    if payload.auth_key.trim().is_empty() {
        return Err("API 密钥不能为空".to_string());
    }
    let mut guard = state
        .config
        .lock()
        .map_err(|_| "配置状态锁定失败".to_string())?;
    guard.auth_key = payload.auth_key.trim().to_string();
    let path = save_config(&guard).map_err(map_err)?;
    Ok(SettingsInfo {
        config_path: path.display().to_string(),
        auth_key_set: true,
        auth_key_len: guard.auth_key.len(),
        base_url: guard.base_url.clone(),
    })
}

#[tauri::command]
fn clear_auth_key(state: State<'_, AppState>) -> Result<SettingsInfo, String> {
    let mut guard = state
        .config
        .lock()
        .map_err(|_| "配置状态锁定失败".to_string())?;
    guard.auth_key = String::new();
    let path = save_config(&guard).map_err(map_err)?;
    Ok(SettingsInfo {
        config_path: path.display().to_string(),
        auth_key_set: false,
        auth_key_len: 0,
        base_url: guard.base_url.clone(),
    })
}

#[tauri::command]
async fn verify_auth(state: State<'_, AppState>) -> Result<bool, String> {
    let cfg = read_config(&state)?;
    let client = build_client(&cfg)?;
    client.verify_auth().await.map_err(map_err)
}

#[tauri::command]
async fn search_accounts(
    state: State<'_, AppState>,
    keyword: String,
) -> Result<Vec<AccountItem>, String> {
    if keyword.trim().is_empty() {
        return Err("请输入搜索关键词".to_string());
    }
    let cfg = read_config(&state)?;
    let client = build_client(&cfg)?;
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
async fn list_articles(
    state: State<'_, AppState>,
    payload: ListArticlesRequest,
) -> Result<Vec<ArticleItem>, String> {
    let cfg = read_config(&state)?;
    let client = build_client(&cfg)?;
    client
        .list_articles(&payload.fakeid, payload.begin, payload.size, None)
        .await
        .map_err(map_err)
}

#[derive(Debug, Serialize, Clone)]
struct FetchProgress {
    fetched: usize,
    page: usize,
    status: String,
}

const ARTICLE_PAGE_SIZE: u32 = 20;
/// 翻页安全上限，防止异常时死循环
const FETCH_BEGIN_LIMIT: u32 = 20_000;

/// 抓取某公众号全部历史文章的核心逻辑（自动翻页、按 url 去重、过滤空条目）。
/// `event_name` 指定进度事件通道，便于「仅抓取」与「一键抓取+下载」复用同一逻辑。
async fn fetch_all_inner(
    app: &AppHandle,
    client: &MptextClient,
    fakeid: &str,
    event_name: &str,
) -> Result<Vec<ArticleItem>, String> {
    let mut all: Vec<ArticleItem> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut begin: u32 = 0;
    let mut page: usize = 0;

    loop {
        page += 1;
        let _ = app.emit(
            event_name,
            &FetchProgress {
                fetched: all.len(),
                page,
                status: "fetching".to_string(),
            },
        );

        let batch = client
            .list_articles(fakeid, begin, ARTICLE_PAGE_SIZE, None)
            .await
            .map_err(map_err)?;
        let got = batch.len();
        if got == 0 {
            break;
        }

        // 按 url 去重，过滤标题/链接为空的无效条目
        for art in batch {
            if art.url.trim().is_empty() || art.title.trim().is_empty() {
                continue;
            }
            if seen.insert(art.url.clone()) {
                all.push(art);
            }
        }

        begin += ARTICLE_PAGE_SIZE;
        if got < ARTICLE_PAGE_SIZE as usize {
            break; // 最后一页
        }
        if begin > FETCH_BEGIN_LIMIT {
            break;
        }
        // 轻微节流，避免触发风控
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    }

    let _ = app.emit(
        event_name,
        &FetchProgress {
            fetched: all.len(),
            page,
            status: "done".to_string(),
        },
    );

    Ok(all)
}

/// 抓取某公众号的全部历史文章（自动翻页直到无更多）。
/// 边抓边通过 fetch-articles-progress 事件上报进度，返回去重后的完整列表。
#[tauri::command]
async fn fetch_all_articles(
    app: AppHandle,
    state: State<'_, AppState>,
    fakeid: String,
) -> Result<Vec<ArticleItem>, String> {
    if fakeid.trim().is_empty() {
        return Err("请先选择公众号".to_string());
    }
    let cfg = read_config(&state)?;
    let client = build_client(&cfg)?;
    fetch_all_inner(&app, &client, &fakeid, "fetch-articles-progress").await
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
    client: &MptextClient,
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
    state: State<'_, AppState>,
    payload: DownloadRequest,
) -> Result<DownloadResult, String> {
    if payload.articles.is_empty() {
        return Err("请选择要下载的文章".to_string());
    }

    let output_dir = PathBuf::from(&payload.output_dir);
    let cfg = read_config(&state)?;
    let client = build_client(&cfg)?;

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
    state: State<'_, AppState>,
    payload: AutoDownloadRequest,
) -> Result<AutoDownloadResult, String> {
    if payload.fakeid.trim().is_empty() {
        return Err("请先选择公众号".to_string());
    }
    if payload.output_dir.trim().is_empty() {
        return Err("请先选择保存目录".to_string());
    }

    let output_dir = PathBuf::from(&payload.output_dir);
    let cfg = read_config(&state)?;
    let client = build_client(&cfg)?;

    // 阶段一：抓取全部文章
    let articles = fetch_all_inner(&app, &client, &payload.fakeid, "fetch-articles-progress").await?;

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = load_config().unwrap_or_default();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            config: Mutex::new(config),
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_auth_key,
            clear_auth_key,
            verify_auth,
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
        ])
        .run(tauri::generate_context!())
        .expect("启动 mptext 桌面应用失败");
}
