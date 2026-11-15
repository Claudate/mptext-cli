#!/usr/bin/env python3
"""合集 4352921130605740036 翻到底，看官察笔记能拿多少篇。"""
import json, subprocess, urllib.parse

UA_M = ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 "
        "(KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49")
BIZ = 'MzIwMDY3MTE2NQ=='
ALBUM = '4352921130605740036'


def fetch(cursor=None):
    q = {'__biz': BIZ, 'action': 'getalbum', 'album_id': ALBUM, 'count': '20', 'f': 'json'}
    if cursor:
        q['begin_msgid'], q['begin_itemidx'] = cursor
    url = 'https://mp.weixin.qq.com/mp/appmsgalbum?' + urllib.parse.urlencode(q)
    r = subprocess.run(['curl', '-s', '-A', UA_M, '--max-time', '25', url],
                       capture_output=True, text=True)
    return json.loads(r.stdout.strip().lstrip('\ufeff'))


total = 0
seen = set()
cursor = None
for page in range(40):
    d = fetch(cursor)
    resp = d.get('getalbum_resp') or {}
    arts = resp.get('article_list') or []
    n = len(arts)
    total += n
    flag = resp.get('continue_flag')
    print(f'  第{page+1}页 本页={n} 累计={total} continue_flag={flag}')
    if n == 0:
        break
    for a in arts[:2]:
        seen.add(a.get('title', '')[:30])
    if flag in (0, '0'):
        print('  到底了')
        break
    last = arts[-1]
    cursor = (str(last.get('msgid')), str(last.get('itemidx', 1)))

print('合集合计:', total)
print('样例标题:', list(seen)[:6])
