#!/usr/bin/env python3
"""直接用 session.json 打后台 appmsgpublish，看 60 篇的真实来源与上限。"""
import json, os, subprocess, urllib.parse

s = json.load(open(os.path.expanduser('~/.config/mptext/session.json')))
token = s.get('token', '')
cookies = '; '.join(f'{k}={v}' for k, v in (s.get('cookies') or {}).items())
UA = ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36")


def call(fakeid, begin, count=20, typ='101_1'):
    q = {
        'sub': 'list', 'search_field': 'null', 'begin': str(begin), 'count': str(count),
        'query': '', 'fakeid': fakeid, 'type': typ, 'free_publish_type': '1',
        'sub_action': 'list_ex', 'token': token, 'lang': 'zh_CN', 'f': 'json', 'ajax': '1',
    }
    url = 'https://mp.weixin.qq.com/cgi-bin/appmsgpublish?' + urllib.parse.urlencode(q)
    r = subprocess.run(['curl', '-s', '-A', UA, '-H', f'Cookie: {cookies}',
                        '--max-time', '25', url], capture_output=True, text=True)
    try:
        return json.loads(r.stdout.strip().lstrip('\ufeff'))
    except Exception:
        return {'_raw': r.stdout[:200]}


for fakeid, name in [('MzIwMDY3MTE2NQ==', '官察笔记'), ('MzkwMzUxNDE3NA==', '观察笔记')]:
    print(f'=== {name} {fakeid} ===')
    d = call(fakeid, 0)
    br = d.get('base_resp', {})
    print('  base_resp =', br)
    if br.get('ret') != 0:
        print('  被拒:', br.get('err_msg'), '| raw:', str(d.get('_raw', ''))[:120])
        continue
    pp = d.get('publish_page')
    if isinstance(pp, str):
        pp = json.loads(pp)
    print('  total_count =', pp.get('total_count'))
    lst = pp.get('publish_list') or []
    print('  本页发布条数 =', len(lst))
    for item in lst[:3]:
        info = item.get('publish_info')
        if isinstance(info, str):
            try:
                info = json.loads(info)
            except Exception:
                pass
        if isinstance(info, dict):
            msgs = info.get('appmsgex') or []
            print('    -', (msgs[0].get('title', '')[:40] if msgs else '?'))
