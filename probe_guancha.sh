#!/bin/zsh
# 诊断：官察笔记 252 篇原创，程序只拿到 60 篇，缺在哪
UA="Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36"
UA_M="Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49"
URL="https://mp.weixin.qq.com/s/aW42E-SFYkU8qRmJmGZsNQ"

curl -s -A "$UA" -L --max-time 25 "$URL" -o /tmp/gc.html -w 'http=%{http_code} size=%{size_download}\n'

python3 - <<'PY'
import re, json
h=open('/tmp/gc.html',encoding='utf-8',errors='ignore').read()
print('页面长度', len(h))
biz = re.search(r'__biz=([A-Za-z0-9+/=%]+)', h)
print('__biz =', biz.group(1) if biz else None)
n1 = re.search(r'var nickname\s*=\s*["\']([^"\']+)', h)
n2 = re.search(r'data-nickname="([^"]+)"', h)
print('nickname =', (n1.group(1) if n1 else (n2.group(1) if n2 else None)))
a = re.search(r"author_id\s*[:=]\s*['\"]([A-Za-z0-9_\-]{12,})", h)
print('author_id =', a.group(1) if a else None)
albums = sorted(set(re.findall(r"album_id:\s*'(\d+)'", h)))
titles = sorted(set(re.findall(r"title:\s*'([^']{2,40})'", h)))
print('合集 id:', albums)
print('疑似合集标题:', [t for t in titles if '合集' in t or '列' in t][:10])
# 页面里所有 /s/ 文章链接
links = set(re.findall(r'https?://mp\.weixin\.qq\.com/s/[A-Za-z0-9_\-]{10,30}', h))
print('相关文章链接数:', len(links))
PY

BIZ=$(python3 -c "
import re
h=open('/tmp/gc.html',encoding='utf-8',errors='ignore').read()
m=re.search(r'__biz=([A-Za-z0-9+/=%]+)',h)
print(m.group(1) if m else '')
")
echo ""
echo "=== __biz=$BIZ 各通道探测 ==="
for hid in 1 2 3; do
  echo -n "主页栏目 hid=$hid: "
  curl -s -A "$UA_M" "https://mp.weixin.qq.com/mp/homepage?__biz=${BIZ}&hid=${hid}&cid=0&begin=0&count=30&action=appmsg_list&f=json" | head -c 120
  echo ""
done
