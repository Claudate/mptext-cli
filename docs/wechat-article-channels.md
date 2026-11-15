# 公众号文章获取通道说明

本文说明这个项目**实际在用的全部文章获取方式**：每一条通道请求哪个接口、需要什么凭据、
能翻多少、代码在哪、以及它们是怎么拼在一起的。

结论先行：**没有任何一条通道能稳定拿到某个号的全部文章**。微信把「某号全部历史」
这件事只开放给两种身份——对该号有后台权限的运营者，或微信客户端内部的私有长连接。
公开 HTTP 通道只能从不同侧面拼凑，篇数取决于目标号自己有没有栏目、有没有合集。

**为什么单独成文**：通道逻辑分散在 `wechat.rs`（5 条列表通道 + 正文）、`author.rs`
（作者历史）、`mitm.rs`（票据抓取）三处，合并顺序写在 `main.rs` / `lib.rs` 的两个
`fetch_*` 函数里，只看其中任何一处都拼不出全貌。排查「为什么只有几十篇」「加一条通道
该加在哪」时先读本文。本文只描述**当前实现**；某个接口为什么被放弃、哪条路已被证伪，
见 [wechat-pc-channel-findings.md](wechat-pc-channel-findings.md)。

PC 客户端显示的上万篇走的是客户端内部的原生 MMTLS 会话（CGI 5814），不是网页接口；
另一个项目对这条路的完整实现记录见 [wechat-mpider-method.md](wechat-mpider-method.md)。

改动通道逻辑后请同步更新本文的表格与合并顺序——把它当成代码的一部分，而不是外挂说明。

## 一、通道总览

| # | 通道 | 端点 | 需要的凭据 | 单号覆盖 | 实现位置 |
| --- | --- | --- | --- | --- | --- |
| 1 | 后台发布列表 | `/cgi-bin/appmsgpublish` | 扫码登录后台 | 仅自己名下的号 | `wechat.rs` `list_articles` |
| 2 | 主页栏目 | `/mp/homepage?action=appmsg_list` | 无 | 最多探测 40 个栏目，每栏目 30 篇 | `wechat.rs` `list_articles_homepage` |
| 3 | 搜狗移动搜索 | `weixin.sogou.com/weixinwap` | 无 | 索引很浅，混排 | `wechat.rs` `list_articles_sogou_all` |
| 4 | 合集 | `/mp/appmsgalbum?action=getalbum` | 无 | 无条数上限，但要先有 `album_id` | `wechat.rs` `list_all_albums` |
| 5 | 历史消息 | `/mp/profile_ext?action=getmsg` | 后台登录会话，或抓包票据 | 设计上全量（PC 端实际受票据限制） | `wechat.rs` `list_articles_full_history` |
| 6 | 作者历史 | `/mp/author` | MITM 抓到的 `uin`/`key` | 设计上全量，每页 30 条（同样卡在 PC 端票据） | `author.rs` `list_all` |
| 7 | 正文页面 | `mp.weixin.qq.com/s?...` | 无 | 单篇 | `wechat.rs` `download_article` |

第 7 条只负责「已知 URL → 正文」，前 6 条负责「发现有哪些文章」。
第 5、6 条依赖第 8 条基础设施：本地 MITM 代理抓取微信客户端票据（`mitm.rs`）。

## 二、逐条说明

### 1. 后台发布列表（`/cgi-bin/appmsgpublish`）

扫码登录一个你有权限的公众号后台，用会话里的 `token` + Cookie 请求发布列表。

- 参数：`fakeid`、`begin`（偏移）、`count`（上限 20）、`type=101_1`、`sub_action=list_ex`。
- **返回 `200013` 不是频控，是权限模型**：同一个会话查自己名下的号（`fakeid` 等于
  Cookie 里的 `bizuin`）能正常逐页翻到底；查第三方号固定返回 `200013`，重新扫码、换出口 IP
  都不恢复。所以这条通道对「导出别人的号」基本无效，代码在收到 200013 后自动降级到第 2、3 条。
- `200003` 才是登录失效，只有这个码才该提示用户重扫。
- 覆盖率：只有「你自己运营的号」能到全量。

### 2. 主页栏目（`/mp/homepage?action=appmsg_list`）

免登录。微信给每个公众号的「主页」按栏目（`hid`）下发文章列表。

- 请求：`POST /mp/homepage?__biz=<fakeid>&hid=<n>&begin=<偏移>&count=30&action=appmsg_list`。
- 探测 `hid` 从 1 到 40，连续 3 个空栏目就收工；每个栏目最多翻 4 页。
- **硬上限**：每栏目 30 篇，`count` 再大也会被截到 30，`begin=30` 起返回空——不能翻页。
- 缺陷：部分号（建号时没配栏目）`ret=5`，一个栏目都没有，这条通道贡献 0 篇。
- 返回的链接是 `http://`，解析时会升级成 `https://`。

### 3. 搜狗移动搜索

免登录，按公众号昵称搜。入口是 `https://weixin.sogou.com/weixinwap?type=2&query=<昵称>&page=<n>`。

- 每条结果是搜狗的 `/link?url=…` 跳转页，里面是一小段 JS；解析器把 JS 拼回真实的
  `mp.weixin.qq.com` 地址，再按 `__biz` 过滤掉混排进来的别的号。
- 翻页终止条件：**整页 0 条结果**才算到底。某页没命中这个号不算——搜狗按相关度混排，
  有的号要到第 4、5 页才出现自己的文章。
- 有验证码风险：命中 `antispider` 或「请输入验证码」直接抛错，`_all` 版本会把已拿到的结果保留下来。
- 覆盖率：很浅，且**偏旧**。实测「占豪」翻 10 页只对上 2 条，且时间戳集中在 2020 年前后。
- 只当兜底：给「没有栏目、没有合集」的号凑几条。

### 4. 合集（`/mp/appmsgalbum?action=getalbum`）

免登录，是免票据通道里**唯一能翻到底**的列表接口。

- 请求：`/mp/appmsgalbum?__biz=<biz>&action=getalbum&album_id=<id>&count=20&f=json`。
- 翻页游标：`begin_msgid` + `begin_itemidx`，取上一页最后一条的值；`continue_flag` 归 0 表示到底。
- 单页 20 条，游标不前进就停（防死循环）。
- **难点在发现 `album_id`**：没有任何接口能列出「一个号有哪些合集」
  （`getalbumlist` 一律 `no session`）。唯一入口是扫文章页 HTML 里的
  `appmsgalbuminfo.album_id`。
- 因为合集之间互不相通（种子文章只挂合集 A，A 里的文章才挂合集 B），
  `list_all_albums` 按轮展开：拉完一批就用新拿到的文章继续反查，最多 4 轮，
  种子扫描上限 300 篇。
- 覆盖：只有作者主动建过合集的部分，不等于全部文章；但对资讯号以外的号提升最大。

### 5. 历史消息（`/mp/profile_ext?action=getmsg`）

能翻到**全部历史**，每页 10 条，靠 `offset` 递增 + `can_msg_continue`。

门槛不是 UA 而是 Cookie：不带会话返回 `ret:-3 no session`。凭据有两条来源：

1. **后台登录会话**（`profile_ext_ticket`）：用 `session.json` 里的 Cookie 打开
   `/mp/profile_ext?action=home&__biz=…&scene=124&is_ok=1`，从返回页面里抽
   `key` / `uin` / `pass_ticket` / `appmsg_token`。不用抓包、不用装证书。
   页面若回「请在微信客户端打开」，说明这台机器没有该号的历史页登录态。
2. **抓包票据**（`list_articles_profile_ext`）：读 `~/.config/mptext/wxticket.json`，
   由 `tools/wxmitm_addon.py` 从微信客户端流量里写入，`key` 约两小时过期。

注意：第 1 条在 PC 端的实际效果受限——PC 微信不建立网页登录态
（Cookie 里没有 `wap_sid2`），实测换不出可用列表。详见 findings 文档。

### 6. 作者历史（`/mp/author`）

设计上最接近「全量」的通道，不受「每栏目 30 篇」「合集每页 20 条」限制。三步：

1. 从任意一篇公开文章页反查 `author_id`（匿名可读，`resolve_article`）；
2. 用 MITM 抓到的 `uin` / `key` / `pass_ticket` 请求 `/mp/author?action=show`
   （带 `devicetype=UnifiedPCMac`），换一枚 **24 小时**有效的 Cookie；
3. 带 Cookie 请求 `/mp/author?action=get_articles`，`limit=30`，
   用上一页最后一条的 `mid` 作为下一页的 `from_article_id`。

终止条件是「本页没有文章」或「游标没前进」，不是按页数判断——服务端重复返回同一页时必须停。
单号翻页上限 2000 页（防挂死）。

同一套 `uin`/`key` 在 PC 端大概率换不出 Cookie（与第 5 条同因），
这是已知的阻塞点，未在移动端实机验证。

### 7. 正文（`mp.weixin.qq.com/s?...`）

所有通道最后都落到这一个方法：`download_article(url, format)`。

- 不带 `chksm` 的文章链接会 302 到「环境异常」验证页。腾讯只校验参数存在、不校验取值，
  所以代码补一个**占位 `chksm`** 重试即可拿到全文。
- `chksm` 必须加在 `#`（fragment）之前，否则被页面忽略。
- 提取 `#js_content`，再转 `markdown` / `html` / `text` / `json`。
- 不需要登录，也不需要票据。

#### 四种输出格式与「只要文字」

| format | 扩展名 | 内容 | 含图片 |
| --- | --- | --- | --- |
| `markdown`（默认） | `.md` | 正文 + `![](图片URL)` | 是（不下载图片，只留链接） |
| `text` | `.txt` | **纯文字 + 段落** | **否** |
| `html` | `.html` | 原始正文 HTML | 是 |
| `json` | `.json` | `{url, html, text}` | `html` 字段里有 |

要「全部文章、不要图片和视频」，用 `text`：

```bash
mptext export 公众号名 --format text -o output
mptext download '<文章URL>' -f text -o out.txt
```

`text` 的处理方式（`html_to_text`）：

- 先整块删掉 `script` / `style` / `iframe` / `video` / `mpvideo` / `svg`；
- `img` 本身不含文字节点，剥标签后不会留下任何图片地址；
- 按块级元素（`p` / `div` / `h1`~`h6` / `li` / `tr` / `br` / `blockquote`）分段，
  段落之间保留一个空行；
- 行内标签（`a` / `span` / `strong` …）直接剥掉，不动文字。

2026-10-21 实测：一篇文章 `text` 输出 185 行 / 17 KB，`mmbiz.qpic.cn`、`mpvideo`、
`iframe`、`![](…)` 的出现次数均为 **0**；同一篇的 `markdown` 输出保留 6 处图片链接。

两个格式差异要注意：

- `markdown` **不下载图片文件**，只在原位写 `![](https://mmbiz.qpic.cn/...)`。
  想彻底不要图片就得用 `text`，光靠 markdown 只是「有链接」。
- `text` 会丢标题层级（标题只当成普通一行），标题与正文能分开、但分不出是几级标题。
  需要保留标题结构又不能有图片时，改用 markdown 再删掉 `![]()` 行。

### 8. MITM 抓包基础设施（`mitm.rs`）

第 5、6 条要的 `uin`/`key`/`pass_ticket` 由本地代理从微信客户端流量里截取。

- 起一个本地代理（默认 `127.0.0.1:8899`），只对 `mp.weixin.qq.com` 和
  `*.weixin.qq.com` 做中间人解密，其余流量原样隧道透传。
- 自签根证书 `mptext MITM CA`，装进**当前用户**信任域（用户域即可，不用管理员密码）。
- 抓到的凭据按 `__biz` 归档到 `~/.config/mptext/wxcapture.json`。
- 证书信任与系统代理开关都只在 **macOS** 上自动完成（`networksetup` 需提权一次）；
  其他平台的 `trust_ca` / `set_system_proxy` 直接返回「请手动设置」，需自行把
  HTTP/HTTPS 代理指到 `127.0.0.1:8899` 并导入生成的 CA。
- 用完记得关代理，否则程序退出后系统会断网。

## 三、合并顺序与去重

`mptext export`（CLI，`main.rs` 里的 `merge_push`）和 GUI 的「搜索下载」
（`lib.rs` 的 `fetch_by_biz`）走的是**同一种顺序、但是两份实现**：

```
主页栏目 → 搜狗索引 → 合集（多轮展开） → 登录态全部历史 → 抓包票据(全部历史)
```

- 每一步按文章 URL 去重累加，任一步失败都不中断整体，只跳过该步。
- 合集通道的种子来自前两步已经拿到的文章 URL（GUI 里还会把你贴的那一篇也当种子）。
- 登录态不可用时打印原因，降级为纯免登录通道，不报失败。
- 最后按发布时间倒序，没有时间的排到最后。
- 每一步的新增篇数都会打印（CLI）或通过事件推给界面（GUI），可以据此判断是哪个通道在起作用。

要注意两份实现不共用：合集的多轮展开逻辑在 core 的 `list_all_albums` 里是共用的，
但「先哪条、后哪条」「失败怎么降级」分别写在两处。改通道顺序时**两处都要改**，
否则 CLI 和 GUI 会悄悄跑出不同的篇数。

## 四、实测覆盖量级（2026-10-19，免票据，非保证值）

| 公众号 | 篇数 | 主要来源 |
| --- | --- | --- |
| 胡锡进观察 | 167 | 主页栏目 74 + 搜狗 93 |
| 浙江宣传 | 117 | 主页栏目 |
| 后沙月光 | 79 | 主页栏目 |
| 深圳本地宝 | 73 | 主页栏目 |
| 远方青木 | 52 | 主页栏目 |
| 英国那些事儿 | 17 | 主页栏目 |
| 熊猫反诈 | 13 | 主页栏目 |
| 卢克文工作室 | 9 | 搜狗 |
| 我们的东莞 | 3 | 搜狗 |
| 占豪 | 2 | 搜狗 |
| 阿昆的科研日常 | 482 | 搜狗 85 + 4 个合集 397（该号无栏目） |

同一时期用户在微信客户端里看到的「1243 篇」「10415 篇」来自客户端私有接口，
公开 HTTP 通道拿不到，**不是抓取漏了**。

## 五、已停用的通道

- **`down.mptext.top` 公开 API**：2026-07-30 起因上游核心接口被关而停止服务，
  现在请求只会得到「认证信息无效」。`crates/mptext-core/src/client.rs` 里的
  `MptextClient` 还编译在包里，但 CLI 和 GUI 都已不再调用；`mptext config set-token`
  仍会写入 `config.toml`，只在启动时打一行「密钥已停用」的提示，不会恢复任何功能。
- **`/cgi-bin/appmsg?action=list_ex`**：旧版列表接口，与 `appmsgpublish` 同因失效。

## 六、排障对照表

| 现象 | 含义 | 处理 |
| --- | --- | --- |
| `ret=200013` | 后台权限模型，不是频控 | 别重复扫码；该号走免登录通道 |
| `ret=200003` | 登录已失效 | 重新 `mptext login` |
| 主页栏目全 `ret=5` | 该号没配栏目 | 依赖搜狗 + 合集 |
| 搜狗报「要求验证码」 | 被反爬拦截 | 等一段时间或换网络；已拿到的结果会保留 |
| 文章页 302 到「环境异常」 | 缺 `chksm` | 代码已自动补占位值重试 |
| 历史列表回「请在微信客户端打开」 | 本机没有该号登录态 | 只能在移动端网页会话里成立 |
| `/mp/author` 卡在换 Cookie | PC 端票据换不出会话 | 已知阻塞，见第 6 条 |
| 拿到的篇数远少于客户端显示 | 客户端专属数据 | 正常，见第四节 |

## 七、辅助脚本

| 文件 | 用途 |
| --- | --- |
| `tools/wxcapture.py` | mitmproxy 插件，把微信经代理发出的 `mp.weixin.qq.com` 请求原样落盘（排查用） |
| `tools/wxmitm_addon.py` | mitmproxy 插件，抽取 `uin`/`key`/`pass_ticket` 写入 `wxticket.json` |
| `probe_*.py` / `probe_*.sh` / `probe_*.mjs` | 各通道的实网探针，用来验证某个接口当下是否仍然可用 |
| `tools/verify_output.py` | 检查落盘产物是否为真实正文，而不是验证码页或空壳 |

探针脚本是排查工具，不参与正常导出流程。
