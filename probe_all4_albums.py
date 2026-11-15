#!/usr/bin/env python3
"""量全部 4 个合集的篇数与 item_show_type，给出真实的免登录覆盖。"""
import collections, json, subprocess, urllib.parse

UA = ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 "
      "(KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49")
BIZ = 'MzIwMDY3MTE2NQ=='
ALBUMS = ['4352921130605740036', '4352927384849874960',
          '4352914447787622403', '4352899587368550401']


def curl(url):
    r = subprocess.run(['curl', '-s', '-A', UA, '--max-time', '25', url],
                       capture_output=True, text=True)
    return json.loads(r.stdout.strip().lstrip('\ufeff'))


def album(aid):
    items, cursor = [], None
    for _ in range(60):
        q = {'__biz': BIZ, 'action': 'getalbum', 'album_id': aid, 'count': '20', 'f': 'json'}
        if cursor:
            q['begin_msgid'], q['begin_itemidx'] = cursor
        resp = curl('https://mp.weixin.qq.com/mp/appmsgalbum?' + urllib.parse.urlencode(q)).get('getalbum_resp') or {}
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
    its = album(aid)
    print(f'{aid}: {len(its)} 篇')
    for it in its:
        u = it.get('url')
        if u:
            seen.setdefault(u, it)

print(f'\n4 个合集去重后: {len(seen)} 篇')
c = collections.Counter(it.get('item_show_type') for it in seen.values())
print('item_show_type 分布:', dict(c))

# 时间跨度
ts = sorted(int(it['create_time']) for it in seen.values() if it.get('create_time'))
import datetime
if ts:
    fmt = lambda x: datetime.datetime.fromtimestamp(x).strftime('%Y-%m-%d')
    print(f'时间跨度: {fmt(ts[0])} ~ {fmt(ts[-1])}')
