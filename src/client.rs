use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;

const DEFAULT_BASE: &str = "https://down.mptext.top";

#[derive(Debug, Clone)]
pub struct MptextClient {
    http: Client,
    base_url: String,
    auth_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArticleItem {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub create_time: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccountItem {
    pub nickname: String,
    pub fakeid: String,
    #[serde(default)]
    pub alias: Option<String>,
}

impl MptextClient {
    pub fn new(base_url: impl Into<String>, auth_key: impl Into<String>) -> Result<Self> {
        let auth_key = auth_key.into();
        if auth_key.trim().is_empty() {
            bail!("API token 为空，请通过 --token 或环境变量 MPTEXT_AUTH_KEY 设置");
        }

        let http = Client::builder()
            .user_agent(format!("mptext-cli/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .context("创建 HTTP 客户端失败")?;

        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            auth_key,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    async fn get_json(&self, path: &str, query: &[(&str, String)]) -> Result<Value> {
        let response = self
            .http
            .get(self.url(path))
            .header("X-Auth-Key", &self.auth_key)
            .query(query)
            .send()
            .await
            .with_context(|| format!("请求失败: {path}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .with_context(|| format!("读取响应失败: {path}"))?;

        if !status.is_success() {
            bail!("HTTP {status} — {body}");
        }

        serde_json::from_str(&body).with_context(|| format!("解析 JSON 失败: {body}"))
    }

    fn ensure_api_ok(value: &Value) -> Result<()> {
        if let Some(code) = value.get("code").and_then(|c| c.as_i64()) {
            if code != 0 {
                let msg = value
                    .get("msg")
                    .or_else(|| value.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("未知错误");
                bail!("API 错误 (code={code}): {msg}");
            }
        }
        Ok(())
    }

    fn unwrap_data(value: Value) -> Value {
        if value.is_array() {
            return value;
        }
        if let Some(data) = value.get("data").cloned() {
            if data.is_array() {
                return data;
            }
            if let Some(list) = data.get("list").cloned() {
                return list;
            }
            if let Some(articles) = data.get("articles").cloned() {
                return articles;
            }
            return data;
        }
        value
    }

    /// 验证 API 密钥是否有效（code=0 有效，-1 过期）。
    pub async fn verify_auth(&self) -> Result<bool> {
        let value = self.get_json("/api/public/v1/authkey", &[]).await?;
        if let Some(code) = value.get("code").and_then(|c| c.as_i64()) {
            return Ok(code == 0);
        }
        Ok(true)
    }

    /// 按关键词搜索公众号。
    pub async fn search_accounts(&self, keyword: &str) -> Result<Vec<AccountItem>> {
        let value = self
            .get_json(
                "/api/public/v1/account",
                &[("keyword", keyword.to_string())],
            )
            .await?;
        Self::ensure_api_ok(&value)?;
        let data = Self::unwrap_data(value);
        parse_account_list(&data)
    }

    /// 获取公众号文章列表（支持翻页 begin/size）。
    pub async fn list_articles(
        &self,
        fakeid: &str,
        begin: u32,
        size: u32,
        keyword: Option<&str>,
    ) -> Result<Vec<ArticleItem>> {
        let mut query = vec![
            ("fakeid", fakeid.to_string()),
            ("begin", begin.to_string()),
            ("size", size.to_string()),
        ];
        if let Some(kw) = keyword {
            query.push(("keyword", kw.to_string()));
        }

        let value = self.get_json("/api/public/v1/article", &query).await?;
        Self::ensure_api_ok(&value)?;
        let data = Self::unwrap_data(value);
        parse_article_list(&data)
    }

    /// 下载单篇文章正文。
    pub async fn download_article(&self, url: &str, format: &str) -> Result<String> {
        let value = self
            .get_json(
                "/api/public/v1/download",
                &[
                    ("url", url.to_string()),
                    ("format", format.to_string()),
                ],
            )
            .await?;

        if value.is_string() {
            return Ok(value.as_str().unwrap_or_default().to_string());
        }

        Self::ensure_api_ok(&value)?;

        if let Some(content) = value.get("content").and_then(|c| c.as_str()) {
            return Ok(content.to_string());
        }
        if let Some(data) = value.get("data") {
            if let Some(content) = data.as_str() {
                return Ok(content.to_string());
            }
            if let Some(content) = data.get("content").and_then(|c| c.as_str()) {
                return Ok(content.to_string());
            }
            if let Some(markdown) = data.get("markdown").and_then(|c| c.as_str()) {
                return Ok(markdown.to_string());
            }
        }

        Ok(value.to_string())
    }

    /// 通过文章 URL 反查公众号信息。
    pub async fn account_by_url(&self, url: &str) -> Result<Value> {
        let value = self
            .get_json(
                "/api/public/v1/accountbyurl",
                &[("url", url.to_string())],
            )
            .await?;
        Self::ensure_api_ok(&value)?;
        Ok(Self::unwrap_data(value))
    }
}

fn parse_article_list(data: &Value) -> Result<Vec<ArticleItem>> {
    if let Ok(items) = serde_json::from_value::<Vec<ArticleItem>>(data.clone()) {
        return Ok(items);
    }
    if let Some(arr) = data.as_array() {
        let mut out = Vec::new();
        for item in arr {
            if let Ok(article) = serde_json::from_value::<ArticleItem>(item.clone()) {
                out.push(article);
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }
    }
    bail!("无法解析文章列表: {data}");
}

fn parse_account_list(data: &Value) -> Result<Vec<AccountItem>> {
    if let Ok(items) = serde_json::from_value::<Vec<AccountItem>>(data.clone()) {
        return Ok(items);
    }
    if let Some(arr) = data.as_array() {
        let mut out = Vec::new();
        for item in arr {
            if let Ok(account) = serde_json::from_value::<AccountItem>(item.clone()) {
                out.push(account);
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }
    }
    bail!("无法解析公众号列表: {data}");
}

pub fn default_base_url() -> &'static str {
    DEFAULT_BASE
}
