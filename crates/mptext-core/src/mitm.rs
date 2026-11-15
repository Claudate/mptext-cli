//! 从微信客户端流量里抓 `/mp/author` 需要的票据。
//!
//! 为什么必须抓包：`author_id` 在公开文章页里就有，但 `uin` / `key` /
//! `pass_ticket` 只存在于**真实登录的微信客户端**发出的请求里。匿名页面里这三项
//! 是空的，直接打作者接口只会得到 `ret=-3 no session`。
//!
//! 这里起一个本地 MITM 代理（hudsucker），只拦截 `mp.weixin.qq.com`，
//! 其余流量原样转发。微信客户端打开任意一篇公众号文章时，请求 URL 里就带着
//! 这三项票据，抓下来存进 `wxcapture.json`，再由 [`crate::author`] 换成 Cookie。
//!
//! 抓包要成立，机器上得有两件事：
//! 1. 本模块生成的 CA 被系统信任（[`trust_ca`]）；
//! 2. 系统 HTTP/HTTPS 代理指向本代理（[`set_system_proxy`]）。
//!
//! 两步都需要管理员权限，所以都做成由用户在界面上点一下才执行的动作，
//! 不在后台偷偷改系统设置。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use hudsucker::certificate_authority::RcgenAuthority;
use hudsucker::hyper::Request;
use hudsucker::rustls::crypto::aws_lc_rs;
use hudsucker::{Body, HttpContext, HttpHandler, Proxy, RequestOrResponse};
use rcgen::{BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair};
use serde::Serialize;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::author::{AuthorCredential, capture_for, save_capture};
use crate::config::config_dir;

/// 默认监听端口。选一个不常见的值，避免和本机已有代理工具撞车。
pub const DEFAULT_PORT: u16 = 8899;

fn ca_cert_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("mptext-mitm-ca.pem"))
}

fn ca_key_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("mptext-mitm-ca.key"))
}

/// 生成（或复用）本机 MITM 用的根证书。
///
/// 复用是刻意的：证书一旦装进系统钥匙串，重新生成会让旧信任失效，
/// 用户就得反复授权。只有文件缺失或损坏时才重新生成。
pub fn ensure_ca() -> Result<(PathBuf, PathBuf)> {
    let cert_path = ca_cert_path()?;
    let key_path = ca_key_path()?;
    if cert_path.exists() && key_path.exists() {
        return Ok((cert_path, key_path));
    }
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir)?;

    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "mptext MITM CA");
    dn.push(DnType::OrganizationName, "mptext");
    params.distinguished_name = dn;

    let key = KeyPair::generate().context("生成 CA 私钥失败")?;
    let cert = params
        .self_signed(&key)
        .context("签发 CA 证书失败")?;

    std::fs::write(&cert_path, cert.pem())
        .with_context(|| format!("写入 CA 证书失败: {}", cert_path.display()))?;
    std::fs::write(&key_path, key.serialize_pem())
        .with_context(|| format!("写入 CA 私钥失败: {}", key_path.display()))?;
    Ok((cert_path, key_path))
}

/// 代理运行状态，交给界面展示。
#[derive(Debug, Clone, Serialize, Default)]
pub struct MitmStatus {
    pub running: bool,
    pub port: u16,
    /// 已经抓到票据的公众号数量。
    pub captured_accounts: usize,
    /// 最近一次抓到的 `__biz`，空表示还没抓到。
    pub last_biz: String,
    /// 提示信息，例如「已抓到票据，请在微信里继续浏览」。
    pub message: String,
}

/// 一次成功抓取的记录，用于让界面看到「刚刚抓到了」。
#[derive(Debug, Clone, Default)]
struct CaptureSnapshot {
    biz: String,
    uin: String,
    key: String,
}

/// 代理自身的共享状态：最后一次抓取快照，用来避免每个请求都重写磁盘。
#[derive(Clone)]
struct MitmHandler {
    last: Arc<Mutex<CaptureSnapshot>>,
}

impl HttpHandler for MitmHandler {
    async fn handle_request(&mut self, _ctx: &HttpContext, req: Request<Body>) -> RequestOrResponse {
        if let Some(cred) = credential_from_request(&req) {
            persist_capture(cred, &self.last);
        }
        req.into()
    }

    /// 只拦截微信域名的 TLS，避免把整机流量都解密。
    ///
    /// 返回 false 时 hudsucker 走 CONNECT 隧道透传，不碰证书。
    async fn should_intercept_connect(
        &mut self,
        _ctx: &HttpContext,
        req: &Request<Body>,
    ) -> bool {
        let host = req.uri().authority().map(|a| a.host()).unwrap_or("");
        host == "mp.weixin.qq.com" || host.ends_with(".weixin.qq.com")
    }
}

/// 解析查询串，同时做 percent-decode。
///
/// 微信把 `uin` 编码成 `MTIzNDU%3D` 这类形式，不解码会把带 `%3D` 的串写进票据文件，
/// 后续请求再被服务端编码一次，`uin` 就对不上了。
fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (
                urlencoding::decode(k).map(|s| s.into_owned()).unwrap_or_else(|_| k.to_string()),
                urlencoding::decode(v).map(|s| s.into_owned()).unwrap_or_else(|_| v.to_string()),
            )
        })
        .collect()
}

/// 从一条解密后的微信请求里抽出票据。不是微信请求、或票据不全时返回 `None`。
fn credential_from_request(req: &Request<Body>) -> Option<AuthorCredential> {
    let uri = req.uri();
    let host = uri.authority().map(|a| a.host()).unwrap_or("");
    if !host.ends_with("weixin.qq.com") {
        return None;
    }
    let query = parse_query(uri.query().unwrap_or_default());

    let get = |name: &str| -> String {
        query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };

    let uin = get("uin");
    let key = get("key");
    // `__biz` 也可能只出现在 Cookie / 路径里，但实测参数里必带。
    let biz = get("__biz");
    if uin.is_empty() || key.is_empty() {
        // 微信的静态资源、埋点请求都会走到这里，正常跳过。
        return None;
    }
    let pass_ticket = get("pass_ticket");
    let appmsg_token = get("appmsg_token");
    let client_cookie = req
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    Some(AuthorCredential {
        biz,
        uin,
        key,
        pass_ticket,
        appmsg_token,
        client_cookie,
        captured_at: crate::author::unix_now(),
        ..Default::default()
    })
}

/// 把抓到的票据并进已有记录。同名号只更新票据字段，
/// 保留已经换来的 Cookie 与 `author_id`，否则每抓到一次就得重新换 Cookie。
fn persist_capture(mut fresh: AuthorCredential, last: &Arc<Mutex<CaptureSnapshot>>) {
    let snapshot_key = format!("{}|{}", fresh.biz, fresh.key);
    {
        let mut guard = match last.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        // 同一个 key 反复出现（微信会为一篇文章发很多子请求），不必重复落盘。
        if guard.key == fresh.key && guard.uin == fresh.uin && guard.biz == fresh.biz {
            return;
        }
        guard.biz = fresh.biz.clone();
        guard.uin = fresh.uin.clone();
        guard.key = fresh.key.clone();
        let _ = snapshot_key;
    }

    let existing = capture_for(&fresh.biz);
    if let Some(old) = existing {
        if old.biz == fresh.biz {
            if fresh.pass_ticket.is_empty() {
                fresh.pass_ticket = old.pass_ticket;
            }
            if fresh.appmsg_token.is_empty() {
                fresh.appmsg_token = old.appmsg_token;
            }
            if fresh.client_cookie.is_empty() {
                fresh.client_cookie = old.client_cookie;
            }
            // 票据换了，旧 Cookie 对应的是旧票据，一并作废。
            if old.key != fresh.key {
                fresh.cookie.clear();
                fresh.cookie_expires_at = 0;
            } else {
                fresh.cookie = old.cookie;
                fresh.cookie_expires_at = old.cookie_expires_at;
            }
            fresh.author_id = old.author_id;
            fresh.nickname = old.nickname;
        }
    }
    let _ = save_capture(&fresh);
}

/// 正在运行的代理。用全局单例，界面重复点「开始抓包」不会起第二个。
struct RunningProxy {
    port: u16,
    shutdown: Option<oneshot::Sender<()>>,
    handle: JoinHandle<()>,
}

fn running_slot() -> &'static Mutex<Option<RunningProxy>> {
    static SLOT: OnceLock<Mutex<Option<RunningProxy>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// 启动代理。重复调用时返回已在运行的端口。
pub async fn start(port: u16) -> Result<u16> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let (cert_path, key_path) = ensure_ca()?;
    let ca_cert = std::fs::read_to_string(&cert_path)
        .with_context(|| format!("读取 CA 证书失败: {}", cert_path.display()))?;
    let ca_key = std::fs::read_to_string(&key_path)
        .with_context(|| format!("读取 CA 私钥失败: {}", key_path.display()))?;
    let key_pair = KeyPair::from_pem(&ca_key).context("解析 CA 私钥失败")?;
    let issuer = Issuer::from_ca_cert_pem(&ca_cert, key_pair).context("构造 CA 签发者失败")?;

    let provider = aws_lc_rs::default_provider();
    let ca = RcgenAuthority::new(issuer, 512, provider.clone());
    let handler = MitmHandler {
        last: Arc::new(Mutex::new(CaptureSnapshot::default())),
    };

    let (tx, rx) = oneshot::channel::<()>();
    let proxy = Proxy::builder()
        .with_addr(addr)
        .with_ca(ca)
        .with_rustls_connector(provider)
        .with_http_handler(handler)
        .with_graceful_shutdown(async {
            let _ = rx.await;
        })
        .build()
        .context("构造抓包代理失败")?;

    let handle = tokio::spawn(async move {
        if let Err(err) = proxy.start().await {
            eprintln!("抓包代理退出: {err}");
        }
    });

    // 等端口真正在监听，避免界面刚显示「已开启」就连不上。
    for _ in 0..40 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let mut slot = running_slot().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(old) = slot.take() {
        if let Some(sd) = old.shutdown {
            let _ = sd.send(());
        }
        old.handle.abort();
    }
    *slot = Some(RunningProxy {
        port,
        shutdown: Some(tx),
        handle,
    });
    Ok(port)
}

/// 停止代理。
pub fn stop() {
    let mut slot = running_slot().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(old) = slot.take() {
        if let Some(sd) = old.shutdown {
            let _ = sd.send(());
        }
        old.handle.abort();
    }
}

/// 代理是否在运行。
pub fn is_running() -> Option<u16> {
    let slot = running_slot().lock().unwrap_or_else(|e| e.into_inner());
    slot.as_ref().map(|p| p.port)
}

/// 当前抓取状态，给界面轮询。
pub fn status() -> MitmStatus {
    let captures = crate::author::load_captures();
    let last = captures
        .values()
        .max_by_key(|c: &&AuthorCredential| c.captured_at);
    let running = is_running();
    let captured_accounts = captures.values().filter(|c| c.is_complete()).count();
    let last_biz = last.map(|c| c.biz.clone()).unwrap_or_default();
    let message = if running.is_none() {
        "代理未运行".to_string()
    } else if captured_accounts == 0 {
        "代理已运行。请在微信里打开任意一篇公众号文章".to_string()
    } else {
        format!("已抓到 {captured_accounts} 个公众号的票据，可以换取 Cookie 了")
    };
    MitmStatus {
        running: running.is_some(),
        port: running.unwrap_or(DEFAULT_PORT),
        captured_accounts,
        last_biz,
        message,
    }
}

// ---------------------------------------------------------------------------
// 系统代理与证书信任（macOS）。两步都要管理员权限，只由用户在界面上触发。
// ---------------------------------------------------------------------------

/// 以**当前用户身份**执行一条命令，权限提升交给系统自己的授权框。
///
/// 不能自己套 `osascript ... with administrator privileges` 再跑 `security`：
/// 那样拿到的是 root 进程，而 root 没有图形会话，`SecTrustSettingsSetTrustSettings`
/// 会回 `The authorization was denied since no user interaction was possible`。
/// `security` 自己调用时 SecurityAgent 才有地方弹框，这也正是 mkcert 的做法。
///
/// 用户取消授权时返回错误，调用方按「用户取消」处理即可，不要当崩溃。
#[cfg(target_os = "macos")]
fn run_current_user(program: &str, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("调用 {program} 失败"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        bail!("执行 {program} 失败：{}", err.trim());
    }
    Ok(())
}

/// 找到当前主用网络服务名（通常是 "Wi-Fi"）。
///
/// `networksetup` 按服务名操作，而服务名是本地化的，不能写死。
/// 用「有 IP 地址的第一个服务」来判定主用链路。
#[cfg(target_os = "macos")]
pub fn active_network_service() -> Result<String> {
    let out = std::process::Command::new("networksetup")
        .arg("-listallnetworkservices")
        .output()
        .context("读取网络服务列表失败")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut candidates = Vec::new();
    for line in text.lines().skip(1) {
        let name = line.trim();
        if name.is_empty() || name.starts_with('*') {
            // 带 * 的是被停用的服务。
            continue;
        }
        candidates.push(name.to_string());
    }
    for name in &candidates {
        let info = std::process::Command::new("networksetup")
            .arg("-getinfo")
            .arg(name)
            .output();
        let Ok(info) = info else { continue };
        let body = String::from_utf8_lossy(&info.stdout);
        if body.contains("IP address: ") {
            let ip = body
                .lines()
                .find_map(|l| l.trim().strip_prefix("IP address: "))
                .unwrap_or("");
            if !ip.trim().is_empty() && ip.trim() != "none" {
                return Ok(name.clone());
            }
        }
    }
    candidates
        .into_iter()
        .next()
        .context("没有找到可用的网络服务")
}

/// 打开/关闭系统 HTTP + HTTPS 代理。
#[cfg(target_os = "macos")]
pub fn set_system_proxy(enabled: bool, host: &str, port: u16) -> Result<()> {
    let service = active_network_service()?;
    let port_s = port.to_string();
    // 一条命令里改 HTTP 与 HTTPS 两项，用 zsh 串起来一次授权弹框完成。
    let script = if enabled {
        format!(
            "networksetup -setwebproxy '{service}' {host} {port_s} && \
             networksetup -setsecurewebproxy '{service}' {host} {port_s}"
        )
    } else {
        format!(
            "networksetup -setwebproxystate '{service}' off && \
             networksetup -setsecurewebproxystate '{service}' off"
        )
    };
    run_shell_with_authorization(&script)
}

/// 用 osascript 以管理员身份跑一条 shell 命令，用于 `networksetup`。
///
/// `networksetup` 改的是系统级网络配置，必须提权，这一步无法避免 root；
/// 它与证书信任不同——不依赖图形会话，所以 `administrator privileges` 在这里是可行的。
#[cfg(target_os = "macos")]
fn run_shell_with_authorization(command: &str) -> Result<()> {
    let escaped = command.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("do shell script \"{escaped}\" with administrator privileges");
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .context("调用 osascript 失败")?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        bail!("需要管理员权限：{}", err.trim());
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn set_system_proxy(_enabled: bool, _host: &str, _port: u16) -> Result<()> {
    bail!("系统代理的自动设置目前只支持 macOS，请手动把 HTTP/HTTPS 代理指到 127.0.0.1:{_port}")
}

/// 把 CA 加进**当前用户**的信任设置。
///
/// 走登录钥匙串而不是系统钥匙串：微信客户端和本程序都跑在这个用户下，
/// 用户域信任对它们一样有效，但不需要 `-d`（管理员域），也就避开了 `root`。
/// `security` 会自己弹一次系统的「是否允许修改证书信任设置」框，这一步必须由用户点。
#[cfg(target_os = "macos")]
pub fn trust_ca() -> Result<()> {
    let (cert_path, _) = ensure_ca()?;
    let path = cert_path.display().to_string();
    let Some(home) = std::env::var_os("HOME") else {
        bail!("读不到 HOME，无法定位登录钥匙串");
    };
    let keychain = PathBuf::from(home).join("Library/Keychains/login.keychain-db");
    if !keychain.exists() {
        bail!("找不到登录钥匙串：{}", keychain.display());
    }
    run_current_user(
        "security",
        &[
            "add-trusted-cert",
            "-r",
            "trustRoot",
            "-k",
            &keychain.display().to_string(),
            &path,
        ],
    )?;
    // 命令成功不代表信任已生效，回读一次实际状态。
    if !ca_trusted() {
        bail!(
            "证书已写入登录钥匙串，但系统仍未信任它。\
             请打开「钥匙串访问」→ 登录 → 证书 → {path} → 双击 → 信任 → 「使用此证书时：始终信任」"
        );
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn trust_ca() -> Result<()> {
    bail!("自动安装证书目前只支持 macOS，请手动把生成的 CA 证书导入系统信任列表")
}

/// CA 是否已被系统信任。
#[cfg(target_os = "macos")]
pub fn ca_trusted() -> bool {
    let Ok((cert_path, _)) = ensure_ca() else {
        return false;
    };
    let out = std::process::Command::new("security")
        .arg("verify-cert")
        .arg("-c")
        .arg(cert_path)
        .output();
    match out {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn ca_trusted() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(uri: &str, cookie: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri(uri);
        if let Some(c) = cookie {
            builder = builder.header("cookie", c);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[test]
    fn captures_uin_and_key_from_wechat_request() {
        let r = req(
            "https://mp.weixin.qq.com/mp/profile_ext?action=home&__biz=Mzg4Mjk3ODcxNA==&uin=MTIzNDU%3D&key=abc123&pass_ticket=pt&appmsg_token=tok",
            Some("wap_sid2=xyz"),
        );
        let cred = credential_from_request(&r).expect("应当抓到");
        assert_eq!(cred.uin, "MTIzNDU=");
        assert_eq!(cred.key, "abc123");
        assert_eq!(cred.pass_ticket, "pt");
        assert_eq!(cred.appmsg_token, "tok");
        assert_eq!(cred.client_cookie, "wap_sid2=xyz");
        assert!(cred.is_complete());
    }

    #[test]
    fn ignores_requests_without_ticket() {
        let r = req("https://mp.weixin.qq.com/s?__biz=abc&mid=1&idx=1", None);
        assert!(credential_from_request(&r).is_none());
    }

    #[test]
    fn ignores_other_hosts() {
        let r = req("https://example.com/?uin=1&key=2", None);
        assert!(credential_from_request(&r).is_none());
    }
}
