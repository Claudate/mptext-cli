#! 直接对接微信公众平台后台（mp.weixin.qq.com）。
//!
//! down.mptext.top 的公开接口只是把扫码得到的后台 Cookie 和 token 存在它自己的服务器上再转发。
//! 这里改成把同一套会话存在本机：扫码登录、按 fakeid 拉发布列表、再抓文章页正文。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use reqwest::header::{CONTENT_TYPE, COOKIE, LOCATION, REFERER, SET_COOKIE, USER_AGENT, ACCEPT_ENCODING};
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::{AccountItem, ArticleItem};
use crate::config::config_dir;
use crate::util::article_key;
use scraper::{Html, Selector};
use urlencoding::encode;
use regex::Regex;

const MP_ORIGIN: &str = "https://mp.weixin.qq.com";
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";
const SOGOU_MOBILE_UA: &str = "Mozilla/5.0 (Linux; Android 13; SM-G991B) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/112.0.0.0 Mobile Safari/537.36";
/// 搜狗移动搜索一页的条数，只用于测试构造整页 HTML。
#[cfg(test)]
pub(crate) const SOGOU_PAGE_SIZE: usize = 10;

/// 公众号主页公开接口（/mp/homepage）最多探测的栏目数上限，仅防止异常号无限探测。
/// 实测栏目数因号而异：「良辰美」有 15 个（hid 1-15），写死 8 会漏掉一半以上文章。
const HOMEPAGE_MAX_HID: u64 = 40;
/// 连续这么多个空栏目就认为探测到底。栏目编号实测是连续的，末尾 ret=5。
const HOMEPAGE_MAX_EMPTY_HIDS: u32 = 3;
/// 每个栏目最多翻页次数，防止对异常大号无限翻。
/// 实测每栏目硬上限 30 篇（`begin=30` 起返回空），与 `count` 无关，所以 2 页就够。
const HOMEPAGE_MAX_PAGES_PER_HID: u32 = 4;
/// 主页接口一页的条数。实测 `count` 超过 30 会被截到 30。
const HOMEPAGE_PAGE_SIZE: u32 = 30;
/// 微信内置浏览器的 UA，只在抓票据那条路上用。
///
/// 这里以前写着「`profile_ext` 只认客户端身份，桌面 UA 会拿到空页」——**这是错的**。
/// 实测桌面 UA 打 `getmsg` 会正常返回 `{"ret":-3,"errmsg":"no session"}`，
/// 说明接口对任何 UA 都开放，真正的门槛是**登录 cookie**，不是客户端身份。
const WX_CLIENT_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.40(0x18002834) NetType/WIFI Language/zh_CN";
/// 主页接口没有频率保护，两次请求之间慢一点稳。
const HOMEPAGE_REQUEST_INTERVAL: Duration = Duration::from_millis(800);

/// 搜狗移动结果页里的一条，链接仍是 `/link?url=`，还没换成文章地址。
pub(crate) struct SogouArticleRef {
    pub title: String,
    pub link: String,
    pub source_name: String,
    pub last_modified: Option<u64>,
}

/// `articles` 已按 nickname 滤过。`total_rows` 是本页所有 `sogou_vr_` 条目数（不按昵称过滤）。
/// 翻页只看 `total_rows`：搜狗按相关度混排，某页没命中这个号不等于到底了。
pub(crate) struct SogouMobilePage {
    pub articles: Vec<SogouArticleRef>,
    pub total_rows: usize,
}

/// `/mp/homepage?action=appmsg_list` 一页的解析结果。
pub(crate) struct HomepagePage {
    pub articles: Vec<ArticleItem>,
    pub has_more: bool,
}

/// `/mp/appmsgalbum?action=getalbum` 一页的解析结果。
/// `continue_flag` 为 1 表示还有下一页；`cursor` 是"下一页从哪开始"的游标。
pub(crate) struct AlbumPage {
    pub articles: Vec<ArticleItem>,
    pub has_more: bool,
    pub cursor: Option<(String, String)>,
}

/// 合集通道一页的条数。实测 20 是服务端接受的稳定值。
const ALBUM_PAGE_SIZE: u32 = 20;
/// 单个合集的翻页上限。实测最大的合集 100 篇（5 页），留出余量。
const ALBUM_MAX_PAGES: u32 = 60;
/// 合集反查最多扫多少篇种子文章。一页文章页会挂多个合集，不同文章挂的子集不同，
/// 只看十几篇会漏；实测「官察笔记」第 4 个合集要到第 125 篇才出现。
const ALBUM_SEED_LIMIT: usize = 300;
/// 合集反查的轮次上限：合集里的文章可能挂着另一个合集，按轮展开直到没有新合集。
const ALBUM_DISCOVER_ROUNDS: usize = 4;
/// 合集通道两次请求之间的间隔，和主页接口一样慢一点更稳。
const ALBUM_REQUEST_INTERVAL: Duration = Duration::from_millis(600);

/// 微信客户端票据：由 `tools/wxmitm_addon.py` 从微信客户端流量里抓下来存到本机。
///
/// `key` 约两小时过期，抓包后要尽快用；`uin` / `key` 绑定当时登录的微信账号，
/// 换号或重新登录后必须重抓。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WxTicket {
    #[serde(default)]
    pub captured_at: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub cookie: String,
}

/// 票据文件路径，与抓包插件 `tools/wxmitm_addon.py` 写入的位置一致。
pub fn ticket_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("wxticket.json"))
}

/// 读取票据。文件不存在或损坏都返回 `Ok(None)`，让调用方给出「先去抓包」的提示。
pub fn load_ticket() -> Result<Option<WxTicket>> {
    let path = ticket_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("读取票据失败: {}", path.display()))?;
    Ok(serde_json::from_str(&text).ok())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MpSession {
    #[serde(default)]
    pub cookies: BTreeMap<String, String>,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub nickname: String,
}

pub struct WechatClient {
    http: Client,
    session: MpSession,
    /// 搜狗移动搜索下发的 Cookie（SNUID / SUID / ABTEST）。不进公众号会话文件：
    /// `/link` 不带 SNUID 会 302 到验证码页，和微信后台登录无关。
    sogou_cookies: BTreeMap<String, String>,
}

impl WechatClient {
    pub fn new(session: MpSession) -> Result<Self> {
        let http = Client::builder()
            .user_agent(UA)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .context("创建 HTTP 客户端失败")?;
        Ok(Self {
            http,
            session,
            sogou_cookies: BTreeMap::new(),
        })
    }

    /// 读取本机已保存的扫码会话。没有登录过时返回错误，提示先执行 login。
    pub fn from_saved_session() -> Result<Self> {
        let session = load_session()?;
        if session.token.trim().is_empty() || session.cookies.is_empty() {
            bail!("尚未登录微信公众平台。请先运行: mptext login");
        }
        Self::new(session)
    }

    pub fn session(&self) -> &MpSession {
        &self.session
    }

    /// 扫码登录。`on_qr` 在拿到二维码图片后调用一次（参数是保存路径）。
    /// `on_status` 在扫码状态变化时调用。成功后会话写入本机配置目录。
    pub async fn login_qr<F, G>(&mut self, mut on_qr: F, mut on_status: G) -> Result<MpSession>
    where
        F: FnMut(&std::path::Path),
        G: FnMut(&str),
    {
        self.session = MpSession::default();
        let sid = format!(
            "{}{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.subsec_nanos() % 100)
                .unwrap_or(0)
        );

        let started = self
            .mp_form(
                Method::POST,
                "/cgi-bin/bizlogin",
                &[("action", "startlogin")],
                &[
                    ("userlang", "zh_CN"),
                    ("redirect_url", ""),
                    ("login_type", "3"),
                    ("sessionid", &sid),
                    ("token", ""),
                    ("lang", "zh_CN"),
                    ("f", "json"),
                    ("ajax", "1"),
                ],
            )
            .await?;
        ensure_ret(&started, "发起登录")?;

        let qr_path = config_dir()?.join("login-qr.png");
        let qr = self
            .mp_bytes(
                Method::GET,
                "/cgi-bin/scanloginqrcode",
                &[
                    ("action", "getqrcode"),
                    ("random", &millis_now().to_string()),
                ],
                &[],
            )
            .await?;
        if let Some(parent) = qr_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&qr_path, &qr).with_context(|| format!("写入二维码失败: {}", qr_path.display()))?;
        on_qr(&qr_path);
        on_status("请用微信扫描二维码，并在手机上确认登录公众号后台");

        let _last_status = -1i64;
        let deadline = std::time::Instant::now() + Duration::from_secs(180);
        loop {
            if std::time::Instant::now() > deadline {
                bail!("等待扫码超时（3 分钟）。请重新运行 mptext login");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            let ask = self
                .mp_get(
                    "/cgi-bin/scanloginqrcode",
                    &[
                        ("action", "ask"),
                        ("token", ""),
                        ("lang", "zh_CN"),
                        ("f", "json"),
                        ("ajax", "1"),
                    ],
                )
                .await?;
            ensure_ret(&ask, "查询扫码状态")?;
            let status = ask.get("status").and_then(|v| v.as_i64()).unwrap_or(-1);
            if status == 0 || status == 4 || status == 5 || status == 6 {
                continue;
            }
            if status == 2 || status == 3 {
                let qr = self
                    .mp_bytes(
                        Method::GET,
                        "/cgi-bin/scanloginqrcode",
                        &[
                            ("action", "getqrcode"),
                            ("random", &millis_now().to_string()),
                        ],
                        &[],
                    )
                    .await?;
                std::fs::write(&qr_path, &qr)?;
                on_qr(&qr_path);
            }
            if status == 1 {
                break;
            } else {
                bail!("未知扫码状态 {status}: {ask}");
            }
        }

        let logged = self
            .mp_form(
                Method::POST,
                "/cgi-bin/bizlogin",
                &[("action", "login")],
                &[
                    ("userlang", "zh_CN"),
                    ("redirect_url", ""),
                    ("cookie_forbidden", "0"),
                    ("cookie_cleaned", "0"),
                    ("plugin_used", "0"),
                    ("login_type", "3"),
                    ("token", ""),
                    ("lang", "zh_CN"),
                    ("f", "json"),
                    ("ajax", "1"),
                ],
            )
            .await?;
        ensure_ret(&logged, "确认登录")?;
        let redirect = logged
            .get("redirect_url")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let token = token_from_url(redirect).context("登录响应里没有 token")?;
        // 跟一次跳转，把后台首页下发的 Cookie 也收下。
        let home = if redirect.starts_with("http") {
            redirect.to_string()
        } else {
            format!("{MP_ORIGIN}{}", redirect)
        };
        let _ = self.raw_get_follow(&home).await;
        self.session.token = token;
        self.session.nickname = self.fetch_nickname().await.unwrap_or_default();
        save_session(&self.session)?;
        Ok(self.session.clone())
    }

    /// 用一次轻量搜索确认后台会话还在。ret=200003 表示要重新扫码。
    pub async fn verify(&mut self) -> Result<bool> {
        let value = self
            .mp_get(
                "/cgi-bin/searchbiz",
                &[
                    ("action", "search_biz"),
                    ("begin", "0"),
                    ("count", "1"),
                    ("query", "微信"),
                    ("token", &self.session.token.clone()),
                    ("lang", "zh_CN"),
                    ("f", "json"),
                    ("ajax", "1"),
                ],
            )
            .await?;
        match ret_of(&value) {
            Some(0) => Ok(true),
            Some(200003) => Ok(false),
            Some(code) => bail!("会话校验失败 (ret={code}): {}", err_msg(&value)),
            None => bail!("会话校验返回无法识别: {value}"),
        }
    }

    pub async fn search_accounts(&mut self, keyword: &str) -> Result<Vec<AccountItem>> {
        let token = self.session.token.clone();
        let value = self
            .mp_get(
                "/cgi-bin/searchbiz",
                &[
                    ("action", "search_biz"),
                    ("begin", "0"),
                    ("count", "5"),
                    ("query", keyword),
                    ("token", &token),
                    ("lang", "zh_CN"),
                    ("f", "json"),
                    ("ajax", "1"),
                ],
            )
            .await?;
        ensure_ret(&value, "搜索公众号")?;
        parse_account_list(&value)
    }

    /// 拉取一页已发布文章。`begin` 是偏移，`size` 最大 20（微信后台的上限）。
    pub async fn list_articles(
        &mut self,
        fakeid: &str,
        nickname: &str,
        begin: u32,
        size: u32,
        keyword: Option<&str>,
    ) -> Result<Vec<ArticleItem>> {
        let token = self.session.token.clone();
        let size = size.clamp(1, 20).to_string();
        let begin = begin.to_string();
        let keyword = keyword.unwrap_or("");
        let searching = !keyword.is_empty();
        let value = self
            .mp_get(
                "/cgi-bin/appmsgpublish",
                &[
                    ("sub", if searching { "search" } else { "list" }),
                    ("search_field", if searching { "7" } else { "null" }),
                    ("begin", &begin),
                    ("count", &size),
                    ("query", keyword),
                    ("fakeid", fakeid),
                    ("type", "101_1"),
                    ("free_publish_type", "1"),
                    ("sub_action", "list_ex"),
                    ("token", &token),
                    ("lang", "zh_CN"),
                    ("f", "json"),
                    ("ajax", "1"),
                ],
            )
            .await?;

        match ret_of(&value) {
            Some(0) => {}
            Some(200013) => {
                // 2026-10-19 实测：200013 不是频控、不是账号被封、也不是出口 IP 的问题
                // （换上海电信家宽仍是 200013）。用同一份 session 查**自己名下**的号
                // （fakeid = cookie 里的 bizuin）时 type=101_1 正常返回 total_count 并
                // 逐页翻到底（83 篇 = 20/20/20/20/3），查**别人的号**才固定 200013。
                // 也就是说这是后台的权限模型：登录会话只能发布列表自己运营的公众号。
                // 因此这里回退到免登录的公开通道，两者都不需要票据。
                // searchbiz 返回的 fakeid 和文章链接里的 __biz 是同一个值。
                let begin_num: u32 = begin.parse().unwrap_or(0);
                let size_num: u32 = size.parse().unwrap_or(20);
                let kw = if searching { Some(keyword) } else { None };
                match self
                    .list_articles_homepage(fakeid, begin_num, size_num, kw)
                    .await
                {
                    // 主页接口是回退通道之一：没拿到（空或出错）都不挡路，交给搜狗再试。
                    Ok(articles) if !articles.is_empty() => return Ok(articles),
                    _ => {}
                }
                // Calculate page number for Sogou: assuming 10 items per page
                let page_num = (begin_num / 10) + 1;
                return self
                    .list_articles_sogou(fakeid, nickname, page_num)
                    .await
                    .map(|(articles, _)| articles);
            }
            Some(200003) => {
                bail!("获取文章列表失败：登录已失效，请重新运行 mptext login");
            }
            Some(code) => {
                bail!("获取文章列表失败 (ret={code}): {}", err_msg(&value));
            }
            None => {
                bail!("获取文章列表失败：返回里没有 base_resp ({value})");
            }
        }

        parse_publish_page(&value)
    }

    /// 按昵称翻搜狗移动搜索，直到没有更多结果或到页数上限。
    ///
    /// 搜狗只索引到每个号很浅的一层（实测「占豪」翻 10 页只对上 2 条），
    /// 所以它只是兜底：`/mp/homepage` 完全没有栏目的号（占豪、卢克文工作室、
    /// 熊猫反诈、我们的东莞）至少还能从这里拿到几条。
    pub async fn list_articles_sogou_all(
        &mut self,
        fakeid: &str,
        nickname: &str,
        max_pages: u32,
    ) -> Result<Vec<ArticleItem>> {
        let mut seen = std::collections::HashSet::new();
        let mut all = Vec::new();
        for page in 1..=max_pages.max(1) {
            let (batch, is_last_page) = match self.list_articles_sogou(fakeid, nickname, page).await {
                // 搜狗偶发验证码会直接报错，已经拿到的结果仍然有价值，不再往上抛。
                Ok(batch) => batch,
                Err(_) => break,
            };
            for article in batch {
                if seen.insert(article_key(&article.url)) {
                    all.push(article);
                }
            }
            if is_last_page {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
        Ok(all)
    }

    /// 使用搜狗移动搜索作为后备方案获取文章列表。
    /// 页面解析和 JS 拼 URL 都是纯函数；这里只负责发请求。
    async fn list_articles_sogou(&mut self, fakeid: &str, nickname: &str, page_num: u32) -> Result<(Vec<ArticleItem>, bool)> {
        let encoded_query = encode(nickname);
        let sogou_url = format!(
            "https://weixin.sogou.com/weixinwap?type=2&query={}&page={}",
            encoded_query, page_num
        );

        let response = self.sogou_get(&sogou_url, None).await?;
        let html = response.text;

        let page = parse_sogou_mobile_page(&html, nickname);
        // 整页没有任何结果条目才是真到底。「本页没命中这个号」不算——
        // 搜狗按相关度混排，占豪自己的文章要到第 4、5 页才出现。
        let is_last = page.total_rows == 0;
        let mut articles = Vec::new();
        for hit in page.articles {
            let link_url = if hit.link.starts_with("http") {
                hit.link
            } else {
                format!("https://weixin.sogou.com{}", hit.link)
            };
            let js_response = self.sogou_get(&link_url, Some(&sogou_url)).await?;
            let article_url = extract_article_url_from_js(&js_response.text).with_context(|| {
                format!(
                    "解析「{}」的搜狗跳转失败（HTTP {}）",
                    hit.source_name, js_response.status
                )
            })?;
            if let Some(biz) = query_param(&article_url, "__biz") {
                if biz != fakeid {
                    continue;
                }
            }
            articles.push(ArticleItem {
                title: hit.title,
                url: article_url,
                create_time: hit.last_modified.map(Value::from),
            });
        }
        Ok((articles, is_last))
    }

    /// 公众号主页公开接口：免登录，先按 hid 探测栏目，再跟随 has_more 翻页合并。
    /// searchbiz 返回的 fakeid 和文章链接里的 __biz 是同一个值，直接当 __biz 用。
    /// 主页栏目之外的历史文章拿不到（那是后台接口的独占数据），这里是 200013 下最好的公开来源。
    pub async fn list_articles_homepage(
        &mut self,
        biz: &str,
        begin: u32,
        size: u32,
        keyword: Option<&str>,
    ) -> Result<Vec<ArticleItem>> {
        let mut seen = std::collections::HashSet::new();
        let mut all = Vec::new();
        let mut empty_hids = 0u32;
        for hid in 1..=HOMEPAGE_MAX_HID {
            let mut offset = 0u32;
            let mut hid_articles = 0usize;
            for _ in 0..HOMEPAGE_MAX_PAGES_PER_HID {
                let value = self.homepage_page(biz, hid, offset).await?;
                // ret 非 0（hid 不存在时实测是 5）说明这个栏目没有数据，换下一个 hid。
                let Some(page) = parse_homepage_json(&value)? else {
                    break;
                };
                hid_articles += page.articles.len();
                let count = page.articles.len() as u32;
                for article in page.articles {
                    if seen.insert(article_key(&article.url)) {
                        all.push(article);
                    }
                }
                if !page.has_more || count == 0 {
                    break;
                }
                offset += count;
                tokio::time::sleep(HOMEPAGE_REQUEST_INTERVAL).await;
            }
            if hid_articles == 0 {
                empty_hids += 1;
                if empty_hids >= HOMEPAGE_MAX_EMPTY_HIDS {
                    break;
                }
            } else {
                empty_hids = 0;
            }
        }
        Ok(finish_homepage_articles(all, begin, size, keyword))
    }

    /// `appmsgalbum` 合集通道：免登录、可翻页翻到底，是免票据通道里覆盖最大的一个。
    ///
    /// 实测「阿昆的科研日常」两个合集分别 48 篇 / 100 篇，而主页栏目对该号返回
    /// `ret=5`（没有栏目）、搜狗只索引到 7 篇。合集列表是作者在后台自己建的，
    /// 只有被收进合集的文章才在，但它没有任何条数上限，也不受后台权限限制。
    pub async fn list_articles_album(&mut self, biz: &str, album_id: &str) -> Result<Vec<ArticleItem>> {
        let mut seen = std::collections::HashSet::new();
        let mut all = Vec::new();
        let mut cursor: Option<(String, String)> = None;
        for _ in 0..ALBUM_MAX_PAGES {
            let page = self.album_page(biz, album_id, cursor.as_ref()).await?;
            if page.articles.is_empty() {
                break;
            }
            for article in page.articles {
                if seen.insert(article_key(&article.url)) {
                    all.push(article);
                }
            }
            if !page.has_more {
                break;
            }
            // 游标不推进就说明服务端不再给新数据，别原地死循环。
            if page.cursor.is_none() || page.cursor == cursor {
                break;
            }
            cursor = page.cursor;
            tokio::time::sleep(ALBUM_REQUEST_INTERVAL).await;
        }
        Ok(all)
    }

    /// POST 一页合集数据。`cursor` 是上一页最后一条的 `(msgid, itemidx)`。
    async fn album_page(
        &self,
        biz: &str,
        album_id: &str,
        cursor: Option<&(String, String)>,
    ) -> Result<AlbumPage> {
        let count = ALBUM_PAGE_SIZE.to_string();
        let mut query: Vec<(&str, &str)> = vec![
            ("__biz", biz),
            ("action", "getalbum"),
            ("album_id", album_id),
            ("count", count.as_str()),
            ("f", "json"),
        ];
        let (msgid, itemidx);
        if let Some((m, i)) = cursor {
            msgid = m.clone();
            itemidx = i.clone();
            query.push(("begin_msgid", msgid.as_str()));
            query.push(("begin_itemidx", itemidx.as_str()));
        }
        let response = self
            .http
            .get(format!("{MP_ORIGIN}/mp/appmsgalbum"))
            .query(&query)
            .header(REFERER, MP_ORIGIN)
            .header(USER_AGENT, WX_CLIENT_UA)
            .send()
            .await
            .with_context(|| format!("请求合集失败: album_id={album_id}"))?;
        let status = response.status();
        let bytes = response.bytes().await.context("读取合集接口响应失败")?;
        if !status.is_success() {
            bail!("合集接口 HTTP {status}");
        }
        parse_album_json(&parse_mp_json(&bytes)?)
    }

    /// 从一个已存在的文章页里发现合集 id。
    ///
    /// 合集 id 没有任何按公众号列举的接口，只能从文章页的
    /// `appmsgalbuminfo` 里反查；一篇文章最多挂一个合集。
    pub async fn discover_album_ids(&self, article_url: &str) -> Vec<String> {
        let Ok(html) = self.public_html(article_url).await else {
            return Vec::new();
        };
        extract_album_ids(&html)
    }

    /// 把某个号的所有合集都翻完，返回去重后的文章。
    ///
    /// 合集 id 只能从文章页反查，而一个号常有多个互不相通的合集：
    /// 实测「官察笔记」种子文章只挂合集 A（61 篇），A 里的文章才挂合集 B（97 篇），
    /// 只看种子那一轮会漏掉整个 B。所以按轮展开——每拉完一批合集，
    /// 就用新拿到的文章继续反查，直到没有新合集为止。
    ///
    /// `seed_urls` 是第一批种子（通常是主页栏目/搜狗已拿到的文章，
    /// 外加用户手动贴的那一篇）。`progress` 在每拉完一个合集后回调。
    pub async fn list_all_albums(
        &mut self,
        biz: &str,
        seed_urls: &[String],
        mut progress: impl FnMut(&str, usize),
    ) -> Vec<ArticleItem> {
        let mut album_ids: Vec<String> = Vec::new();
        let mut seen_urls: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut articles: Vec<ArticleItem> = Vec::new();
        let mut scanned: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut pending: Vec<String> = seed_urls.to_vec();

        for _round in 0..ALBUM_DISCOVER_ROUNDS {
            // 反查这一批种子，凑出新发现的合集。
            let mut new_albums = 0usize;
            for seed in &pending {
                if !scanned.insert(seed.clone()) {
                    continue;
                }
                let before = album_ids.len();
                for id in self.discover_album_ids(seed).await {
                    if !album_ids.contains(&id) {
                        album_ids.push(id);
                    }
                }
                if album_ids.len() > before {
                    new_albums += album_ids.len() - before;
                }
                // 只按总量设上限：早期版本用「连续 N 篇没新发现就收手」，
                // 实测「官察笔记」第 4 个合集要扫到第 125 篇才出现，会被提前截断
                // （218 篇只拿到 158 篇）。相邻文章确实高度重复，但重复一次不能
                // 说明后面没有新合集，所以这里只受 ALBUM_SEED_LIMIT 约束。
                if scanned.len() >= ALBUM_SEED_LIMIT {
                    break;
                }
            }

            if new_albums == 0 {
                break;
            }
            // 本轮的全部合集都拉一遍；新拿到的文章作为下一轮的种子。
            let before_len = articles.len();
            for album_id in album_ids.clone() {
                let list = self.list_articles_album(biz, &album_id).await.unwrap_or_default();
                for article in list {
                    if seen_urls.insert(article_key(&article.url)) {
                        articles.push(article);
                    }
                }
                progress(&album_id, articles.len());
            }
            pending = articles
                .iter()
                .skip(before_len)
                .map(|a| a.url.clone())
                .collect();
            if pending.is_empty() {
                break;
            }
        }
        articles
    }

    /// 用后台登录会话换 `profile_ext` 的 `key` / `uin`，再翻完整历史。
    ///
    /// 这是拿到**全部历史**的通道：后台 `appmsgpublish` / `appmsg?action=list_ex` 已被
    /// 200013 `freq control` 挡住，公开的 `/mp/homepage` 每栏目硬上限 30 篇。
    ///
    /// 关键事实（实测）：`/mp/profile_ext` 不是「微信客户端专属」，桌面 UA 也照常返回。
    /// 不带登录态时它回 `no session`（`cookie_count: 0`），带上就是完整列表。
    /// 登录后 `/mp/profile_ext?action=home` 的页面里就嵌着 `key` / `uin` /
    /// `pass_ticket`，所以不用抓包、不用装证书。
    ///
    /// `max_pages` 每页 10 条，传 0 表示一直翻到 `can_msg_continue` 为 0。
    pub async fn list_articles_full_history(
        &mut self,
        biz: &str,
        max_pages: u32,
    ) -> Result<Vec<ArticleItem>> {
        let ticket = self.profile_ext_ticket(biz).await?;
        self.list_articles_profile_ext(biz, &ticket, max_pages).await
    }

    /// 拿登录会话打开 `/mp/profile_ext?action=home`，从页面里取出翻页凭据。
    ///
    /// 页面里凭据有三种写法（实测都出现过）：`var key = "..."`、URL 查询串里的
    /// `key=`、以及 `pass_ticket=`。取不到 `key` 就说明这台机器没登录过该号的历史页，
    /// 或者登录态已过期。
    pub async fn profile_ext_ticket(&mut self, biz: &str) -> Result<WxTicket> {
        let url = format!(
            "{MP_ORIGIN}/mp/profile_ext?action=home&__biz={biz}&scene=124&is_ok=1"
        );
        let bytes = self.raw_get_follow(&url).await?;
        let html = String::from_utf8_lossy(&bytes);

        // 登录态失效时这里会返回「Open link in Weixin.」的提示页，长度约 2 KB。
        if html.contains("Open link in Weixin") || html.contains("请在微信客户端打开链接") {
            bail!(
                "拿不到历史列表：这台机器上没有该公众号的登录态。请运行 mptext login 扫码后重试"
            );
        }

        let key = js_string(&html, "key")
            .or_else(|| query_param(&html, "key"))
            .unwrap_or_default();
        let uin = js_string(&html, "uin")
            .or_else(|| query_param(&html, "uin"))
            .unwrap_or_default();
        if key.is_empty() || uin.is_empty() {
            bail!("历史列表页面里没有 key/uin，登录态可能已过期，请重新扫码");
        }
        let pass_ticket = js_string(&html, "pass_ticket")
            .or_else(|| query_param(&html, "pass_ticket"))
            .unwrap_or_default();
        let appmsg_token = js_string(&html, "appmsg_token").unwrap_or_default();
        let mut ticket_url = format!("{url}&key={key}&uin={uin}");
        if !pass_ticket.is_empty() {
            ticket_url.push_str(&format!("&pass_ticket={pass_ticket}"));
        }
        if !appmsg_token.is_empty() {
            ticket_url.push_str(&format!("&appmsg_token={appmsg_token}"));
        }
        Ok(WxTicket {
            captured_at: String::new(),
            url: ticket_url,
            cookie: self
                .session
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; "),
        })
    }

    /// 用微信客户端票据翻历史消息（`/mp/profile_ext?action=getmsg`）。
    ///
    /// 票据由 `tools/wxmitm_addon.py` 从微信流量里抓，`key` 约两小时过期。
    /// 正常情况下用 [`list_articles_full_history`] 就够了，它会自动从登录会话取凭据；
    /// 这个方法留给已经抓到票据的场景。
    /// `max_pages` 每页 10 条，传 0 表示一直翻到 `can_msg_continue` 为 0。
    pub async fn list_articles_profile_ext(
        &self,
        biz: &str,
        ticket: &WxTicket,
        max_pages: u32,
    ) -> Result<Vec<ArticleItem>> {
        let uin = query_param(&ticket.url, "uin").unwrap_or_default();
        let key = query_param(&ticket.url, "key").unwrap_or_default();
        let pass_ticket = query_param(&ticket.url, "pass_ticket").unwrap_or_default();
        if key.is_empty() || uin.is_empty() {
            bail!("票据里没有 uin 或 key，请在微信里重新打开一次该公众号的历史消息");
        }
        let appmsg_token = query_param(&ticket.url, "appmsg_token").unwrap_or_default();

        let mut seen = std::collections::HashSet::new();
        let mut all = Vec::new();
        let mut offset = 0u32;
        let limit = if max_pages == 0 { u32::MAX } else { max_pages };
        for _ in 0..limit {
            let offset_s = offset.to_string();
            let mut query = vec![
                ("action", "getmsg"),
                ("__biz", biz),
                ("f", "json"),
                ("offset", offset_s.as_str()),
                ("count", "10"),
                ("is_ok", "1"),
                ("scene", "124"),
                ("uin", uin.as_str()),
                ("key", key.as_str()),
                ("pass_ticket", pass_ticket.as_str()),
                ("wxtoken", ""),
                ("x5", "0"),
                ("lang", "zh_CN"),
            ];
            if !appmsg_token.is_empty() {
                query.push(("appmsg_token", appmsg_token.as_str()));
            }
            let mut req = self
                .http
                .get(format!("{MP_ORIGIN}/mp/profile_ext"))
                .query(&query)
                .header(USER_AGENT, WX_CLIENT_UA)
                .header(REFERER, MP_ORIGIN);
            if !ticket.cookie.is_empty() {
                req = req.header(COOKIE, ticket.cookie.as_str());
            }
            let response = req.send().await.context("请求历史消息失败")?;
            let status = response.status();
            let bytes = response.bytes().await.context("读取历史消息响应失败")?;
            if !status.is_success() {
                bail!("历史消息接口 HTTP {status}（票据多半已过期，重新抓一次）");
            }
            let value = parse_mp_json(&bytes)?;
            match ret_of(&value) {
                Some(0) => {}
                Some(code) => bail!("历史消息被拒绝 (ret={code})：{}", err_msg(&value)),
                None => bail!("历史消息返回里没有 base_resp"),
            }
            let (articles, next_offset, more) = parse_profile_ext_page(&value)?;
            let count = articles.len();
            for article in articles {
                if seen.insert(article_key(&article.url)) {
                    all.push(article);
                }
            }
            if !more || count == 0 || next_offset <= offset {
                break;
            }
            offset = next_offset;
            tokio::time::sleep(HOMEPAGE_REQUEST_INTERVAL).await;
        }
        Ok(all)
    }

    /// POST 一页主页数据。公开接口不带后台会话，它的 Cookie 也不收进会话文件。
    async fn homepage_page(&self, biz: &str, hid: u64, begin: u32) -> Result<Value> {
        let hid_s = hid.to_string();
        let begin_s = begin.to_string();
        let response = self
            .http
            .post(format!("{MP_ORIGIN}/mp/homepage"))
            .query(&[
                ("__biz", biz),
                ("hid", hid_s.as_str()),
                ("begin", begin_s.as_str()),
                ("count", HOMEPAGE_PAGE_SIZE.to_string().as_str()),
                ("action", "appmsg_list"),
            ])
            .header(REFERER, MP_ORIGIN)
            .header("X-Requested-With", "XMLHttpRequest")
            .body("")
            .send()
            .await
            .with_context(|| format!("请求公众号主页失败: __biz={biz} hid={hid}"))?;
        let status = response.status();
        let bytes = response.bytes().await.context("读取主页接口响应失败")?;
        if !status.is_success() {
            bail!("主页接口 HTTP {status}");
        }
        parse_mp_json(&bytes)
    }

    /// 搜狗请求：带上搜索页刚下发的 Cookie，并收下新的 Set-Cookie。
    /// 客户端全局是不跟随跳转的；这里若仍被送到验证码页，把正文原样交回去，让解析报出 HTTP 状态。
    async fn sogou_get(&mut self, url: &str, referer: Option<&str>) -> Result<SogouResponse> {
        let mut req = self
            .http
            .get(url)
            .header(USER_AGENT, SOGOU_MOBILE_UA)
            .header(ACCEPT_ENCODING, "identity");
        if let Some(referer) = referer {
            req = req.header(REFERER, referer);
        }
        let cookie = cookie_header(&self.sogou_cookies);
        if !cookie.is_empty() {
            req = req.header(COOKIE, cookie);
        }
        let response = req
            .send()
            .await
            .with_context(|| format!("请求搜狗失败: {url}"))?;
        absorb_cookies(&mut self.sogou_cookies, &response);
        let status = response.status();
        if status.is_redirection() {
            let loc = response
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            bail!("搜狗把请求转到了验证码页（HTTP {status} {loc}）");
        }
        let text = response
            .text()
            .await
            .with_context(|| format!("读取搜狗响应失败: {url}"))?;
        if !status.is_success() {
            bail!("搜狗 HTTP {status}");
        }
        if text.contains("antispider") || text.contains("请输入验证码") {
            bail!("搜狗要求验证码，这一页的跳转解不开");
        }
        Ok(SogouResponse { status, text })
    }

    /// 抓取一篇公开文章页，按 html / markdown / text 返回正文。不需要后台登录。
    pub async fn download_article(&self, url: &str, format: &str) -> Result<String> {
        if !url.contains("mp.weixin.qq.com") {
            bail!("不是微信文章链接: {url}");
        }
        let html = self.public_html(url).await?;
        let inner = extract_js_content(&html).context("这篇文章没有可读正文（可能已删除，或页面结构变了）")?;
        match format {
            "html" => Ok(inner),
            "text" => Ok(html_to_text(&inner)),
            "json" => Ok(serde_json::json!({
                "url": url,
                "html": inner,
                "text": html_to_text(&inner),
            })
            .to_string()),
            _ => Ok(html_to_markdown(&inner)),
        }
    }

    /// 从文章链接或文章页里取出公众号名和 fakeid（`__biz`）。
    pub async fn account_by_url(&self, url: &str) -> Result<Value> {
        let mut biz = query_param(url, "__biz").unwrap_or_default();
        let html = self.public_html(url).await?;
        if biz.is_empty() {
            biz = query_param(&find_mp_url(&html).unwrap_or_default(), "__biz").unwrap_or_default();
        }
        let title = meta_content(&html, "og:title").or_else(|| html_title(&html));
        let author = meta_content(&html, "og:article:author")
            .or_else(|| js_string(&html, "nickname"));
        Ok(serde_json::json!({
            "nickname": author,
            "fakeid": if biz.is_empty() { Value::Null } else { Value::String(biz) },
            "title": title,
        }))
    }

    async fn fetch_nickname(&mut self) -> Result<String> {
        let token = self.session.token.clone();
        let url = format!("{MP_ORIGIN}/cgi-bin/home?t=home/index&lang=zh_CN&token={token}");
        let body = self.raw_get_follow(&url).await?;
        let text = String::from_utf8_lossy(&body);
        js_string(&text, "nick_name")
            .or_else(|| js_string(&text, "nickname"))
            .context("首页里没有昵称")
    }

    async fn mp_get(&mut self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let bytes = self.mp_bytes(Method::GET, path, query, &[]).await?;
        parse_mp_json(&bytes)
    }

    async fn mp_form(
        &mut self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<Value> {
        let bytes = self.mp_bytes(method, path, query, form).await?;
        parse_mp_json(&bytes)
    }

    async fn mp_bytes(
        &mut self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<Vec<u8>> {
        let url = format!("{MP_ORIGIN}{path}");
        self.exchange(method, &url, query, form).await
    }

    async fn public_html(&self, url: &str) -> Result<String> {
        let response = self
            .http
            .get(url)
            .header(REFERER, MP_ORIGIN)
            .send()
            .await
            .with_context(|| format!("请求文章失败: {url}"))?;
        let status = response.status();
        let bytes = response.bytes().await.context("读取文章响应失败")?;
        // 不带 chksm 的文章链接会 302 到「环境异常」验证页。腾讯只校验参数存在，
        // 不校验取值，因此补一个占位 chksm 重试即可拿到完整正文。
        // 302 本身就是这个风控最常见的表现，所以一并走重试，不直接报错。
        if status.is_success() {
            let html = String::from_utf8_lossy(&bytes).into_owned();
            if extract_js_content(&html).is_some() {
                return Ok(html);
            }
            let Some(with_chksm) = with_placeholder_chksm(url) else {
                return Ok(html);
            };
            return self.get_html_retry(&with_chksm, status).await;
        }
        match with_placeholder_chksm(url) {
            Some(with_chksm) => self.get_html_retry(&with_chksm, status).await,
            None => bail!("文章页 HTTP {status}"),
        }
    }

    /// `public_html` 的重试分支：带上占位 `chksm` 再请求一次。
    async fn get_html_retry(&self, url: &str, first_status: reqwest::StatusCode) -> Result<String> {
        let retry = self
            .http
            .get(url)
            .header(REFERER, MP_ORIGIN)
            .send()
            .await
            .with_context(|| format!("重试文章失败: {url}"))?;
        let retry_status = retry.status();
        if !retry_status.is_success() {
            bail!("文章页 HTTP {first_status}（补 chksm 重试后仍为 {retry_status}）");
        }
        let retry_bytes = retry.bytes().await.context("读取文章响应失败")?;
        Ok(String::from_utf8_lossy(&retry_bytes).into_owned())
    }

    async fn raw_get_follow(&mut self, url: &str) -> Result<Vec<u8>> {
        self.exchange(Method::GET, url, &[], &[]).await
    }

    /// 手动跟跳转，这样 302 上的 Set-Cookie 不会被丢掉。
    async fn exchange(
        &mut self,
        method: Method,
        url: &str,
        query: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<Vec<u8>> {
        let mut current = url.to_string();
        let mut verb = method;
        let mut use_form = !form.is_empty();
        for _ in 0..6 {
            let mut req = self
                .http
                .request(verb.clone(), &current)
                .header(REFERER, MP_ORIGIN);
            if !query.is_empty() && !current.contains('?') {
                req = req.query(query);
            }
            let cookie = cookie_header(&self.session.cookies);
            if !cookie.is_empty() {
                req = req.header(COOKIE, cookie);
            }
            if use_form {
                req = req.form(form);
            }
            let response = req.send().await.with_context(|| format!("请求失败: {current}"))?;
            absorb_cookies(&mut self.session.cookies, &response);
            let status = response.status();
            if status.is_redirection() {
                let loc = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                if loc.is_empty() {
                    bail!("跳转没有 Location: {current}");
                }
                current = if loc.starts_with("http") {
                    loc
                } else if loc.starts_with('/') {
                    format!("{MP_ORIGIN}{loc}")
                } else {
                    format!("{MP_ORIGIN}/{loc}")
                };
                verb = Method::GET;
                use_form = false;
                continue;
            }
            let bytes = response.bytes().await.context("读取响应失败")?.to_vec();
            if !status.is_success() && status != StatusCode::OK {
                let snippet = String::from_utf8_lossy(&bytes);
                let snippet: String = snippet.chars().take(180).collect();
                bail!("HTTP {status} — {snippet}");
            }
            if !self.session.token.is_empty() {
                let _ = save_session(&self.session);
            }
            return Ok(bytes);
        }
        bail!("跳转次数过多: {url}")
    }
}

pub fn session_path() -> Result<PathBuf> {
        Ok(config_dir()?.join("session.json"))
    }

    pub fn load_session() -> Result<MpSession> {
        let path = session_path()?;
        if !path.exists() {
            return Ok(MpSession::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("读取登录会话失败: {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("解析登录会话失败: {}", path.display()))
    }

    pub fn save_session(session: &MpSession) -> Result<PathBuf> {
        let path = session_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(session).context("序列化登录会话失败")?;
        std::fs::write(&path, text)
            .with_context(|| format!("写入登录会话失败: {}", path.display()))?;
        Ok(path)
    }

    pub fn clear_session() -> Result<()> {
        let path = session_path()?;
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    fn millis_now() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    }

    fn cookie_header(cookies: &BTreeMap<String, String>) -> String {
        cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn absorb_cookies(cookies: &mut BTreeMap<String, String>, response: &reqwest::Response) {
        for value in response.headers().get_all(SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let pair = text.split(';').next().unwrap_or("").trim();
            let Some((name, raw)) = pair.split_once('=') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() || name.eq_ignore_ascii_case("path") {
                continue;
            }
            cookies.insert(name.to_string(), raw.trim().to_string());
        }
        let _ = response.headers().get(CONTENT_TYPE);
    }

    fn parse_mp_json(bytes: &[u8]) -> Result<Value> {
        let text = String::from_utf8_lossy(bytes);
        let text = text.trim().trim_start_matches('\u{feff}');
        serde_json::from_str(text).with_context(|| {
            let snippet: String = text.chars().take(180).collect();
            format!("微信接口没有返回 JSON: {snippet}")
        })
    }

    fn ret_of(value: &Value) -> Option<i64> {
        value
            .get("base_resp")
            .and_then(|b| b.get("ret"))
            .and_then(|v| v.as_i64())
    }

    fn err_msg(value: &Value) -> String {
        value
            .get("base_resp")
            .and_then(|b| b.get("err_msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("未知错误")
            .to_string()
    }

    fn ensure_ret(value: &Value, what: &str) -> Result<()> {
        match ret_of(value) {
            Some(0) => Ok(()),
            Some(200003) => bail!("{what}失败：登录已失效，请重新运行 mptext login"),
            // 2026-07-30 起，appmsgpublish / appmsg 的第一页也会返回这个码，响应里只有 base_resp。
            // 补 Origin、改 type=9、换成登录号自己的 fakeid 都一样，所以不要再提示成“等一会重试”。
            Some(200013) => bail!(
                "{what}被微信拒绝（200013 freq control）。登录和搜索仍然有效，但后台文章列表从 2026-07-30 起对第一页也直接拒绝，重新扫码不会恢复。公开文章链接仍可下载：mptext download <文章URL>"
            ),
            Some(code) => bail!("{what}失败 (ret={code}): {}", err_msg(value)),
            None => bail!("{what}失败：返回里没有 base_resp ({value})"),
        }
    }

    /// 从 `publish_page` 里拆出文章。微信把这一层又包成了字符串。
    pub fn parse_publish_page(value: &Value) -> Result<Vec<ArticleItem>> {
        let page = value
            .get("publish_page")
            .and_then(|v| v.as_str())
            .context("返回里没有 publish_page")?;
        let page: Value = serde_json::from_str(page).context("publish_page 不是 JSON")?;
        let list = page
            .get("publish_list")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut articles = Vec::new();
        for item in list {
            let Some(info) = item.get("publish_info").and_then(|v| v.as_str()) else {
                continue;
            };
            if info.is_empty() {
                continue;
            }
            let Ok(info) = serde_json::from_str::<Value>(info) else {
                continue;
            };
            let Some(msgs) = info.get("appmsgex").and_then(|v| v.as_array()) else {
                continue;
            };
            for msg in msgs {
                if let Some(article) = article_from_msg(msg) {
                    articles.push(article);
                }
            }
        }
        Ok(articles)
}

fn article_from_msg(msg: &Value) -> Option<ArticleItem> {
    let title = msg.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
    let link = msg
        .get("link")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    if title.is_empty() && link.is_empty() {
        return None;
    }
    let url = if link.starts_with("http") {
        link.to_string()
    } else {
        format!("{MP_ORIGIN}{link}")
    };
    Some(ArticleItem {
        title: if title.is_empty() {
            "未命名".to_string()
        } else {
            title.to_string()
        },
        url,
        create_time: msg.get("create_time").cloned(),
    })
}

pub fn parse_account_list(value: &Value) -> Result<Vec<AccountItem>> {
    let list = value
        .get("list")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for item in list {
        let Some(fakeid) = item.get("fakeid").and_then(|v| v.as_str()) else {
            continue;
        };
        let nickname = item
            .get("nickname")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let alias = item
            .get("alias")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        out.push(AccountItem {
            nickname,
            fakeid: fakeid.to_string(),
            alias,
            favorite: false,
        });
    }
    Ok(out)
}

pub fn token_from_url(url: &str) -> Option<String> {
    for part in url.split(['?', '&']) {
        if let Some(value) = part.strip_prefix("token=") {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// 取出 `id="js_content"` 这个 div 的内部 HTML。正文里还有嵌套 div，按深度配对。
pub fn extract_js_content(html: &str) -> Option<String> {
    let marker = html.find("id=\"js_content\"").or_else(|| html.find("id='js_content'"))?;
    let open_end = html[marker..].find('>')? + marker;
    let mut depth = 1i32;
    let bytes = &html.as_bytes()[open_end + 1..];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let rest = &html[open_end + 1 + i..];
            let lower = rest.get(..6).unwrap_or(rest).to_ascii_lowercase();
            if lower.starts_with("</div") {
                depth -= 1;
                if depth == 0 {
                    return Some(html[open_end + 1..open_end + 1 + i].to_string());
                }
            } else if lower.starts_with("<div") {
                let after = rest.find('>').unwrap_or(0);
                let tag = &rest[..=after.min(rest.len().saturating_sub(1))];
                if !tag.ends_with("/>") {
                    depth += 1;
                }
            }
        }
        i += 1;
    }
    None
}

/// 把正文 HTML 转成纯文字：只留文字与段落，图片、视频、播放器一律不出现。
///
/// 旧实现把所有标签一律换成空格再交给 `collapse_ws`，整篇文章会被压成一行——
/// 拿去当语料时段落结构全丢。这里改成按块级元素分段：段落、标题、列表项、换行
/// 各自结束一行，行内标签（`a` / `span` / `strong` …）直接剥掉不动文字。
///
/// 媒体内容靠两道处理保证不残留：`img` / `video` / `iframe` / `mpvideo` 本身不含
/// 文字节点，剥标签后自然不会留下；`script` / `style` 先整块删掉，避免脚本里的
/// 图片地址混进正文。
pub fn html_to_text(html: &str) -> String {
    let mut cleaned = html.to_string();
    for tag in ["script", "style", "iframe", "video", "mpvideo", "svg"] {
        cleaned = strip_tag_block(&cleaned, tag);
    }

    let mut out = String::new();
    let mut rest = cleaned.as_str();
    while let Some(rel) = rest.find('<') {
        out.push_str(&decode_entities(&rest[..rel]));
        rest = &rest[rel..];
        let Some(end) = rest.find('>') else {
            out.push_str(rest);
            break;
        };
        let tag = &rest[..=end];
        match tag_name(tag).as_str() {
            // 开标签也要断一次：`<h3>标题</h3><p>正文</p>` 这种紧挨着的写法，
            // 若只认闭标签，标题会和正文粘在同一行。
            "p" | "div" | "section" | "blockquote" | "li" | "tr" | "h1" | "h2" | "h3" | "h4"
            | "h5" | "h6" => out.push('\n'),
            _ => {}
        }
        rest = &rest[end + 1..];
    }
    out.push_str(&decode_entities(rest));

    // 源码缩进会留下行首空白，先逐行 trim 再折空行，段落之间保持一个空行。
    let trimmed = out
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n");
    collapse_blank_lines(&trimmed)
}

pub fn html_to_markdown(html: &str) -> String {
    let without_script = strip_tag_block(html, "script");
    let without_style = strip_tag_block(&without_script, "style");
    let mut out = String::new();
    let mut rest = without_style.as_str();
    while let Some(rel) = rest.find('<') {
        out.push_str(&decode_entities(&rest[..rel]));
        rest = &rest[rel..];
        let Some(end) = rest.find('>') else {
            out.push_str(rest);
            break;
        };
        let tag = &rest[..=end];
        let name = tag_name(tag);
        match name.as_str() {
            "br" => out.push('\n'),
            "p" | "div" | "section" | "h1" | "h2" | "h3" | "h4" | "tr" => {
                if !out.ends_with("\n\n") {
                    out.push_str("\n\n");
                }
            }
            "/p" | "/div" | "/section" | "/h1" | "/h2" | "/h3" | "/h4" | "/li" | "/tr" => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            "li" => out.push_str("\n- "),
            "img" => {
                if let Some(src) = attr_value(tag, "data-src").or_else(|| attr_value(tag, "src")) {
                    out.push_str(&format!("![]({src})"));
                }
            }
            _ => {}
        }
        rest = &rest[end + 1..];
    }
    out.push_str(&decode_entities(rest));
    collapse_blank_lines(&out)
}

fn strip_tag_block(html: &str, tag: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(start) = lower.find(&open) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let after = &rest[start + open.len()..];
        let lower_after = after.to_ascii_lowercase();
        if let Some(end) = lower_after.find(&close) {
            rest = &after[end + close.len()..];
        } else {
            break;
        }
    }
    out
}

fn tag_name(tag: &str) -> String {
    let inner = tag.trim_matches(|c| c == '<' || c == '>' || c == '/');
    // 标签名允许字母后跟数字：`h1`~`h6` 若只收字母会被截成 `h`，
    // 那些按 `h2` 写的分支就永远匹配不上（标题会粘进上一段）。
    let name: String = inner
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    if tag.starts_with("</") {
        format!("/{name}").to_ascii_lowercase()
    } else {
        name.to_ascii_lowercase()
    }
}

fn attr_value(tag: &str, name: &str) -> Option<String> {
    let patterns = [format!("{name}=\""), format!("{name}='")];
    for pattern in patterns {
        if let Some(pos) = tag.find(&pattern) {
            let rest = &tag[pos + pattern.len()..];
            let quote = pattern.chars().last()?;
            let end = rest.find(quote)?;
            return Some(decode_entities(&rest[..end]));
        }
    }
    None
}

fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::new();
    let mut blanks = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks <= 2 && !out.is_empty() {
                out.push('\n');
            }
        } else {
            blanks = 0;
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
    out.trim().to_string()
}

/// 给文章链接补一个占位 `chksm`，绕开「环境异常」验证页。
///
/// 腾讯只检查参数是否出现，不校验取值，所以任意非空值都有效，并且必须放在 `#` 之前。
/// 同时把 `http://` 换成 `https://`：`/mp/homepage` 返回的是 http 链接，
/// 本机走 http 会拿到 501，换 https 后才是一篇正常文章页。
/// 链接里已经有 `chksm`，或缺少 `__biz` / `mid` / `sn` 时返回 `None`。
fn with_placeholder_chksm(url: &str) -> Option<String> {
    if query_param(url, "chksm").is_some() {
        return None;
    }
    for key in ["__biz", "mid", "sn"] {
        query_param(url, key)?;
    }
    let url = url
        .strip_prefix("http://mp.weixin.qq.com")
        .map(|rest| format!("https://mp.weixin.qq.com{rest}"))
        .unwrap_or_else(|| url.to_string());
    let (base, fragment) = match url.split_once('#') {
        Some((base, frag)) => (base, Some(frag)),
        None => (url.as_str(), None),
    };
    let separator = if base.contains('?') { '&' } else { '?' };
    let mut out = format!("{base}{separator}chksm=1");
    if let Some(frag) = fragment {
        out.push('#');
        out.push_str(frag);
    }
    Some(out)
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split('?').nth(1).unwrap_or(url);
    for part in query.split('&') {
        if let Some(value) = part.strip_prefix(&format!("{key}=")) {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn find_mp_url(html: &str) -> Option<String> {
    let key = "mp.weixin.qq.com";
    let pos = html.find(key)?;
    let start = html[..pos].rfind("http").unwrap_or(pos);
    let end = html[pos..]
        .find(['"', '\'', ' ', '<'])
        .map(|i| pos + i)
        .unwrap_or(html.len());
    Some(html[start..end].to_string())
}

fn meta_content(html: &str, property: &str) -> Option<String> {
    for marker in [
        format!("property=\"{property}\""),
        format!("name=\"{property}\""),
    ] {
        let Some(pos) = html.find(&marker) else {
            continue;
        };
        let window_start = pos.saturating_sub(80);
        let window_end = (pos + 240).min(html.len());
        let window = &html[window_start..window_end];
        if let Some(value) = attr_value(&format!("<{window}>"), "content") {
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let rest = &html[start + 7..];
    let end = rest.to_ascii_lowercase().find("</title>")?;
    let title = decode_entities(rest[..end].trim());
    if title.is_empty() { None } else { Some(title) }
}

fn js_string(html: &str, key: &str) -> Option<String> {
    for pattern in [
        format!("{key} = \""),
        format!("{key}=\""),
        format!("\"{key}\":\""),
        format!("{key}: \""),
        format!("{key}:\""),
    ] {
        if let Some(pos) = html.find(&pattern) {
            let rest = &html[pos + pattern.len()..];
            let end = rest.find('"')?;
            let value = decode_entities(&rest[..end]);
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// 解析 `profile_ext?action=getmsg` 的一页，不访问网络。
/// 返回 (本页文章, 下一页 offset, 是否还有更多)。
///
/// 微信把列表塞在 `general_msg_list` 这个**字符串**里（里面又是一个 JSON）；
/// `next_offset` 和 `can_msg_continue` 在顶层。
pub(crate) fn parse_profile_ext_page(value: &Value) -> Result<(Vec<ArticleItem>, u32, bool)> {
    let raw = value
        .get("general_msg_list")
        .and_then(Value::as_str)
        .unwrap_or("");
    let next_offset = value
        .get("next_offset")
        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or(0) as u32;
    let more = value
        .get("can_msg_continue")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        != 0;
    if raw.trim().is_empty() {
        return Ok((Vec::new(), next_offset, false));
    }
    let list: Value = serde_json::from_str(raw).context("general_msg_list 不是 JSON")?;
    let empty = Vec::new();
    let items = list
        .get("list")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let mut articles = Vec::new();
    for item in items {
        // 只有图文消息（comm_msg_info.type == 49）才带 app_msg_ext_info。
        let Some(info) = item.get("app_msg_ext_info") else {
            continue;
        };
        let comm = item.get("comm_msg_info");
        let send_time = comm
            .and_then(|c| c.get("datetime"))
            .and_then(Value::as_i64);
        // 头条在 info 本身，多图文在 multi_app_msg_item_list 里。
        let mut entries = vec![info];
        if let Some(sub) = info.get("multi_app_msg_item_list").and_then(Value::as_array) {
            entries.extend(sub.iter());
        }
        for entry in entries {
            let title = entry
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let url = entry
                .get("content_url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .replace("\\/", "/")
                .replace("http://mp.weixin.qq.com", "https://mp.weixin.qq.com");
            if title.is_empty() || url.is_empty() {
                continue;
            }
            articles.push(ArticleItem {
                title,
                url,
                create_time: send_time.map(Value::from),
            });
        }
    }
    Ok((articles, next_offset, more))
}

/// 解析 `/mp/homepage?action=appmsg_list` 的一页返回，不访问网络。
/// `base_resp.ret` 非 0（hid 不存在时实测是 5）不算错误，返回 Ok(None)。
/// 链接统一升级成 https；空白标题、空链接的条目丢弃。
pub(crate) fn parse_homepage_json(value: &Value) -> Result<Option<HomepagePage>> {
    match value.pointer("/base_resp/ret").and_then(Value::as_i64) {
        Some(0) => {}
        Some(_) => return Ok(None),
        None => bail!("主页接口返回里没有 base_resp"),
    }
    let empty = Vec::new();
    let list = value
        .get("appmsg_list")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let articles = list
        .iter()
        .filter_map(|item| {
            let title = item.get("title")?.as_str()?.trim().to_string();
            if title.is_empty() {
                return None;
            }
            let url = item
                .get("link")?
                .as_str()?
                .replace("http://mp.weixin.qq.com", "https://mp.weixin.qq.com");
            if url.is_empty() {
                return None;
            }
            Some(ArticleItem {
                title,
                url,
                // 主页接口的时间字段叫 sendtime；留 create_time 兜底以防接口改名。
                create_time: item
                    .get("sendtime")
                    .or_else(|| item.get("create_time"))
                    .and_then(Value::as_i64)
                    .map(Value::from),
            })
        })
        .collect();
    Ok(Some(HomepagePage {
        articles,
        has_more: value.get("has_more").and_then(Value::as_i64).unwrap_or(0) != 0,
    }))
}

/// 解析 `/mp/appmsgalbum?action=getalbum` 的一页返回，不访问网络。
/// `base_resp.ret` 非 0（合集不存在或已删除）返回空页，由调用方停止翻页。
pub(crate) fn parse_album_json(value: &Value) -> Result<AlbumPage> {
    match value.pointer("/base_resp/ret").and_then(Value::as_i64) {
        Some(0) => {}
        Some(_) => {
            return Ok(AlbumPage {
                articles: Vec::new(),
                has_more: false,
                cursor: None,
            });
        }
        None => bail!("合集接口返回里没有 base_resp"),
    }
    let empty = Vec::new();
    // 末页只有一个对象时，接口把 `article_list` 直接给成该对象本身而不是
    // 长度为 1 的数组（实测「官察笔记」合集第 4 页是这样）。只认数组会
    // 把这一篇整个丢掉，并让翻页提前结束。
    let single;
    let list: &Vec<Value> = match value.pointer("/getalbum_resp/article_list") {
        Some(Value::Array(items)) => items,
        Some(item @ Value::Object(_)) => {
            single = vec![item.clone()];
            &single
        }
        _ => &empty,
    };
    let articles = list
        .iter()
        .filter_map(|item| {
            let title = item.get("title")?.as_str()?.trim().to_string();
            if title.is_empty() {
                return None;
            }
            let url = item
                .get("url")?
                .as_str()?
                .replace("http://mp.weixin.qq.com", "https://mp.weixin.qq.com");
            if url.is_empty() {
                return None;
            }
            Some(ArticleItem {
                title,
                url,
                // 合集接口的时间字段是字符串形式的 create_time。
                create_time: item
                    .get("create_time")
                    .and_then(|v| v.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| v.as_i64()))
                    .map(Value::from),
            })
        })
        .collect();
    // 游标取本页最后一条：下一頁从它之后开始。
    let cursor = list.last().and_then(|item| {
        let msgid = item.get("msgid")?.as_str()?.to_string();
        let itemidx = item
            .get("itemidx")
            .and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_i64().map(|n| n.to_string())))
            .unwrap_or_else(|| "1".to_string());
        Some((msgid, itemidx))
    });
    let has_more = value
        .pointer("/getalbum_resp/continue_flag")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| v.as_i64()))
        .unwrap_or(0)
        != 0;
    Ok(AlbumPage {
        articles,
        has_more,
        cursor,
    })
}

/// 从文章页 HTML 里抽出合集 id，去重后返回。合集 id 是 16 位以上的纯数字。
///
/// 同一页里会同时出现多种写法，**必须全部扫一遍**：早期版本用 `if out.is_empty()`
/// 让三种写法互斥，结果一页只带回一个合集，实测「官察笔记」4 个合集里有两个
/// 一直到不了，228 篇只拿到 158 篇。现在的三种写法：
///   - 链接：`appmsgalbum?__biz=…&album_id=1234567890123456`
///   - JSON：`"album_id":"3378123456789012"`
///   - 页面内联脚本的 JS 对象字面量：`album_id: '4352921130605740036'`
pub(crate) fn extract_album_ids(html: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for pattern in [
        r#"appmsgalbum\?__biz=[^"'<>\s]*?album_id=(\d{6,25})"#,
        r#""album_id"\s*:\s*"(\d{6,25})""#,
        r#"album_id\s*:\s*['"]?(\d{6,25})['"]?"#,
    ] {
        let Ok(re) = Regex::new(pattern) else { continue };
        for caps in re.captures_iter(html) {
            if let Some(id) = caps.get(1) {
                let id = id.as_str().to_string();
                if seen.insert(id.clone()) {
                    out.push(id);
                }
            }
        }
    }
    out
}

/// 主页接口翻出来的全量文章按时间倒序、按关键词过滤后切出一页，模拟后台列表接口的分页语义。
pub(crate) fn finish_homepage_articles(
    mut all: Vec<ArticleItem>,
    begin: u32,
    size: u32,
    keyword: Option<&str>,
) -> Vec<ArticleItem> {
    let ts = |a: &ArticleItem| a.create_time.as_ref().and_then(Value::as_i64).unwrap_or(0);
    all.sort_by_key(|a| std::cmp::Reverse(ts(a)));
    if let Some(kw) = keyword {
        let kw = kw.to_lowercase();
        all.retain(|a| a.title.to_lowercase().contains(&kw));
    }
    all.into_iter()
        .skip(begin as usize)
        .take(size as usize)
        .collect()
}

/// 解析搜狗移动搜索结果页，不访问网络。
/// 只收 `id` 以 `sogou_vr_` 开头的 `li`；`data-sourcename` 与 `nickname` 不一致的丢掉。
/// 这两个属性在子元素上（html5ever 会把属性名改成小写）。本页这类 `li` 少于 10 条则为最后一页。
pub(crate) fn parse_sogou_mobile_page(html: &str, nickname: &str) -> SogouMobilePage {
    let document = Html::parse_document(html);
    let li_selector = Selector::parse(r#"li[id^="sogou_vr_"]"#).expect("sogou li 选择器");
    let source_selector = Selector::parse("[data-sourcename]").expect("sourcename 选择器");
    let time_selector = Selector::parse("[data-lastmodified]").expect("lastModified 选择器");
    let title_selector = Selector::parse("h4").expect("标题选择器");
    let link_selector = Selector::parse(r#"a[href^="/link?url="]"#).expect("链接选择器");

    let rows: Vec<_> = document.select(&li_selector).collect();
    let rows_len = rows.len();
    let mut articles = Vec::new();
    for li in rows {
        let source_name = attr_on(&li, "data-sourcename", &source_selector);
        if source_name != nickname {
            continue;
        }
        let last_modified = attr_on(&li, "data-lastmodified", &time_selector)
            .parse::<u64>()
            .ok();
        let title = li
            .select(&title_selector)
            .next()
            .map(|node| node.text().collect::<String>())
            .unwrap_or_default();
        let title = title.split_whitespace().collect::<Vec<_>>().join("");
        let link = li
            .select(&link_selector)
            .next()
            .and_then(|node| node.value().attr("href"))
            .unwrap_or("")
            .to_string();
        if link.is_empty() {
            continue;
        }
        articles.push(SogouArticleRef {
            title: if title.is_empty() {
                "未命名".to_string()
            } else {
                title
            },
            link,
            source_name,
            last_modified,
        });
    }
    SogouMobilePage {
        articles,
        total_rows: rows_len,
    }
}

fn attr_on(li: &scraper::ElementRef<'_>, name: &str, selector: &Selector) -> String {
    li.value()
        .attr(name)
        .or_else(|| {
            li.select(selector)
                .next()
                .and_then(|node| node.value().attr(name))
        })
        .unwrap_or("")
        .to_string()
}

struct SogouResponse {
    status: StatusCode,
    text: String,
}

/// 从 `/link?url=` 返回的 JS 拼出文章 URL。
/// 先按 `url += '…'` 原文拼接，不对拼好的字符串做 HTML 实体解码：`&timestamp` 里的 `&times` 会被解码成乘号。
/// 搜狗现在把域名拆成 `https://mp.` + `weixin.qq.c` + `om/s?...`，拼完才像一条 URL。
pub(crate) fn extract_article_url_from_js(js_text: &str) -> Result<String> {
    if let Some(url) = join_sogou_url_chunks(js_text) {
        return Ok(url);
    }
    if let Some(url) = extract_url_from_js_pattern(js_text) {
        return Ok(url);
    }
    if let Some(url) = extract_url_from_attr(js_text) {
        return Ok(url);
    }
    let re = Regex::new(r#"(https?://[^\s"'<>]+)"#).unwrap();
    if let Some(captures) = re.captures(js_text) {
        let url = captures.get(0).unwrap().as_str();
        if url.contains("mp.weixin.qq.com") {
            return Ok(url.to_string());
        }
    }
    bail!("无法从JavaScript片段中提取文章URL: {}", js_text)
}

/// 把 `url += '…'` / `url += "…"` 按出现顺序拼起来。片段本身不再做实体解码。
/// 只认赋值之后的片段，避免把 `var url = ''` 的空串也拼进去。
fn join_sogou_url_chunks(js_text: &str) -> Option<String> {
    let re = Regex::new(r#"(?s)url\s*\+=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap();
    let mut url = String::new();
    let mut found = false;
    for captures in re.captures_iter(js_text) {
        let part = captures
            .get(1)
            .or_else(|| captures.get(2))
            .map(|m| m.as_str())
            .unwrap_or("");
        url.push_str(part);
        found = true;
    }
    if found && url.contains("mp.weixin.qq.com") {
        Some(url)
    } else {
        None
    }
}

/// 从一次赋值里取出 URL（`url = '…'`），不处理 `url +=`。
pub(crate) fn extract_url_from_js_pattern(js_text: &str) -> Option<String> {
    let patterns = [
        r#"url\s*=\s*["']([^"']+)["']"#,
        r#"link\s*=\s*["']([^"']+)["']"#,
        r#"["'](https?://mp\.weixin\.qq\.com/[^"']+)["']"#,
    ];
    for pattern in patterns {
        let re = Regex::new(pattern).ok()?;
        if let Some(captures) = re.captures(js_text) {
            let url = captures.get(1).or_else(|| captures.get(0))?.as_str();
            if url.contains("mp.weixin.qq.com") {
                return Some(url.to_string());
            }
        }
    }
    None
}

/// 从 `href` / `src` 属性里取出微信文章 URL。
pub(crate) fn extract_url_from_attr(js_text: &str) -> Option<String> {
    for pattern in [
        r#"href\s*=\s*["']([^"']+)["']"#,
        r#"src\s*=\s*["']([^"']+)["']"#,
    ] {
        let re = Regex::new(pattern).unwrap();
        if let Some(captures) = re.captures(js_text) {
            let url = captures.get(1)?.as_str();
            if url.contains("mp.weixin.qq.com") {
                return Some(url.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn album_last_page_may_hold_a_single_object() {
        // 实测「官察笔记」合集第 4 页：只剩一篇时 article_list 直接是对象本身，
        // 不是长度 1 的数组。按数组解析会丢掉这一篇并让翻页提前结束。
        let value = serde_json::json!({
            "base_resp": {"ret": 0},
            "getalbum_resp": {
                "continue_flag": "0",
                "article_list": {
                    "title": "为什么很多领导，都有当秘书的经历？",
                    "url": "http://mp.weixin.qq.com/s?__biz=MzIwMDY3MTE2NQ==&mid=2247496700&idx=1",
                    "create_time": "1792450200",
                    "msgid": "2247496700",
                    "itemidx": "1"
                }
            }
        });
        let page = parse_album_json(&value).expect("应当解析成功");
        assert_eq!(page.articles.len(), 1);
        assert_eq!(page.articles[0].title, "为什么很多领导，都有当秘书的经历？");
        assert!(!page.has_more);
    }

    #[test]
    fn finds_album_id_written_as_js_object_literal() {
        // 文章页里合集 id 的常规写法是内联脚本的对象字面量，
        // 只认 `appmsgalbum?__biz=...album_id=` 链接会一个都找不到。
        let html = r#"<script>var d = {album_id: '4352921130605740036', album_title: '全部'};</script>"#;
        assert_eq!(extract_album_ids(html), vec!["4352921130605740036".to_string()]);
    }

    #[test]
    fn profile_ext_flattens_multi_and_unescapes_slashes() {
        // 真实返回里 general_msg_list 是字符串，里面再套一层 JSON。
        let inner = r#"{"list":[
            {"comm_msg_info":{"type":49,"datetime":1766900000},
             "app_msg_ext_info":{
                "title":"头条",
                "content_url":"http://mp.weixin.qq.com/s?__biz=MzY=\u0026mid=1\u0026idx=1\u0026sn=a",
                "multi_app_msg_item_list":[
                   {"title":"次条","content_url":"http:\/\/mp.weixin.qq.com\/s?mid=2"}
                ]}},
            {"comm_msg_info":{"type":49,"datetime":1766800000},
             "app_msg_ext_info":{"title":"旧头条","content_url":"http://mp.weixin.qq.com/s?mid=3"}},
            {"comm_msg_info":{"type":1,"datetime":1766700000}}
        ]}"#;
        let value = serde_json::json!({
            "base_resp": {"ret": 0},
            "general_msg_list": inner,
            "next_offset": 10,
            "can_msg_continue": 1
        });
        let (articles, next, more) = parse_profile_ext_page(&value).expect("应当解析成功");
        assert_eq!(next, 10);
        assert!(more);
        // 头条 + 次条 + 旧头条；纯文本消息没有 app_msg_ext_info，被丢弃。
        assert_eq!(articles.len(), 3);
        assert_eq!(articles[0].title, "头条");
        assert_eq!(articles[1].title, "次条");
        assert_eq!(articles[2].title, "旧头条");
        // 反斜杠转义和 http 前缀都要归一。
        assert!(articles[1].url.starts_with("https://mp.weixin.qq.com/s?mid=2"));
        assert!(articles[0].url.starts_with("https://mp.weixin.qq.com/s?__biz=MzY=&mid=1"));
        assert_eq!(
            articles[0].create_time.as_ref().and_then(Value::as_i64),
            Some(1766900000)
        );
    }

    #[test]
    fn profile_ext_empty_list_is_terminal() {
        let value = serde_json::json!({
            "base_resp": {"ret": 0},
            "general_msg_list": "",
            "next_offset": 0,
            "can_msg_continue": 1
        });
        let (articles, _, more) = parse_profile_ext_page(&value).expect("空列表不算错误");
        assert!(articles.is_empty());
        // 即使 can_msg_continue=1，没有内容也必须停，否则会无限翻。
        assert!(!more);
    }

    #[test]
    fn chksm_goes_before_fragment_and_upgrades_http() {
        let raw = "http://mp.weixin.qq.com/s?__biz=MzY=&mid=1&idx=1&sn=abc&scene=19#wechat_redirect";
        let fixed = with_placeholder_chksm(raw).expect("应当补 chksm");
        assert_eq!(
            fixed,
            "https://mp.weixin.qq.com/s?__biz=MzY=&mid=1&idx=1&sn=abc&scene=19&chksm=1#wechat_redirect"
        );
        // 拿到 fragment 之后再加 chksm 会被页面忽略，仍然进验证页。
        assert!(fixed.find("chksm").unwrap() < fixed.find('#').unwrap());
    }

    #[test]
    fn chksm_is_skipped_when_already_present_or_incomplete() {
        let with_chksm =
            "https://mp.weixin.qq.com/s?__biz=MzY=&mid=1&idx=1&sn=abc&chksm=deadbeef";
        assert!(with_placeholder_chksm(with_chksm).is_none());
        let no_sn = "https://mp.weixin.qq.com/s?__biz=MzY=&mid=1&idx=1";
        assert!(with_placeholder_chksm(no_sn).is_none());
    }

    #[test]
    fn publish_page_flattens_appmsgex() {
        let raw = r#"{
            "base_resp": {"ret": 0, "err_msg": "ok"},
            "publish_page": "{\"publish_list\":[{\"publish_info\":\"{\\\"appmsgex\\\":[{\\\"title\\\":\\\"你好\\\",\\\"link\\\":\\\"https://mp.weixin.qq.com/s/abc\\\",\\\"create_time\\\":1710000000}]}\"}]}"
        }"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let articles = parse_publish_page(&value).unwrap();
        assert_eq!(articles.len(), 1);
        assert_eq!(articles[0].title, "你好");
        assert_eq!(articles[0].url, "https://mp.weixin.qq.com/s/abc");
    }

    #[test]
    fn js_content_keeps_nested_divs() {
        let html = r#"<div class="rich" id="js_content"><p>甲</p><div><p>乙</p></div></div><div>尾</div>"#;
        let inner = extract_js_content(html).unwrap();
        assert!(inner.contains("甲"));
        assert!(inner.contains("乙"));
        assert!(!inner.contains("尾"));
    }

    #[test]
    fn markdown_keeps_image_and_paragraph() {
        let html = r#"<p>开头</p><img data-src="https://mmbiz.qpic.cn/a.png" src="">"#;
        let md = html_to_markdown(html);
        assert!(md.contains("开头"));
        assert!(md.contains("![](https://mmbiz.qpic.cn/a.png)"));
    }

    #[test]
    fn text_drops_media_and_keeps_paragraphs() {
        let html = r#"
            <p>第一段文字</p>
            <img data-src="https://mmbiz.qpic.cn/a.png" src="">
            <iframe src="https://mp.weixin.qq.com/mp/videoplayer"></iframe>
            <video src="https://mpvideo.qpic.cn/b.mp4"></video>
            <script>var cover = "https://mmbiz.qpic.cn/cover.png";</script>
            <p>第二段<strong>加粗</strong>文字</p>
        "#;
        let text = html_to_text(html);
        // 纯文字里不能出现任何图片、视频、播放器地址。
        assert!(!text.contains("mmbiz.qpic.cn"));
        assert!(!text.contains("mpvideo"));
        assert!(!text.contains("videoplayer"));
        assert!(!text.contains("cover.png"));
        // 段落仍要分段：两段各自成行，不能被压成一整行。
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines, vec!["第一段文字", "第二段加粗文字"]);
    }

    #[test]
    fn heading_starts_its_own_line() {
        // `h2` 里的数字曾让 tag_name 截成 "h"，标题会粘在上一段尾部。
        let html = r#"<p>上一段</p><h2><span>小标题</span></h2><p>下一段</p>"#;
        let lines: Vec<String> = html_to_text(html)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect();
        assert_eq!(lines, vec!["上一段", "小标题", "下一段"]);
        // markdown 那条路同样要分段：只断言「小标题独占一行」，
        // 不锁死前后空行数（块级开闭标签各会补一次换行，空行数实现相关）。
        let md = html_to_markdown(html);
        assert!(md.lines().any(|l| l.trim() == "小标题"));
        assert!(!md.contains("上一段小标题"));
    }

    #[test]
    fn token_is_taken_from_redirect() {
        let url = "/cgi-bin/home?t=home/index&lang=zh_CN&token=1841";
        assert_eq!(token_from_url(url).as_deref(), Some("1841"));
    }

    fn fixture(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("读取夹具 {} 失败: {err}", path.display()))
    }

    #[test]
    fn test_sogou_url_extraction() {
        let js = "var url = 'https://mp.weixin.qq.com/s?__biz=MzUxxxx&mid=2247483647&idx=1&sn=abc&chksm=def#rd'";
        assert_eq!(
            extract_url_from_js_pattern(js).unwrap(),
            "https://mp.weixin.qq.com/s?__biz=MzUxxxx&mid=2247483647&idx=1&sn=abc&chksm=def#rd"
        );

        let js2 = "<a href=\"https://mp.weixin.qq.com/s?__biz=MzUxxxx&mid=2247483647&idx=1&sn=abc&chksm=def#rd\">link</a>";
        assert_eq!(
            extract_url_from_attr(js2).unwrap(),
            "https://mp.weixin.qq.com/s?__biz=MzUxxxx&mid=2247483647&idx=1&sn=abc&chksm=def#rd"
        );

        let js3 = "var link = \"https://mp.weixin.qq.com/s?__biz=MzUxxxx&mid=2247483647&idx=1&sn=abc&chksm=def#rd\";";
        assert_eq!(
            extract_article_url_from_js(js3).unwrap(),
            "https://mp.weixin.qq.com/s?__biz=MzUxxxx&mid=2247483647&idx=1&sn=abc&chksm=def#rd"
        );
    }

    #[test]
    fn sogou_mobile_page_keeps_matching_source_and_marks_short_page() {
        let html = fixture("sogou_mobile_page.html");
        let page = parse_sogou_mobile_page(&html, "廉洁江西");
        assert_eq!(page.articles.len(), 1, "昵称不一致的条目必须丢掉");
        assert!(
            page.total_rows > page.articles.len(),
            "total_rows 数的是整页条目，不是按昵称过滤后的条数——翻页靠它判断到底"
        );
        let item = &page.articles[0];
        assert_eq!(item.source_name, "廉洁江西");
        assert_eq!(item.last_modified, Some(1_792_287_242));
        assert!(item.title.contains("人民日报"));
        assert!(item.title.contains("三资"));
        assert!(item.link.starts_with("/link?url="));
        assert!(
            item.link.contains("&type=2"),
            "列表 href 里的 &amp; 是参数分隔符，应还原成 &"
        );
        assert!(!item.link.contains("should-not-be-kept"));
        assert!(!item.title.contains("盛廷"));
    }

    #[test]
    fn sogou_full_page_is_not_last() {
        let row = r#"<li id="sogou_vr_11002601_box_0"><h4><a href="/link?url=abc&amp;type=2"><div>标题</div></a></h4><span data-sourcename="廉洁江西"></span><span data-lastModified="10"></span></li>"#;
        let html = format!("<ul>{}</ul>", row.repeat(SOGOU_PAGE_SIZE));
        let page = parse_sogou_mobile_page(&html, "廉洁江西");
        assert_eq!(page.articles.len(), SOGOU_PAGE_SIZE);
    }

    #[test]
    fn sogou_link_js_keeps_timestamp_ampersand() {
        let js = fixture("sogou_link.js");
        let url = extract_article_url_from_js(&js).unwrap();
        assert_eq!(
            url,
            "https://mp.weixin.qq.com/s?src=11&timestamp=1710000000&ver=3554&signature=abc&new=1"
        );
        assert!(url.contains("&timestamp="));
        assert!(!url.contains('\u{00d7}'));
    }

    #[test]
    fn sogou_link_js_joins_split_host() {
        let js = fixture("sogou_link_split.js");
        let url = extract_article_url_from_js(&js).unwrap();
        assert!(url.starts_with("https://mp.weixin.qq.com/s?src=11&timestamp="));
        assert!(url.contains("&signature="));
        assert!(url.ends_with("&new=1"));
        assert!(!url.contains("approve"));
    }

    fn homepage_value(raw: &str) -> Value {
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn homepage_json_parses_articles_and_upgrades_https() {
        let page = parse_homepage_json(&homepage_value(
            r#"{
                "base_resp": {"ret": 0, "err_msg": "ok"},
                "appmsg_list": [
                    {"aid": 100, "title": "欢迎来到 Web 4.0！",
                     "link": "http://mp.weixin.qq.com/s?__biz=Mzg2OTg5NDg3Mg==&mid=1&idx=1&sn=a",
                     "sendtime": 1767000000},
                    {"aid": 101, "title": "   ",
                     "link": "http://mp.weixin.qq.com/s?__biz=x&mid=2&idx=1&sn=b",
                     "sendtime": 1766900000}
                ],
                "has_more": 1
            }"#,
        ))
        .unwrap()
        .expect("ret=0 应有结果");
        assert!(page.has_more);
        assert_eq!(page.articles.len(), 1, "空白标题的条目应被丢弃");
        assert_eq!(page.articles[0].title, "欢迎来到 Web 4.0！");
        assert!(page.articles[0].url.starts_with("https://mp.weixin.qq.com"));
        assert_eq!(page.articles[0].create_time, Some(Value::from(1767000000)));
    }

    #[test]
    fn homepage_json_missing_hid_is_none() {
        let parsed = parse_homepage_json(&homepage_value(r#"{"base_resp": {"ret": 5}}"#))
            .unwrap();
        assert!(parsed.is_none());
    }

    #[test]
    fn homepage_json_without_base_resp_is_error() {
        assert!(parse_homepage_json(&homepage_value(r#"{"appmsg_list": []}"#)).is_err());
    }

    #[test]
    fn homepage_articles_sorted_filtered_sliced() {
        let mk = |title: &str, ts: i64| ArticleItem {
            title: title.into(),
            url: format!("https://mp.weixin.qq.com/s/{title}"),
            create_time: Some(Value::from(ts)),
        };
        let page = finish_homepage_articles(vec![mk("a", 100), mk("b", 300), mk("c", 200)], 0, 2, None);
        let titles: Vec<_> = page.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(titles, ["b", "c"]);

        let page = finish_homepage_articles(
            vec![mk("深度复盘", 300), mk("清迈印象", 200), mk("清迈印象（中）", 100)],
            1,
            10,
            Some("清迈"),
        );
        let titles: Vec<_> = page.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(titles, ["清迈印象（中）"]);
    }
}