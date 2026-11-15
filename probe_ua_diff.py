#!/usr/bin/env python3
"""对照：同一篇文章页，Mac Chrome UA 与 iPhone 微信 UA 抓到的 album_id 是否不同。
用来确认 Rust 侧 list_all_albums 第二轮漏合集，是不是 public_html 的 UA 造成的。"""
import json, re, subprocess, urllib.parse

UA_MAC = ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 "
          "(KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36")
UA_WX = ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 "
         "(KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49")
BIZ = 'MzIwMDY3MTE2NQ=='
ALBUMS = ['4352921130605740036', '4352927384849874960']


def curl(url, ua, as_json=False):
    r = subprocess.run(['curl', '-s', '-L', '-A', ua, '--max-time', '30', url],
                       capture_output=True, text=True)
    t = r.stdout.strip().lstrip('\ufeff')
    return json.loads(t) if as_json else t


def album_ids(html):
    out = set()
    for pat in (r"appmsgalbum\?__biz=[^\"'<>\s]*?album_id=(\d{6,25})",
                r'"album_id"\s*:\s*"(\d{6,25})"',
                r"album_id\s*:\s*['\"]?(\d{6,25})['\"]?"):
        out |= set(re.findall(pat, html))
    return out


def album(aid):
    items, cursor = [], None
    for _ in range(40):
        q = {'__biz': BIZ, 'action': 'getalbum', 'album_id': aid, 'count': '20', 'f': 'json'}
        if cursor:
            q['begin_msgid'], q['begin_itemidx'] = cursor
        resp = curl('https://mp.weixin.qq.com/mp/appmsgalbum?' + urllib.parse.urlencode(q),
                    UA_WX, as_json=True).get('getalbum_resp') or {}
        arts = resp.get('article_list')
        if isinstance(arts, dict):
            arts = [arts]
        if not arts:
            break
        items += arts
        if str(resp.get('continue_flag')) in ('0', 'None'):
            break
        last = arts[-1]
        cursor = (str(last.get('msgid')), str(last.get('itemidx', 1)))
    return items


urls = []
for aid in ALBUMS:
    urls += [it['url'] for it in album(aid) if it.get('url')]
urls = list(dict.fromkeys(urls))
print(f'A+B 去重 {len(urls)} 篇\n')

CHECK = [12, 95, 96, 106, 112, 125, 140, 158]
for i in CHECK:
    if i > len(urls):
        continue
    u = urls[i - 1]
    mid = re.search(r'mid=(\d+)', u)
    mac = album_ids(curl(u, UA_MAC))
    wx = album_ids(curl(u, UA_WX))
    print(f'第 {i:3d} 篇 mid={mid.group(1) if mid else "?"}')
    print(f'   mac UA: {sorted(mac)}')
    print(f'   wx  UA: {sorted(wx)}')
    print(f'   差异: wx 多出 {sorted(wx - mac)}')
