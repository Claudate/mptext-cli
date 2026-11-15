#!/usr/bin/env python3
"""定位：新发现的合集 4352914447787622403 / 4352899587368550401
分别由哪篇文章带出，以及那篇文章在合集 A/B 里的位置。"""
import json, re, subprocess, urllib.parse

UA_M = ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 "
        "(KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49")
BIZ = 'MzIwMDY3MTE2NQ=='
ALBUMS = ['4352921130605740036', '4352927384849874960']
TARGETS = {'4352914447787622403', '4352899587368550401'}


def curl(url, ua=UA_M, as_json=False):
    r = subprocess.run(['curl', '-s', '-L', '-A', ua, '--max-time', '30', url],
                       capture_output=True, text=True)
    t = r.stdout.strip().lstrip('\ufeff')
    return json.loads(t) if as_json else t


def album(aid):
    items, cursor = [], None
    for _ in range(40):
        q = {'__biz': BIZ, 'action': 'getalbum', 'album_id': aid, 'count': '20', 'f': 'json'}
        if cursor:
            q['begin_msgid'], q['begin_itemidx'] = cursor
        resp = curl('https://mp.weixin.qq.com/mp/appmsgalbum?' + urllib.parse.urlencode(q), as_json=True).get('getalbum_resp') or {}
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
print(f'A+B 去重 {len(urls)} 篇')

for i, u in enumerate(urls, 1):
    html = curl(u)
    ids = set(re.findall(r"album_id\s*[:=]\s*['\"]?(\d{6,25})", html))
    ids |= set(re.findall(r"appmsgalbum\?__biz=[^\"'<>\s]*?album_id=(\d{6,25})", html))
    hit = ids & TARGETS
    if hit:
        mid = re.search(r'mid=(\d+)', u)
        print(f'  第 {i} 篇 (mid={mid.group(1) if mid else "?"}) 带出 {hit}')
