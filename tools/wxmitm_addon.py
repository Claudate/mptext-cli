"""mptext 票据捕获 —— mitmdump addon。

抓 `mp.weixin.qq.com` 的会话，把能翻全量历史的票据原子写进
`~/.config/mptext/wxticket.json`。两条路径，被动优先：

1. **被动**：微信自己发出了 `profile_ext?action=home|getmsg`，URL 里就带
   `uin`/`key`/`pass_ticket`，直接照抄——这份票据与当前会话严格匹配，最可靠。
2. **主动**：微信只发出了别的 `mp.weixin.qq.com` 请求（比如打开一篇文章），
   但 Cookie 里已有 `wap_sid` 登录态。此时用这份 Cookie 主动请求一次
   `profile_ext?action=home`，从返回页面里正则提取凭据。

第 2 条是为了省掉「必须点进公众号历史页」这个动作：只要微信打开过任意一个
公众号页面，票据就能自动落盘。控制台输出一律掩码，不打印完整票据。

用法:
    mitmdump -s tools/wxmitm_addon.py -p 8080 --set block_global=false
"""

import json
import os
import re
import time
import urllib.parse
import urllib.request

from mitmproxy import ctx, http

OUT = os.path.expanduser("~/.config/mptext/wxticket.json")
MP = "https://mp.weixin.qq.com"

# 主动换票要伪装成微信内置浏览器（XWeb/CEF），否则 profile_ext 会回
# 「请在微信客户端打开链接」的 2KB 错误页。
WX_UA = (
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 "
    "MicroMessenger/8.0.50 NetType/WIFI Language/zh_CN"
)

# 页面里凭据有几种写法（实测都出现过），按优先级依次尝试。
RE_KEY = re.compile(r'var\s+key\s*=\s*"([^"]+)"')
RE_UIN = re.compile(r'var\s+uin\s*=\s*"([^"]*)"')
RE_PASS = re.compile(r'pass_ticket[=":\s]+([A-Za-z0-9%_\-]{8,})')
RE_TOKEN = re.compile(r'appmsg_token\s*=\s*"([^"]+)"')
RE_BIZ = re.compile(r"__biz=([A-Za-z0-9+/=]+)")

# 微信网页会话里代表「已登录」的 cookie 名。实测 macOS 版实际发的是
# `wap_sid2`（不是老的 `wap_sid`），且键名精确匹配、不做前缀判断。
SESSION_COOKIE_KEYS = (
    "wap_sid", "wap_sid2", "wxuin", "uin", "sessionid",
    "session_us", "appmsg_token", "pass_ticket", "slave_sid",
)


def _normalize_cookie(raw: str) -> str:
    """微信 XWeb 发出的 Cookie 头用 `, ` 分隔，而 HTTP 服务端只认 `; `。

    直接转发原文会让 mp.weixin 回 `{"ret":-3,"cookie_count":0}`——
    服务端一个 cookie 都没解析到。这里统一转成分号分隔。
    """
    if ";" in raw and "," not in raw:
        return raw
    parts = [p.strip() for p in raw.replace(";", ",").split(",") if "=" in p]
    return "; ".join(parts)

# 换票每个 __biz 只做一次，失败不重试——避免给微信接口刷请求。
_tried: set[str] = set()


def _mask(s: str) -> str:
    return (s[:12] + "...") if len(s) > 12 else s


def _save(url: str, cookie: str, how: str) -> None:
    """原子写票据，进程被中断也不会留下半截文件。"""
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    data = {
        "captured_at": time.strftime("%Y-%m-%d %H:%M:%S"),
        "url": url,
        "cookie": cookie,
    }
    tmp = OUT + ".tmp"
    with open(tmp, "w") as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    os.replace(tmp, OUT)
    pt = url.split("pass_ticket=")[-1].split("&")[0] if "pass_ticket=" in url else "-"
    print(
        f"[mptext] 票据已写入（{how}） cookie长度={len(cookie)} "
        f"pass_ticket={_mask(pt)}",
        flush=True,
    )


def _build_ticket_url(html: str, biz: str) -> str | None:
    """从 profile_ext 返回页面里抽出翻页凭据，拼成可直接复用的 URL。"""
    mk, mu = RE_KEY.search(html), RE_UIN.search(html)
    if not (mk and mu and mk.group(1) and mu.group(1)):
        return None
    q = urllib.parse.quote
    url = (
        f"{MP}/mp/profile_ext?action=home&__biz={biz}"
        f"&scene=124&is_ok=1&uin={mu.group(1)}&key={q(mk.group(1), safe='')}"
    )
    if m := RE_PASS.search(html):
        url += f"&pass_ticket={q(m.group(1), safe='')}"
    if m := RE_TOKEN.search(html):
        url += f"&appmsg_token={q(m.group(1), safe='')}"
    return url


def _exchange(cookie: str, biz: str) -> None:
    """拿微信会话 Cookie 主动请求 profile_ext，拿到 key 就落盘。"""
    if biz in _tried:
        return
    _tried.add(biz)

    probe = (
        f"{MP}/mp/profile_ext?action=home&__biz={biz}"
        f"&scene=124&is_ok=1&devicetype=Mac&version=v4.1.62&lang=zh_CN"
    )
    req = urllib.request.Request(
        probe, headers={"Cookie": cookie, "User-Agent": WX_UA, "Referer": f"{MP}/"}
    )
    # 必须绕开代理：mitmdump 自己就监听 8080，经代理回环会死锁。
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with opener.open(req, timeout=20) as r:
            html = r.read().decode("utf-8", "replace")
    except Exception as e:  # 换票失败不该影响微信正常上网
        print(f"[mptext] 换票请求异常：{e}", flush=True)
        return

    if ticket := _build_ticket_url(html, biz):
        _save(ticket, _normalize_cookie(cookie), "主动换票")
    else:
        hint = "会话未登录/已失效" if "请在微信客户端打开" in html else "页面里没有 key"
        print(f"[mptext] 换票失败（{hint}，{len(html)}B）biz={biz}", flush=True)


def response(flow: http.HTTPFlow) -> None:
    """被动优先、主动兜底。"""
    host = flow.request.pretty_host
    if not (host == "mp.weixin.qq.com" or host.endswith(".mp.weixin.qq.com")):
        return

    url = flow.request.pretty_url
    cookie = flow.request.headers.get("Cookie", "")

    # 路径一：微信自己发出了带 key 的历史列表请求，直接采用。
    if "profile_ext" in url and ("action=home" in url or "action=getmsg" in url):
        if "key=" in url:
            _save(url, _normalize_cookie(cookie), "微信原生请求")
        return

    # 路径二：Cookie 必须是登录态才值得换票。
    # 新版微信不一定用 wap_sid 这个名字，所以先看键名里有没有会话类字段。
    names = {kv.split("=", 1)[0].strip() for kv in _normalize_cookie(cookie).split(";") if "=" in kv}
    if not (names & set(SESSION_COOKIE_KEYS)):
        return

    biz = m.group(1) if (m := RE_BIZ.search(url)) else "Mzg2OTg5NDg3Mg=="
    # 只打键名，不打值——用来确认新版微信会话 cookie 到底叫什么。
    print(f"[mptext] 命中 mp.weixin 请求，cookie 键={sorted(names)}", flush=True)
    _exchange(cookie, biz)
