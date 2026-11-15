#!/usr/bin/env python3
"""主页栏目失效后，测所有候选通道能否列出官察笔记的文章。"""
import json, os, subprocess, urllib.parse

S = json.load(open(os.path.expanduser('~/.config/mptext/session.json')))
TOKEN = S.get('token', '')
CK = '; '.join(f'{k}={v}' for k, v in (S.get('cookies') or {}).items())
UA_WEB = ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 "
          "(KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36")
UA_M = ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 "
        "(KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49")

BIZ = 'MzIwMDY3MTE2NQ=='
AUTHOR = 'ofMoI46DZYm9TknSUIgWfAVEBXMQ'
ALBUM = '4352921130605740036'


def get(url, ua=UA_WEB, cookie=None, referer=None):
    cmd = ['curl', '-s', '-A', ua, '--max-time', '25']
    if cookie:
        cmd += ['-H', f'Cookie: {cookie}']
    if referer:
        cmd += ['-H', f'Referer: {referer}']
    cmd.append(url)
    r = subprocess.run(cmd, capture_output=True, text=True)
    return r.stdout.strip().lstrip('\ufeff')


def head(label, text, n=220):
    print(f'  {label}: {text[:n]}')


print('=== A. getalbumlist 带后台 cookie ===')
u = 'https://mp.weixin.qq.com/mp/appmsgalbum?' + urllib.parse.urlencode(
    {'action': 'getalbumlist', '__biz': BIZ, 'begin': '0', 'count': '20', 'f': 'json'})
head('带cookie', get(u, cookie=CK))

print('\n=== B. mp/author get_articles 带后台 cookie ===')
u = 'https://mp.weixin.qq.com/mp/author?' + urllib.parse.urlencode(
    {'action': 'get_articles', 'author_id': AUTHOR, '__biz': BIZ,
     'scene': '142', 'limit': '30', 'f': 'json'})
head('带cookie', get(u, cookie=CK, referer=f'https://mp.weixin.qq.com/mp/author?action=show&__biz={BIZ}&author_id={AUTHOR}'))

print('\n=== C. profile_ext getmsg 带后台 cookie ===')
u = 'https://mp.weixin.qq.com/mp/profile_ext?' + urllib.parse.urlencode(
    {'action': 'getmsg', '__biz': BIZ, 'f': 'json', 'offset': '0', 'count': '10', 'scene': '124'})
head('带cookie', get(u, ua=UA_M, cookie=CK))

print('\n=== D. homepage 带后台 cookie ===')
u = 'https://mp.weixin.qq.com/mp/homepage?' + urllib.parse.urlencode(
    {'__biz': BIZ, 'hid': '1', 'cid': '0', 'begin': '0', 'count': '30',
     'action': 'appmsg_list', 'f': 'json'})
head('带cookie', get(u, ua=UA_M, cookie=CK))

print('\n=== E. appmsgalbum 合集内容（无 cookie，看真实条数）===')
u = 'https://mp.weixin.qq.com/mp/appmsgalbum?' + urllib.parse.urlencode(
    {'__biz': BIZ, 'action': 'getalbum', 'album_id': ALBUM, 'count': '20', 'f': 'json'})
r = get(u, ua=UA_M)
head('合集', r, 300)

print('\n=== F. 后台 appmsgpublish 查自己号（对照 200013 是什么）===')
u = 'https://mp.weixin.qq.com/cgi-bin/appmsgpublish?' + urllib.parse.urlencode(
    {'sub': 'list', 'search_field': 'null', 'begin': '0', 'count': '5', 'query': '',
     'fakeid': '3903745961', 'type': '101_1', 'free_publish_type': '1',
     'sub_action': 'list_ex', 'token': TOKEN, 'lang': 'zh_CN', 'f': 'json', 'ajax': '1'})
head('自己号', get(u, cookie=CK), 300)
