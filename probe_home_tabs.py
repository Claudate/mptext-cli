#!/usr/bin/env python3
"""抓公众号主页（profile_ext action=home），看「全部 / 贴图 / 文章 / 视频」各 tab 的真实条数。"""
import re, subprocess

UA = ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 "
      "(KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49")
BIZ = 'MzIwMDY3MTE2NQ=='

url = (f'https://mp.weixin.qq.com/mp/profile_ext?action=home&__biz={BIZ}'
       '&scene=124&devicetype=iOS17.0&version=18002e2c&lang=zh_CN&nettype=WIFI')

r = subprocess.run(['curl', '-s', '-A', UA, '--max-time', '30', url],
                   capture_output=True, text=True)
html = r.stdout
print('长度:', len(html))

# 主页里通常会带 article_count / 合集列表 / 各 tab 的元信息
for kw in ['252', 'article_count', 'tab', 'home_page_list', 'appmsg_list']:
    idx = html.find(kw)
    print(f'{kw!r} 首次出现位置: {idx}')
    if idx > 0:
        print('  ', html[max(0, idx - 150):idx + 250].replace('\n', ' ')[:400])

# 找所有 title 字段
titles = re.findall(r'"title"\s*:\s*"([^"]{4,60})"', html)
print('\n页面里 title 字段数:', len(titles))
for t in titles[:10]:
    print('  -', t)

# 找 .html 形式的跳转（主页里通常是 appmsg 链接列表）
links = re.findall(r'/s\?__biz=[^"\'\s]{20,160}', html)
print('\n文章链接数:', len(set(links)))
print('示例:', list(set(links))[:2])
