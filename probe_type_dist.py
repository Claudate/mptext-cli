#!/usr/bin/env python3
"""统计两个合集里 item_show_type 的分布——即「贴图 / 文章 / 视频」各占多少。"""
import collections, json, subprocess, urllib.parse

UA = ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 "
      "(KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49")
BIZ = 'MzIwMDY3MTE2NQ=='
ALBUMS = ['4352921130605740036', '4352927384849874960']


def get(url):
    r = subprocess.run(['curl', '-s', '-A', UA, '--max-time', '25', url],
                       capture_output=True, text=True)
    return json.loads(r.stdout.strip().lstrip('\ufeff'))


def album(aid):
    items, cursor = [], None
    for _ in range(40):
        q = {'__biz': BIZ, 'action': 'getalbum', 'album_id': aid, 'count': '20', 'f': 'json'}
        if cursor:
            q['begin_msgid'], q['begin_itemidx'] = cursor
        resp = get('https://mp.weixin.qq.com/mp/appmsgalbum?' + urllib.parse.urlencode(q)).get('getalbum_resp') or {}
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


seen = {}
for aid in ALBUMS:
    for it in album(aid):
        u = it.get('url')
        if u:
            seen[u] = it

print(f'两合集去重后 {len(seen)} 篇\n')
c = collections.Counter(it.get('item_show_type') for it in seen.values())
print('item_show_type 分布:', dict(c))
print('\n按类型举例：')
by = collections.defaultdict(list)
for it in seen.values():
    by[it.get('item_show_type')].append((it.get('title', '')[:40], it.get('url', '')[:60]))
for k, v in by.items():
    print(f'\n  type={k} 共 {len(v)} 篇')
    for t, u in v[:3]:
        print('   -', t)
