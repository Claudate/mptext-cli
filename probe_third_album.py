#!/usr/bin/env python3
"""用两个合集里全部 158 篇当种子，逐篇反查合集 id，看还有没有第三个合集。"""
import json, re, subprocess, urllib.parse

UA_M = ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 "
        "(KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49")
BIZ = 'MzIwMDY3MTE2NQ=='
ALBUMS = ['4352921130605740036', '4352927384849874960']


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
print(f'种子 {len(urls)} 篇，逐篇反查合集 id…')

found = set(ALBUMS)
new = {}
for i, u in enumerate(urls, 1):
    html = curl(u)
    ids = set(re.findall(r"album_id\s*[:=]\s*['\"]?(\d{6,25})", html))
    ids |= set(re.findall(r"appmsgalbum\?__biz=[^\"'<>\s]*?album_id=(\d{6,25})", html))
    for x in ids - found:
        found.add(x)
        new[x] = u
    if i % 25 == 0:
        print(f'  {i}/{len(urls)}  当前合集 {sorted(found)}')

print(f'\n共发现合集 {len(found)} 个: {sorted(found)}')
if new:
    print('新增合集来源:')
    for aid, src in new.items():
        print(f'  {aid} <- {src[:80]}')
else:
    print('没有新合集：两个合集已覆盖免登录通道能拿到的全部文章。')
