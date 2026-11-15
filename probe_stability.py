#!/usr/bin/env python3
"""同一篇文章页连抓 5 次，看 album_id 出现是否稳定。
若不稳定，说明腾讯对文章页的合集模块做了概率化下发，
'扫种子反查' 这种一次性扫描天然会漏。"""
import re, subprocess

UA = ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 "
      "(KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49")
BIZ = 'MzIwMDY3MTE2NQ=='
URLS = [
    ('第 12 篇', 'https://mp.weixin.qq.com/s?__biz=MzIwMDY3MTE2NQ==&mid=2247490837&idx=1&sn=0'),
    ('第 106 篇', 'https://mp.weixin.qq.com/s?__biz=MzIwMDY3MTE2NQ==&mid=2247491753&idx=1&sn=0'),
    ('第 158 篇', 'https://mp.weixin.qq.com/s?__biz=MzIwMDY3MTE2NQ==&mid=2247496552&idx=1&sn=0'),
]


def ids(html):
    out = set()
    for pat in (r"appmsgalbum\?__biz=[^\"'<>\s]*?album_id=(\d{6,25})",
                r'"album_id"\s*:\s*"(\d{6,25})"',
                r"album_id\s*:\s*['\"]?(\d{6,25})['\"]?"):
        out |= set(re.findall(pat, html))
    return out


def fetch(url):
    r = subprocess.run(['curl', '-s', '-L', '-A', UA, '--max-time', '25', url],
                       capture_output=True, text=True)
    return r.stdout


for name, u in URLS:
    print(f'--- {name} ---')
    for i in range(5):
        h = fetch(u)
        print(f'  第{i+1}次: len={len(h):6d} album_ids={sorted(ids(h))}')
