#!/bin/zsh
# 拆解 60 篇的构成，并测试合集能翻到多少
UA_M="Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49"
BIZ="MzIwMDY3MTE2NQ=="
ALBUM="4352921130605740036"

echo "=== 合集 ${ALBUM} 翻页实测 ==="
python3 - <<'PY'
import json, subprocess, urllib.parse, os
BIZ="MzIwMDY3MTE2NQ=="
ALBUM="4352921130605740036"
UA="Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.49"
cursor=None
total=0
for page in range(30):
    q={'__biz':BIZ,'action':'getalbum','album_id':ALBUM,'count':'20','f':'json'}
    if cursor:
        q['begin_msgid'],q['begin_itemidx']=cursor
    url='https://mp.weixin.qq.com/mp/appmsgalbum?'+urllib.parse.urlencode(q)
    r=subprocess.run(['curl','-s','-A',UA,'--max-time','25',url],capture_output=True,text=True)
    try:
        d=json.loads(r.stdout.strip().lstrip('\ufeff'))
    except Exception as e:
        print(f'  第{page+1}页 解析失败:', r.stdout[:150]); break
    ret=d.get('base_resp',{}).get('ret')
    arts=d.get('article_list') or []
    if isinstance(arts,dict): arts=arts.get('list') or []
    n=len(arts)
    total+=n
    print(f'  第{page+1}页 ret={ret} 本页={n} 累计={total}')
    if n==0: break
    if not d.get('continue_flag') in (1,'1'): 
        print('  continue_flag 结束'); break
    last=arts[-1]
    cursor=(str(last.get('msgid')), str(last.get('itemidx',1)))
print('合集合计:', total)
PY

echo ""
echo "=== 搜狗索引 官察笔记 ==="
curl -s -A "$UA_M" "https://weixin.sogou.com/weixinwap?type=2&query=%E5%AE%98%E5%AF%9F%E7%AC%94%E8%AE%B0&page=1" -o /tmp/sg.html -w 'http=%{http_code}\n'
python3 - <<'PY'
import re
h=open('/tmp/sg.html',encoding='utf-8',errors='ignore').read()
names=re.findall(r'data-sourcename="([^"]*)"',h)
print('本条搜索结果来源:', names)
print('命中「官察笔记」条数:', names.count('官察笔记'))
PY
