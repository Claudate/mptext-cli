#!/bin/zsh
# 用真实票据 + 后台会话分别试 /mp/author，看能否突破 60 篇
BIZ="MzIwMDY3MTE2NQ=="
AUTHOR="ofMoI46DZYm9TknSUIgWfAVEBXMQ"
UA="Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36"

UIN=$(python3 -c "import json,os;d=json.load(open(os.path.expanduser('~/.config/mptext/wxcapture.json')));print(d['$BIZ']['uin'])")
KEY=$(python3 -c "import json,os;d=json.load(open(os.path.expanduser('~/.config/mptext/wxcapture.json')));print(d['$BIZ']['key'])")
PT=$(python3 -c "import json,os;d=json.load(open(os.path.expanduser('~/.config/mptext/wxcapture.json')));print(d['$BIZ']['pass_ticket'])")
TOK=$(python3 -c "import json,os;d=json.load(open(os.path.expanduser('~/.config/mptext/wxcapture.json')));print(d['$BIZ']['appmsg_token'])")

SHOW="https://mp.weixin.qq.com/mp/author?action=show&__biz=${BIZ}&idx=1&scene=142&rscene=128&uin=${UIN}&key=${KEY}&devicetype=UnifiedPCMac&version=f2640619&lang=zh_CN&ascene=1&acctmode=0&pass_ticket=${PT}&countrycode=CN&author_id=${AUTHOR}"

echo "=== 1. action=show 换 Cookie ==="
curl -s -A "$UA" -D /tmp/hdr.txt -o /tmp/show.html --max-time 25 "$SHOW"
grep -iE "^(HTTP/|set-cookie:)" /tmp/hdr.txt | head -10
echo "--- show 页面内容判定 ---"
python3 - <<'PY'
import re
h=open('/tmp/show.html',encoding='utf-8',errors='ignore').read()
print('长度', len(h))
m=re.search(r'<title[^>]*>(.*?)</title>',h,re.S)
print('title =', repr(m.group(1)[:60]) if m else None)
for kw in ['验证','环境异常','no session','请在微信客户端打开','参数错误']:
    if kw in h: print('!! 命中', kw)
PY

echo ""
echo "=== 2. 直接用票据请求 get_articles（不带 cookie）==="
curl -s -A "$UA" -G "https://mp.weixin.qq.com/mp/author" \
  --data-urlencode "action=get_articles" \
  --data-urlencode "author_id=${AUTHOR}" \
  --data-urlencode "__biz=${BIZ}" \
  --data-urlencode "uin=${UIN}" \
  --data-urlencode "key=${KEY}" \
  --data-urlencode "pass_ticket=${PT}" \
  --data-urlencode "appmsg_token=${TOK}" \
  --data-urlencode "scene=142" \
  --data-urlencode "limit=30" \
  --data-urlencode "f=json" \
  -H "Referer: $SHOW" \
  --max-time 25 | head -c 400
echo ""

echo ""
echo "=== 3. 后台会话现有能力（session.json）==="
python3 - <<'PY'
import json,os,time
p=os.path.expanduser('~/.config/mptext/session.json')
d=json.load(open(p))
print('nickname =', d.get('nickname'))
print('token len =', len(d.get('token','')))
print('cookies =', list((d.get('cookies') or {}).keys())[:12])
PY
