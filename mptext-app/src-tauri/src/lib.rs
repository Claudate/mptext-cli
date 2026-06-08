use std::path::PathBuf;
use std::sync::Mutex;

use mptext_core::{
    AccountItem, ArticleItem, MptextClient, UserConfig, config_path, load_config, safe_filename,
    save_config,
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
    status: String,
    saved_path: Option<String>,
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
    client
        .search_accounts(keyword.trim())
        .await
        .map_err(map_err)
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

#[tauri::command]
async fn pick_output_dir(app: AppHandle) -> Result<Option<String>, String> {
    let path = app
        .dialog()
        .file()
        .set_title("选择文章保存目录")
        .blocking_pick_folder();
    Ok(path.map(|p| p.to_string()))
}

#[tauri::command]
async fn download_articles(
    app: AppHandle,
    state: State<'_, AppState>,
    payload: DownloadRequest,
) -> Result<usize, String> {
    if payload.articles.is_empty() {
        return Err("请选择要下载的文章".to_string());
    }

    let output_dir = PathBuf::from(&payload.output_dir);
    std::fs::create_dir_all(&output_dir).map_err(map_err)?;

    let cfg = read_config(&state)?;
    let client = build_client(&cfg)?;
    let total = payload.articles.len();
    let mut saved = 0usize;

    for (idx, article) in payload.articles.iter().enumerate() {
        let progress = DownloadProgress {
            current: idx + 1,
            total,
            title: article.title.clone(),
            status: "downloading".to_string(),
            saved_path: None,
        };
        let _ = app.emit("download-progress", &progress);

        if idx > 0 && payload.interval_secs > 0.0 {
            tokio::time::sleep(tokio::time::Duration::from_secs_f64(
                payload.interval_secs,
            ))
            .await;
        }

        let content = client
            .download_article(&article.url, &payload.format)
            .await
            .map_err(map_err)?;
        let filename = safe_filename(&article.title, &payload.format);
        let path = output_dir.join(filename);
        std::fs::write(&path, content).map_err(map_err)?;
        saved += 1;

        let done = DownloadProgress {
            current: idx + 1,
            total,
            title: article.title.clone(),
            status: "done".to_string(),
            saved_path: Some(path.display().to_string()),
        };
        let _ = app.emit("download-progress", &done);
    }

    Ok(saved)
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
            verify_auth,
            search_accounts,
            list_articles,
            pick_output_dir,
            download_articles,
        ])
        .run(tauri::generate_context!())
        .expect("启动 mptext 桌面应用失败");
}
