//! `/mp/author` 作者历史通道。
//!
//! 这是目前唯一能拿到**完整历史**的通道，不再受「主页每栏目 30 篇」「合集单页 20 篇」
//! 这类硬上限约束：`author_id` 是作者维度的标识，`from_article_id` 游标可以一直翻到底。
//!
//! 三步：
//! 1. 从任意一篇公开文章页里取出 `author_id`（匿名可读，不需要登录）；
//! 2. 用微信客户端流量里抓到的 `uin` / `key` / `pass_ticket` 请求
//!    `/mp/author?action=show`，换取一枚 24 小时有效的 Cookie；
//! 3. 带着这枚 Cookie 请求 `/mp/author?action=get_articles`，
//!    用上一页最后一条的 `mid` 作为下一页的 `from_article_id`。
//!
//! 票据本身由 [`crate::mitm`] 从微信客户端流量里抓取。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use regex::Regex;
use reqwest::header::{COOKIE, REFERER, SET_COOKIE, USER_AGENT};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::ArticleItem;
use crate::config::config_dir;
use crate::util::article_key;

const MP_ORIGIN: &str = "https://mp.weixin.qq.com";
/// 桌面版微信内置浏览器的 UA。`/mp/author` 对客户端身份敏感，
/// 用移动端 UA 会被判成另一条链路，这里跟参考实现保持一致。
const DESKTOP_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";
/// 服务端接受的稳定单页条数。比 `profile_ext` 的 10 条/页高三倍。
const AUTHOR_PAGE_SIZE: u32 = 30;
/// 翻页硬上限，只用于防止异常号把循环挂死。
const AUTHOR_MAX_PAGES: u32 = 2000;
/// 两次列表请求之间的间隔。作者接口没有公开的频率说明，按保守值走。
const AUTHOR_REQUEST_INTERVAL: Duration = Duration::from_millis(600);
/// `action=show` 换来的 Cookie 服务端给 24 小时。
const COOKIE_TTL_SECS: i64 = 24 * 3600;

/// 一份可用于 `/mp/author` 的凭据，按 `__biz` 归档。
///
/// 字段来自两条来源：`uin`/`key`/`pass_ticket`/`appmsg_token`（以及客户端自己的
/// `Cookie`）由 MITM 从微信流量里抓，`cookie` 是本模块用它们换来的。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthorCredential {
    #[serde(default)]
    pub biz: String,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub uin: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub pass_ticket: String,
    #[serde(default)]
    pub appmsg_token: String,
    /// 微信客户端自己带的 Cookie，换 Cookie 时一并送上去更稳。
    #[serde(default)]
    pub client_cookie: String,
    /// `action=show` 换来的 Cookie。
    #[serde(default)]
    pub cookie: String,
    #[serde(default)]
    pub cookie_expires_at: i64,
    /// 抓取时间，Unix 秒。
    #[serde(default)]
    pub captured_at: i64,
    /// 作者维度标识，抓到文章页后回填，方便下次直接拉。
    #[serde(default)]
    pub author_id: String,
}

impl AuthorCredential {
    /// `uin` / `key` 齐全才算抓到了可用票据。
    pub fn is_complete(&self) -> bool {
        !self.uin.trim().is_empty() && !self.key.trim().is_empty()
    }

    /// 换来的 Cookie 还在有效期内。
    pub fn cookie_valid(&self) -> bool {
        !self.cookie.is_empty() && self.cookie_expires_at > now_secs()
    }

    /// 换 Cookie 的地址，参数与桌面客户端打开作者页时一致。
    fn show_url(&self) -> String {
        let mut q = vec![
            ("action", "show".to_string()),
            ("__biz", self.biz.clone()),
            ("idx", "1".to_string()),
            ("scene", "142".to_string()),
            ("rscene", "128".to_string()),
            ("uin", self.uin.clone()),
            ("key", self.key.clone()),
            ("devicetype", "UnifiedPCMac".to_string()),
            ("version", "f2640619".to_string()),
            ("lang", "zh_CN".to_string()),
            ("ascene", "1".to_string()),
            ("acctmode", "0".to_string()),
            ("pass_ticket", self.pass_ticket.clone()),
            ("countrycode", "CN".to_string()),
        ];
        if !self.author_id.is_empty() {
            q.push(("author_id", self.author_id.clone()));
        }
        let query: Vec<String> = q
            .into_iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(&v)))
            .collect();
        format!("{MP_ORIGIN}/mp/author?{}", query.join("&"))
    }

    /// 列表请求的 Referer。服务端会校验它与当前作者上下文一致。
    fn referer(&self) -> String {
        self.show_url()
    }
}

/// 当前 Unix 时间（秒）。抓包模块也要写 `captured_at`，统一放在这里。
pub(crate) fn unix_now() -> i64 {
    now_secs()
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 抓到的凭据落盘位置，与 `wxticket.json` 分开：后者是旧的 `profile_ext` 票据。
pub fn capture_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("wxcapture.json"))
}

/// 读取全部已抓凭据，按 `__biz` 索引。文件不存在或损坏都返回空表。
pub fn load_captures() -> BTreeMap<String, AuthorCredential> {
    let Ok(path) = capture_path() else {
        return BTreeMap::new();
    };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// 写入/更新一条凭据。同 `__biz` 的旧记录会被覆盖。
pub fn save_capture(cred: &AuthorCredential) -> Result<PathBuf> {
    let mut all = load_captures();
    let key = if cred.biz.is_empty() {
        "unknown".to_string()
    } else {
        cred.biz.clone()
    };
    all.insert(key, cred.clone());
    let path = capture_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&all).context("序列化抓包凭据失败")?;
    std::fs::write(&path, text)
        .with_context(|| format!("写入抓包凭据失败: {}", path.display()))?;
    Ok(path)
}

pub fn clear_captures() -> Result<()> {
    let path = capture_path()?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("删除抓包凭据失败: {}", path.display()))?;
    }
    Ok(())
}

/// 取一条可用于指定公众号的凭据：优先精确匹配 `__biz`，否则退到最近抓到的那条。
///
/// 退到最近一条是必要的：`/mp/profile_ext` 的请求里 `__biz` 经常缺失，
/// 此时凭据仍可用来换 Cookie，只是需要调用方自己带上 `author_id`。
pub fn capture_for(biz: &str) -> Option<AuthorCredential> {
    let all = load_captures();
    if !biz.is_empty() {
        if let Some(hit) = all.get(biz) {
            return Some(hit.clone());
        }
    }
    all.into_values().max_by_key(|c| c.captured_at)
}

/// 从文章页 HTML 里取作者标识。匿名可读，不需要任何登录态。
///
/// 页面里有两种写法，实测都出现过：
/// `author_id: 'ofMoI4z9cVPFGCsUbh2FevJjoris'` 和 `author_id = "ofMoI..."`。
pub fn extract_author_id(html: &str) -> Option<String> {
    let re = Regex::new(r#"author_id\s*[:=]\s*['"]([A-Za-z0-9_\-]{12,})['"]"#).ok()?;
    re.captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// 从文章页 HTML 里取 `__biz`，用于把凭据归到正确的号。
pub fn extract_biz(html: &str) -> Option<String> {
    let re = Regex::new(r#"__biz=([A-Za-z0-9+/=%]+)"#).ok()?;
    re.captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// 从文章页 HTML 里取公众号昵称。
///
/// 昵称是搜狗通道的查询词，而「全量历史」页只拿到一个文章链接，
/// 所以必须从页面本身取。页面里有三种写法，按可靠性依次尝试：
/// 分享用的 `og:title` 带「- 公众号名」后缀，不能用；这里只认昵称字段本身。
pub fn extract_nickname(html: &str) -> Option<String> {
    for pattern in [
        r#"var nickname = "([^"]+)""#,
        r#"var nickname="([^"]+)""#,
        r#"id="js_name"[^>]*>\s*([^<]+?)\s*<"#,
    ] {
        let Ok(re) = Regex::new(pattern) else {
            continue;
        };
        if let Some(name) = re
            .captures(html)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
        {
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// `/mp/author?action=get_articles` 一页的解析结果。
#[derive(Debug)]
struct AuthorPage {
    articles: Vec<ArticleItem>,
    /// 下一页的 `from_article_id`，取本页最后一条的 `mid`。
    next: Option<String>,
}

/// 作者历史通道客户端。
pub struct AuthorClient {
    http: Client,
}

impl AuthorClient {
    pub fn new() -> Result<Self> {
        let http = Client::builder()
            .user_agent(DESKTOP_UA)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .context("创建作者通道 HTTP 客户端失败")?;
        Ok(Self { http })
    }

    /// 用票据换 Cookie。成功后写回 `cred.cookie` 与过期时间。
    ///
    /// 换不到任何 `Set-Cookie` 基本等同于票据过期：`key` 大约两小时一轮，
    /// 需要回微信里再打开一次该公众号的文章，让 MITM 重新抓到。
    pub async fn exchange_cookie(&self, cred: &mut AuthorCredential) -> Result<()> {
        if !cred.is_complete() {
            bail!("票据里缺少 uin/key，请先在软件里开启抓包，然后在微信中打开一次该公众号的文章");
        }
        let mut req = self
            .http
            .get(cred.show_url())
            .header(REFERER, MP_ORIGIN)
            .header("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .header("accept-language", "zh-CN,zh;q=0.9");
        let merged = merge_cookies(&cred.client_cookie, "");
        if !merged.is_empty() {
            req = req.header(COOKIE, merged);
        }
        let response = req.send().await.context("请求 /mp/author?action=show 失败")?;
        let status = response.status();
        let cookies = collect_set_cookie(&response);
        drop(response);
        if !status.is_success() {
            bail!("换取作者 Cookie 失败：HTTP {status}");
        }
        if cookies.is_empty() {
            bail!(
                "换取作者 Cookie 失败：服务端没有下发 Cookie，票据多半已过期。\
                 请回微信再打开一次该公众号的文章后重试"
            );
        }
        cred.cookie = cookies;
        cred.cookie_expires_at = now_secs() + COOKIE_TTL_SECS;
        let _ = save_capture(cred);
        Ok(())
    }

    /// 拉一页作者文章。`from_article_id` 为空表示第一页。
    async fn fetch_page(
        &self,
        cred: &AuthorCredential,
        author_id: &str,
        from_article_id: &str,
    ) -> Result<AuthorPage> {
        let limit = AUTHOR_PAGE_SIZE.to_string();
        let mut query: Vec<(&str, String)> = vec![
            ("action", "get_articles".to_string()),
            ("author_id", author_id.to_string()),
            ("scene", "142".to_string()),
            ("limit", limit),
            ("version", "undefined".to_string()),
            ("appmsg_token", cred.appmsg_token.clone()),
            ("x5", "0".to_string()),
            ("f", "json".to_string()),
            ("user_article_role", "0".to_string()),
        ];
        if !from_article_id.is_empty() {
            query.push(("from_article_id", from_article_id.to_string()));
        }

        let mut req = self
            .http
            .get(format!("{MP_ORIGIN}/mp/author"))
            .query(&query)
            .header(REFERER, cred.referer())
            .header("accept", "*/*")
            .header("accept-language", "zh-CN,zh;q=0.9")
            .header("priority", "u=1, i")
            .header("sec-fetch-dest", "empty")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-site", "same-origin")
            .header("x-requested-with", "XMLHttpRequest");
        let merged = merge_cookies(&cred.client_cookie, &cred.cookie);
        if !merged.is_empty() {
            req = req.header(COOKIE, merged);
        }

        let response = req.send().await.context("请求作者文章列表失败")?;
        let status = response.status();
        let bytes = response.bytes().await.context("读取作者文章列表响应失败")?;
        if !status.is_success() {
            bail!("作者文章列表 HTTP {status}");
        }
        let text = String::from_utf8_lossy(&bytes);
        let text = text.trim().trim_start_matches('\u{feff}');
        let value: Value = serde_json::from_str(text).with_context(|| {
            let snippet: String = text.chars().take(180).collect();
            format!("作者接口没有返回 JSON: {snippet}")
        })?;
        parse_author_page(&value)
    }

    /// 拉全量历史。`max_pages` 传 0 表示一直翻到底。
    ///
    /// 终止条件是「本页没有文章」或「游标没有前进」，**不是**按页数 offset 判断：
    /// 作者接口的游标就是文章 `mid`，服务端重复返回同一页时必须立刻停，
    /// 否则会无限重复同一批文章。
    pub async fn list_all(
        &self,
        cred: &mut AuthorCredential,
        author_id: &str,
        max_pages: u32,
    ) -> Result<Vec<ArticleItem>> {
        if author_id.trim().is_empty() {
            bail!("没有 author_id：请提供一篇该公众号的文章链接用于反查");
        }
        if !cred.cookie_valid() {
            self.exchange_cookie(cred).await?;
        }

        let limit = if max_pages == 0 { AUTHOR_MAX_PAGES } else { max_pages };
        let mut all: Vec<ArticleItem> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut cursor = String::new();
        let mut reexchanged = false;

        for page_no in 0..limit {
            let page = match self.fetch_page(cred, author_id, &cursor).await {
                Ok(page) => page,
                Err(err) => {
                    // Cookie 过期时服务端回 ret=-3。换一次 Cookie 再重试，且只换一次，
                    // 避免在票据真的失效时把同一页反复打成风控。
                    if !reexchanged && needs_cookie_refresh(&err) {
                        reexchanged = true;
                        cred.cookie.clear();
                        self.exchange_cookie(cred).await?;
                        continue;
                    }
                    return Err(err);
                }
            };
            if page.articles.is_empty() {
                break;
            }
            for article in page.articles {
                if seen.insert(article_key(&article.url)) {
                    all.push(article);
                }
            }
            match page.next {
                Some(next) if next != cursor => cursor = next,
                // 游标没前进（或服务端没给）就说明到底了。
                _ => break,
            }
            if page_no + 1 < limit {
                tokio::time::sleep(AUTHOR_REQUEST_INTERVAL).await;
            }
        }
        Ok(all)
    }
}

/// 判断这个错误是否属于「Cookie 失效，值得换一次再试」。
///
/// 只有 `ret=-3 no session` 是。`ret=-6 unknownerror` 是服务端风控，
/// 换 Cookie 也没用，必须原样暴露给调用方。
fn needs_cookie_refresh(err: &anyhow::Error) -> bool {
    let text = err.to_string();
    text.contains("ret=-3") || text.contains("no session")
}

fn parse_author_page(value: &Value) -> Result<AuthorPage> {
    let ret = value
        .get("ret")
        .and_then(Value::as_i64)
        .or_else(|| {
            value
                .get("base_resp")
                .and_then(|b| b.get("ret"))
                .and_then(Value::as_i64)
        })
        .unwrap_or(0);
    if ret != 0 {
        let msg = value
            .get("errmsg")
            .and_then(Value::as_str)
            .or_else(|| {
                value
                    .get("base_resp")
                    .and_then(|b| b.get("err_msg"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("未知错误");
        if ret == -6 {
            bail!("作者接口返回 ret=-6（服务端风控，不是网络问题）：{msg}。请降低频率，隔几小时再试");
        }
        bail!("作者接口被拒绝 (ret={ret})：{msg}");
    }

    let raw = value
        .get("articles")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut articles = Vec::with_capacity(raw.len());
    let mut last_mid = None;
    for item in raw {
        let mid = item
            .get("mid")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        let url = item
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if mid.is_empty() || url.is_empty() {
            continue;
        }
        last_mid = Some(mid);
        let create_time = item.get("publish_time").map(|v| v.clone());
        articles.push(ArticleItem {
            title,
            url: absolutize(&url),
            create_time,
        });
    }
    Ok(AuthorPage {
        articles,
        next: last_mid,
    })
}

/// 接口偶尔给相对路径，统一补成绝对地址。
fn absolutize(url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else if url.starts_with('/') {
        format!("{MP_ORIGIN}{url}")
    } else {
        format!("{MP_ORIGIN}/{url}")
    }
}

/// 把多份 `k=v; k2=v2` 形式的 Cookie 合并，后面的覆盖前面的同名项。
pub(crate) fn merge_cookies(a: &str, b: &str) -> String {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for part in a.split(';').chain(b.split(';')) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((k, v)) = part.split_once('=') {
            let k = k.trim();
            if !k.is_empty() {
                map.insert(k.to_string(), v.trim().to_string());
            }
        }
    }
    map.into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// 收下响应里的全部 `Set-Cookie`，拼成一条请求用的 Cookie 串。
pub(crate) fn collect_set_cookie(response: &reqwest::Response) -> String {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for raw in response.headers().get_all(SET_COOKIE) {
        let Ok(text) = raw.to_str() else { continue };
        // 一条 Set-Cookie 形如 `name=value; Path=/; Domain=...; HttpOnly`，
        // 只取第一段，属性段丢掉。
        let pair = text.split(';').next().unwrap_or("").trim();
        if let Some((k, v)) = pair.split_once('=') {
            let k = k.trim();
            if !k.is_empty() {
                map.insert(k.to_string(), v.trim().to_string());
            }
        }
    }
    map.into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// 用匿名可读的公开文章页反查作者标识与 `__biz`。
///
/// 这一步不需要任何登录态：文章页 HTML 里直接带着 `author_id`。
pub async fn resolve_article(url: &str) -> Result<(Option<String>, Option<String>)> {
    let (author_id, biz, _) = resolve_article_full(url).await?;
    Ok((author_id, biz))
}

/// 反查文章页里的全部身份信息：`author_id`、`__biz`、公众号昵称。
///
/// 昵称用于搜狗通道的查询词——「全量历史」页只给一个链接，
/// 没有昵称就只能走主页栏目与合集两路，覆盖会小很多。
pub async fn resolve_article_full(
    url: &str,
) -> Result<(Option<String>, Option<String>, Option<String>)> {
    let http = Client::builder()
        .user_agent(DESKTOP_UA)
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(Duration::from_secs(30))
        .build()
        .context("创建 HTTP 客户端失败")?;
    let response = http
        .get(url)
        .header(USER_AGENT, DESKTOP_UA)
        .send()
        .await
        .with_context(|| format!("打开文章失败: {url}"))?;
    let status = response.status();
    if !status.is_success() {
        bail!("文章页 HTTP {status}");
    }
    let html = response.text().await.context("读取文章页失败")?;
    // 文章被删/被限制时页里没有 __biz，但 `s?__biz=...` 形式的链接本身就带着它。
    // 有这个就还能用「合集」通道继续反查，不必让整次抓取失败。
    let biz = extract_biz(&html).or_else(|| extract_biz_from_url(url));
    Ok((
        extract_author_id(&html),
        biz,
        extract_nickname(&html),
    ))
}

/// 从文章链接的查询串里取 `__biz`。`https://mp.weixin.qq.com/s/<短码>` 这种
/// 形式没有，返回 `None`。
pub fn extract_biz_from_url(url: &str) -> Option<String> {
    let re = Regex::new(r#"[?&]__biz=([A-Za-z0-9+/=]+)"#).ok()?;
    re.captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_author_id_in_both_styles() {
        let a = "<script>var x={author_id: 'ofMoI4z9cVPFGCsUbh2FevJjoris'}</script>";
        assert_eq!(
            extract_author_id(a).as_deref(),
            Some("ofMoI4z9cVPFGCsUbh2FevJjoris")
        );
        let b = "<script>author_id = \"ofMoI4z9cVPFGCsUbh2FevJjoris\"</script>";
        assert_eq!(
            extract_author_id(b).as_deref(),
            Some("ofMoI4z9cVPFGCsUbh2FevJjoris")
        );
        assert!(extract_author_id("<html>nothing</html>").is_none());
    }

    #[test]
    fn extracts_biz_from_url_in_html() {
        let html = "var u='https://mp.weixin.qq.com/s?__biz=Mzg4Mjk3ODcxNA==&mid=1&idx=1';";
        assert_eq!(extract_biz(html).as_deref(), Some("Mzg4Mjk3ODcxNA=="));
    }

    #[test]
    fn extracts_biz_from_query_string() {
        let url = "https://mp.weixin.qq.com/s?__biz=MzIwMDY3MTE2NQ==&mid=2247491433&idx=1";
        assert_eq!(extract_biz_from_url(url).as_deref(), Some("MzIwMDY3MTE2NQ=="));
        // 短码链接没有 __biz，交给文章页去拿。
        assert_eq!(extract_biz_from_url("https://mp.weixin.qq.com/s/aW42E-SFYKU8qRmJMGZsNQ"), None);
    }

    #[test]
    fn merges_cookies_with_later_winning() {
        let merged = merge_cookies("a=1; b=2", "b=3; c=4");
        assert_eq!(merged, "a=1; b=3; c=4");
    }

    #[test]
    fn parses_articles_and_uses_last_mid_as_cursor() {
        let value: Value = serde_json::from_str(
            r#"{"ret":0,"errmsg":"","articles":[
                {"mid":"100","title":"一","url":"https://mp.weixin.qq.com/s?a=1","publish_time":1700000000},
                {"mid":"99","title":"二","url":"https://mp.weixin.qq.com/s?a=2","publish_time":1690000000}
            ],"base_resp":{"ret":0}}"#,
        )
        .unwrap();
        let page = parse_author_page(&value).unwrap();
        assert_eq!(page.articles.len(), 2);
        assert_eq!(page.articles[0].title, "一");
        assert_eq!(page.next.as_deref(), Some("99"));
    }

    #[test]
    fn surfaces_risk_control_code() {
        let value: Value = serde_json::from_str(r#"{"ret":-6,"errmsg":"unknownerror"}"#).unwrap();
        let err = parse_author_page(&value).unwrap_err().to_string();
        assert!(err.contains("风控"), "实际: {err}");
    }

    #[test]
    fn marks_no_session_as_worth_refreshing() {
        let value: Value = serde_json::from_str(r#"{"ret":-3,"errmsg":"no session"}"#).unwrap();
        let err = parse_author_page(&value).unwrap_err();
        assert!(needs_cookie_refresh(&err));
    }
}
