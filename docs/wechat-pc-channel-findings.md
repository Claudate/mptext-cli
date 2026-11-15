# PC 微信取历史文章的实测记录

本文记录 **2026-10-19** 在 macOS 上对「能否用微信客户端本身拿到公众号全部历史文章」
这件事做的一轮实测：用了什么方法、看到什么输出、由此得出什么结论、哪些还没验证。

记录的原则是：能复现的写清命令，实测到的写清原始数字，推断的标注为推断。
微信客户端和接口行为会变，本文只在标注的版本上成立。

**为什么单独成文**：`mptext export` 的通道选择（主页栏目 + 搜狗索引）是被这些
否定结论逼出来的，不是设计偏好。README 只放结论摘要，凡是「这里为什么不做得更全」
的疑问都该回到本文查证据——否则下一个人会重复踩同一批坑，或者把已经证伪的
「抓个票据就能拿全量」重新写回代码。

**用法**：改通道逻辑、或用户问「为什么只有几十篇」时先读本文；
本文结论与代码冲突时，先按本文复现一遍，再决定改哪边。

## 测试环境

| 项目 | 值 |
| --- | --- |
| 系统 | macOS 15.6，arm64 |
| 微信 | 4.1.13（`build_ver 13`，`patch_ver 62`） |
| 主进程 | `WeChat`（pid 682），监听 `127.0.0.1:14013/14016/14019/14022/14023` |
| 网页外壳 | `WeChatAppEx`（pid 1127），Chromium/XWeb，`--wmpf_root_dir=…/app_data/radium` |
| 网络服务 | `WeChatAppEx Helper`，`--utility-sub-type=network.mojom.NetworkService` |
| 抓包 | mitmproxy（`tools/wxcapture.py`），系统代理指向 `127.0.0.1:8080` |

被测公众号：「英国那些事儿」（`__biz=Mzg2OTg5NDg3Mg==`）。
用户在微信里打开它的主页时，页面显示 **「10415 篇原创内容」**（该数字来自用户截图，
本次没有独立核实客户端如何算出它）。

## 确认事实

### 1. 微信内置浏览器确实走系统代理

此前有过相反结论，这次证伪了。依据：

```
lsof -nP -iTCP -a -p 1150
# WeChatApp 1150 … TCP 127.0.0.1:53890->127.0.0.1:8080 (ESTABLISHED)
```

网络服务进程持续向代理建连，代理侧也收到了它的请求并成功解密 TLS
（如 `mp/jsmonitor`、`mp/geticon`、`s?__biz=…` 正文页）。
所以代理这一层能完整看到微信内置浏览器的 `mp.weixin.qq.com` 流量。

### 2. PC 端不建立网页登录态：Cookie 里没有 `wap_sid2`

统计本次抓到的全部微信请求（24 条含 Cookie 的 `mp.weixin.qq.com` 请求）：

```
所有出现过的 cookie 名（次数）:
  rewardsn             24
wap_sid2 出现次数: 0
wap_sid 出现次数: 0
```

`rewardsn` 只是业务埋点用的标识，不是会话凭据。这解释了后面所有失败：
`profile_ext` 的鉴权要的是网页会话 cookie，而 PC 客户端从不产生它。

另一条独立验证：`radium/web/profiles/*/Cookies` 共 10 个 SQLite 库，
`weixin` 域 **0 条**（唯一有数据的是 13 条 toutiao/bytedance）。
即登录态连磁盘都不落，只在内存里，而内存里也没有 `wap_sid2`。

### 3. 拿真实 `key` 也换不到历史列表

微信自己的请求 URL 里带着完整凭据：`uin`、`key`（256 位）、`pass_ticket`、
`appmsg_token`。把这些原样复用去请求：

| 请求 | 结果 |
| --- | --- |
| `profile_ext?action=home`（真实 XWeb UA） | HTTP 200，**2034 B**，含「请在微信客户端打开」 |
| 同上，换 MacWechat UA | HTTP 200，2034 B，同上 |
| 同上，换纯浏览器 UA | HTTP 200，2034 B，同上 |
| 三种 UA 的 `Set-Cookie` | **0 B**，服务端不下发会话 |
| `profile_ext?action=getmsg`（带 501 B Cookie） | `{"ret":-3,"errmsg":"no session","cookie_count":0}` |
| 同上，Cookie 置空 | 完全相同 |

对比：`/mp/homepage?action=appmsg_list` 免登录可用，但对这个号只有 `hid=1`、
共 **2 篇**。所以「客户端显示上万篇」和「网页接口能拿到几篇」是两件事，
不是参数没调对。

### 4. 那 10415 篇不来自任何 HTTP 接口

开着代理、在页面里连续滚动加载、点进文章再返回，代理索引里
`mp.weixin.qq.com` 的请求只有这些：

```
  11 /mp/jsmonitor
   3 /mp/searchkeywordreport
   2 /mp/relatedsearchword
   1 /s
   1 /mp/wapcommreport
   1 /mp/tts
   1 /mp/geticon
   1 /mp/frontendcommstore
   1 /mp/audiolyrics
```

**没有任何一条是 `profile_ext` 或 `homepage`。** 列表在渲染前就已经有了。
同一时刻主进程 `WeChat`（682）的连接里没有一条指向代理——它走自己的长连接。

**推断**（非实测）：客户端主页的列表由主进程经私有协议取回后交给渲染层，
因此既不在系统代理里，也不是可复用的 HTTP 请求。要证实需要抓主进程的
私有协议流量，本次没做。

### 5. 无障碍树读不到页面内容

用 `AXUIElementCopyAttributeValue` 遍历微信窗口（pid 682，两个窗口）：

```
窗口数: 2
--- 窗口 0: 微信
[AXWindow] 微信
  [AXButton]
  [AXButton]
    [AXGroup]
  [AXButton]
--- 窗口 1: 微信 (窗口)
[AXWindow] 微信 (窗口)
  [AXButton]
  [AXButton]
    [AXGroup]
  [AXButton]
```

整个窗口只暴露 3 个按钮，没有任何文章标题、文本或列表节点。
这是 Chromium 系外壳的常见表现，不是权限问题（TCC 已放行）。

另外 XWeb 未开放远程调试：常见端口 `9222/9223/9229/8315/14013…` 全部无响应
（`9222` 上的 404 是 Chrome 自己的，不是微信）。

### 6. 覆盖量级：免登录通道能拿到多少

`mptext export <名字> --list-only` 实测（2026-10-19，本次运行）：

| 公众号 | 主页栏目 | 搜狗索引 | 合计 | 客户端显示 |
| --- | ---: | ---: | ---: | --- |
| 胡锡进观察 | 74 | 93 | **167** | — |
| 浙江宣传 | 116 | 1 | **117** | — |
| 后沙月光 | 29 | 50 | **79** | — |
| 深圳本地宝 | 8 | 65 | **73** | — |
| 远方青木 | 50 | 2 | **52** | — |
| 英国那些事儿 | 2 | 15 | **17** | 10415 篇原创 |
| 熊猫反诈 | 0 | 13 | **13** | — |
| 卢克文工作室 | 0 | 9 | **9** | — |
| 我们的东莞 | 0 | 3 | **3** | — |
| 占豪 | 0 | 2 | **2** | — |

（「—」表示本次没取客户端显示值，不做猜测。）

结论：免登录通道的量级是**个位到一百多篇**，和客户端的上万篇差两个数量级，
不是「再多翻几页」能补的。

### 7. 下载链路本身是通的

不是所有环节都坏。拿列表里的真实文章下载：

```
mptext export 英国那些事儿 -l 1 -i 0.5
  [1] output/英国那些事儿/事儿君精选_英国户外冲锋衣羽绒服夹克_1折起_这怎么买都不亏啊_.md
完成：成功 1 篇，失败 0 篇 → output/英国那些事儿
```

产物 10 KB。**有 URL 就能下全文**，瓶颈只在「怎么把 URL 全列出来」这一步。

## 方法：怎么复现这次抓包

```bash
# 1. 起代理（--set flow_detail=0 让输出干净）
mitmdump -s tools/wxcapture.py -p 8080 --set block_global=false --set flow_detail=0

# 2. 开系统代理（本次用的接口是 Ethernet，换成你的接口名）
networksetup -setwebproxystate "Ethernet" 127.0.0.1 8080
networksetup -setsecurewebproxystate "Ethernet" 127.0.0.1 8080

# 3. 在微信里打开任一篇公众号文章、滚动、点进点出
# 4. 落盘检查
ls ~/.config/mptext/capture/
```

`tools/wxcapture.py` 只落 `mp.weixin.qq.com`，每条含完整 URL、请求侧 Cookie、
请求/响应头、响应正文前 400 KB。**Cookie 是这条链路上唯一不可再生的东西**，
所以单独留一份。

注意：mitmproxy 根证书必须被系统信任，否则微信的 TLS 握手失败。判断依据是
代理日志里有 `client connect` 并且能看到解密后的请求行；如果只有
`client connect` 没有请求行，就是证书没被信任。本次该系统证书此前已信任过。

收尾：

```bash
networksetup -setwebproxystate "Ethernet" off
networksetup -setsecurewebproxystate "Ethernet" off
```

代理关掉后 `scutil --proxy` 应显示 `HTTPEnable: 0` / `HTTPSEnable: 0`。

## 由此确定的边界

**PC 微信客户端不提供可复用的历史列表凭据。** 这条结论覆盖：
不开调试端口、不注入、不越狱地使用客户端本身的正常能力。

| 想做的事 | 能不能 | 依据 |
| --- | --- | --- |
| 抓微信内置浏览器的 `mp.weixin.qq.com` 流量 | ✅ 能 | 事实 1 |
| 从抓到的流量里拿到网页登录态 | ❌ 拿不到 | 事实 2 |
| 用客户端的 `uin`/`key` 换全量历史 | ❌ `no session` | 事实 3 |
| 从客户端窗口直接读出文章标题 | ❌ AX 为空 | 事实 5 |
| 下全文（有 URL 时） | ✅ 能 | 事实 7 |
| 免登录列出个位到一百多篇 | ✅ 能 | 事实 6 |
| 列出客户端显示的上万篇 | ❌ 本次所有通道都不行 | 事实 4、6 |

**要接近全量，只有两条路：**

1. **公众号后台**（`mptext login` 扫码）：能看你有权限的号的全部历史。
   对别人的号无效。
2. **移动端微信的网页会话**：`profile_ext` 在移动端网页里才成立，
   需要在手机上取会话。本次未在移动端实测，不写成结论。

## 未验证 / 不在本文范围

- 移动端微信（Android / iOS）能否稳定拿到 `profile_ext` 会话：未测。
- 主进程私有协议的具体格式：未抓，事实 4 的相关部分是推断。
- Windows 版微信：未测，行为可能与 macOS 版不同。
- 除「英国那些事儿」之外各号的客户端显示篇数：没取。
- 微信版本升级后上述判断是否仍成立：未知。本文只对 4.1.13 负责。

## 变更记录

- 2026-10-19：首版。基于 macOS 微信 4.1.13 实测。

早前 `README.md` 里曾写过「`profile_ext` 可拿完整历史 / 需要微信客户端票据」，
以及配套的 `mitmptext ticket` 操作路径。本次实测证伪了「PC 端能出票据」这一点，
README 已相应改写并指向本文。
