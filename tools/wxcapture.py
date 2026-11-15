"""把微信内置浏览器经代理发出的 mp.weixin.qq.com 请求原样落盘。

为什么需要它：`profile_ext?action=getmsg` 拿 URL 里的 uin/key 不够，服务端
要的是微信网页会话 cookie（wap_sid2 一类），而这类 cookie 只活在微信
XWeb 进程的内存里、不落磁盘。好在 XWeb 是 Chromium 网络栈，会走系统代理，
所以在代理这一层就能把 Cookie 头与响应体完整截下来。

用法：
    mitmdump -s tools/wxcapture.py -p 8080 --set block_global=false

输出：~/.config/mptext/capture/<时间戳>-<序号>.json
      每条含 url / cookie / 请求方法 / 响应状态 / 响应体前 400KB。
"""
import json
import os
import time

from mitmproxy import http

OUT = os.path.expanduser("~/.config/mptext/capture")
os.makedirs(OUT, exist_ok=True)

_seq = 0
KEEP_HOSTS = ("mp.weixin.qq.com",)


def _want(flow: http.HTTPFlow) -> bool:
    host = flow.request.pretty_host
    return any(host == h or host.endswith("." + h) for h in KEEP_HOSTS)


def response(flow: http.HTTPFlow) -> None:
    """只落 mp.weixin.qq.com 的响应；其余流量直接放过。"""
    global _seq
    if not _want(flow):
        return
    _seq += 1
    body = flow.response.get_text(strict=False) or ""
    rec = {
        "ts": time.strftime("%Y-%m-%d %H:%M:%S"),
        "method": flow.request.method,
        "url": flow.request.pretty_url,
        "status": flow.response.status_code,
        "req_headers": dict(flow.request.headers),
        "resp_headers": dict(flow.response.headers),
        # Cookie 是这条链路上唯一不可再生的东西，单独留一份
        "cookie": flow.request.headers.get("cookie", ""),
        "body_head": body[:400000],
        "body_len": len(body),
    }
    name = f"{time.strftime('%H%M%S')}-{_seq:04d}.json"
    path = os.path.join(OUT, name)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(rec, f, ensure_ascii=False)
    os.replace(tmp, path)
    print(f"[wxcapture] {flow.response.status_code} {len(body):>7}B {flow.request.pretty_url[:120]}")
