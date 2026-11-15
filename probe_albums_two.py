#!/usr/bin/env python3
"""核对两个合集的真实条数与去重后条数。"""
import json, subprocess, urllib.parse

UA = ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 "
      "(KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49")
BIZ = 'MzIwMDY3MTE2NQ=='
ALBUMS = ['4352921130605740036', '4352927384849874960']


def get(url):
    r = subprocess.run(['curl', '-s', '-A', UA, '--max-time', '25', url],
                       capture_output=True, text=True)
    return json.loads(r.stdout.strip().lstrip('\ufeff'))


def album(aid):
    raw, urls, cursor = 0, [], None
    for page in range(40):
        q = {'__biz': BIZ, 'action': 'getalbum', 'album_id': aid, 'count': '20', 'f': 'json'}
        if cursor:
            q['begin_msgid'], q['begin_itemidx'] = cursor
        d = get('https://mp.weixin.qq.com/mp/appmsgalbum?' + urllib.parse.urlencode(q))
        resp = d.get('getalbum_resp') or {}
        arts = resp.get('article_list') or []
        if not isinstance(arts, list):
            print(f'  {aid} 第{page+1}页 article_list 不是 list，是 {type(arts).__name__}，内容前 200 字：')
            print('   ', str(arts)[:200])
            break
        if not arts:
            print(f'  {aid} 第{page+1}页空，停')
            break
        raw += len(arts)
        urls += [a.get('url') for a in arts]
        flag = resp.get('continue_flag')
        if str(flag) in ('0', 'None'):
            print(f'  {aid} 第{page+1}页后 continue_flag={flag}，到底')
            break
        last = arts[-1]
        cursor = (str(last.get('msgid')), str(last.get('itemidx', 1)))
    return raw, urls


all_urls = []
for aid in ALBUMS:
    raw, urls = album(aid)
    print(f'{aid}: 原始 {raw} 条，去重后 {len(set(urls))} 条')
    all_urls += urls
print('两合集并集:', len(set(all_urls)))
