#!/usr/bin/env python3
"""探 /mp/homepage 是否还活着：拿几个已知号做对照。"""
import subprocess, urllib.parse, json

UA_M = ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 "
        "(KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49")


def page(biz, hid, count=30):
    q = {'__biz': biz, 'hid': str(hid), 'cid': '0', 'begin': '0',
         'count': str(count), 'action': 'appmsg_list', 'f': 'json'}
    url = 'https://mp.weixin.qq.com/mp/homepage?' + urllib.parse.urlencode(q)
    r = subprocess.run(['curl', '-s', '-A', UA_M, '--max-time', '20', url],
                       capture_output=True, text=True)
    try:
        return json.loads(r.stdout.strip().lstrip('\ufeff'))
    except Exception:
        return {'_raw': r.stdout[:120]}


# 记忆里明确有栏目的号 + 本次的号
cases = [
    ('MzAxMzg0MzMxNg==', '良辰美（记忆：15 个栏目）'),
    ('Mzg4Mjk3ODcxNA==', '长风的储物柜'),
    ('MzIwMDY3MTE2NQ==', '官察笔记'),
    ('MzU1NzU0NDM5Mg==', '胡锡进观察（记忆：74 篇栏目）'),
]

for biz, name in cases:
    print(f'=== {name} {biz} ===')
    for hid in range(1, 6):
        d = page(biz, hid)
        br = d.get('base_resp', {})
        n = len(d.get('appmsg_list') or [])
        print(f'  hid={hid} ret={br.get("ret")} 条数={n}')
        if br.get('ret') != 0 and hid == 1:
            break
