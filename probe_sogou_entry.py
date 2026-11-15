"""搜狗移动版「账号」搜索 → 公众号主页链接。

关键：搜狗给出的 link?url= 是签名的，302 之后落地页 URL 里自带
key / uin / pass_ticket —— 不需要微信登录就能打开公众号主页。
"""
import re
from urllib.parse import quote

from curl_cffi import requests

MOBILE = ("Mozilla/5.0 (Linux; Android 13; SM-G991B) AppleWebKit/537.36 (KHTML, like Gecko) "
          "Chrome/120.0.0.0 Mobile Safari/537.36")

s = requests.Session(impersonate="chrome131")
s.headers.update({"User-Agent": MOBILE, "Accept-Language": "zh-CN,zh;q=0.9"})

name = "英国那些事儿"
r = s.get("https://weixin.sogou.com/weixinwap", params={"type": "1", "query": name},
          timeout=25)
h = r.text
print("账号页", r.status_code, len(h), "验证码:", "antispider" in r.url)
links = re.findall(r'href="([^"]*(?:/link\?url=|mp\.weixin)[^"]*)"', h)
print("候选链接", len(links))
for x in links[:5]:
    print("  ", x[:150])

# 直接找页面里出现的账号名与链接关系
for m in re.finditer(r"<a[^>]+href=\"([^\"]+)\"[^>]*>(.{0,120}?)</a>", h, re.S):
    href, inner = m.group(1), re.sub(r"<[^>]+>", "", m.group(2))
    inner = inner.strip()
    if name in inner or "link?url=" in href:
        print("  A:", inner[:40], "->", href[:130])

print("\n--- 跟进第一个账号链接 ---")
if links:
    u = links[0]
    if u.startswith("/"):
        u = "https://weixin.sogou.com" + u
    u = u.replace("&amp;", "&")
    rr = s.get(u, timeout=25, allow_redirects=True)
    print("落地:", rr.status_code, len(rr.text), rr.url[:400])
    if "__biz" in rr.text:
        m = re.search(r'__biz=([A-Za-z0-9+/=%]+)', rr.text)
        print("__biz:", m.group(1) if m else "-")
    for frag in re.findall(r'(https?://mp\.weixin\.qq\.com[^"\'\\\s<>]{0,260})', rr.text)[:6]:
        print("   frag:", frag[:280])
