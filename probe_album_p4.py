#!/usr/bin/env python3
"""dump 合集 4352921130605740036 第 4 页的完整结构，找剩余文章在哪。"""
import json, subprocess, urllib.parse

UA = ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 "
      "(KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49")
BIZ = 'MzIwMDY3MTE2NQ=='
ALBUM = '4352921130605740036'


def get(url):
    r = subprocess.run(['curl', '-s', '-A', UA, '--max-time', '25', url],
                       capture_output=True, text=True)
    return json.loads(r.stdout.strip().lstrip('\ufeff'))


cursor = None
for page in range(1, 5):
    q = {'__biz': BIZ, 'action': 'getalbum', 'album_id': ALBUM, 'count': '20', 'f': 'json'}
    if cursor:
        q['begin_msgid'], q['begin_itemidx'] = cursor
    d = get('https://mp.weixin.qq.com/mp/appmsgalbum?' + urllib.parse.urlencode(q))
    resp = d.get('getalbum_resp') or {}
    arts = resp.get('article_list')
    print(f'--- 第{page}页 ---')
    print('  resp keys:', list(resp.keys()))
    print('  article_list type:', type(arts).__name__)
    if isinstance(arts, list):
        print('  条数:', len(arts))
        last = arts[-1]
        cursor = (str(last.get('msgid')), str(last.get('itemidx', 1)))
        print('  continue_flag:', resp.get('continue_flag'), 'cursor:', cursor)
    else:
        print('  dict keys:', list(arts.keys()) if isinstance(arts, dict) else arts)
        print('  完整内容:', json.dumps(arts, ensure_ascii=False)[:1200])
        print('  顶层 getalbum_resp 其它字段:',
              {k: (str(v)[:80] if not isinstance(v, (list, dict)) else f'<{type(v).__name__}>')
               for k, v in resp.items() if k != 'article_list'})
        break
