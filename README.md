# mptext

微信公众号文章下载工具，提供 **图形界面桌面版** 与 **命令行版**，支持 Windows 与 macOS。

## 桌面版（推荐）

图形界面，无需记命令：

- 设置页微信扫码登录（会话只存在本机）
- 搜索公众号并浏览文章列表（分页）
- 勾选文章批量下载（Markdown / HTML / 纯文本）
- 下载进度实时显示

### 安装桌面版

在 [Releases](https://github.com/Claudate/mptext-cli/releases) 下载：

| 平台 | 文件 |
|------|------|
| macOS Apple Silicon | `mptext-gui_x.x.x_macOS-arm64.zip`（内含 `.app`） |
| macOS Intel | `mptext-gui_x.x.x_macOS-x64.zip` |
| Windows | `mptext-gui_x.x.x_Windows-x64.exe` |

首次使用请在应用内「设置」页用微信扫码，登录一个你有权限的公众号后台。

### 本地开发桌面版

```bash
cd mptext-app
npm install
npm run tauri dev
```

## 命令行版

适合脚本自动化场景。

| 平台 | 文件 |
|------|------|
| macOS Apple Silicon | `mptext-cli_x.x.x_macOS-arm64.zip` |
| macOS Intel | `mptext-cli_x.x.x_macOS-x64.zip` |
| Windows | `mptext-cli_x.x.x_Windows-x64.zip` |

```bash
cargo build --release -p mptext-cli
# 二进制位于 target/release/mptext
```

## 登录（搜索和拉列表需要）

```bash
mptext login
mptext auth
mptext search "公众号名"
```

单篇公开文章不需要登录：`mptext download <文章URL>`。

## 按号批量导出

```bash
# 列出能拿到哪些文章，不下载
mptext export 远方青木 占豪 --list-only

# 下载到 output/<公众号名>/
mptext export 远方青木 浙江宣传 --interval 1.5
```

三条来源合并去重，覆盖面各不相同：

| 来源 | 覆盖 | 代价 |
| --- | --- | --- |
| `/mp/homepage` 栏目 | 每号最多 4 个栏目 × 30 篇 | 免登录 |
| 搜狗移动搜索 | 每个号只索引到很浅一层 | 免登录，有验证码风险 |
| `profile_ext` 历史消息 | 仅微信移动端会话 | PC 端取不到，见下 |

后台接口 `/cgi-bin/appmsgpublish` 和 `/cgi-bin/appmsg?action=list_ex` 现在对第一页也返回
`200013 freq control`，重新扫码不会恢复。

### 为什么 PC 微信拿不到完整历史（2026-10-19 实测）

微信客户端主页显示的「10415 篇原创内容」**拿不到**。原因不是参数没调对：

- 微信内置浏览器**确实走系统代理**，`mp.weixin.qq.com` 流量全部抓得到；
- 但 PC 端**不建立网页登录态**——24 条请求的 Cookie 只有 `rewardsn`，`wap_sid2` 出现 0 次；
- 拿客户端真实的 `key`（256 位）+ `uin` + `pass_ticket` 请求：
  `action=home` → 2034 B「请在微信客户端打开」，`action=getmsg` → `ret:-3 no session`，
  三种 UA 都一样，服务端也不下发 Set-Cookie；
- 那上万篇由主进程的私有长连接提供，不经过代理，也不是 HTTP 接口；
- 微信窗口的无障碍树只有 3 个按钮，读不到任何文章标题。

免登录通道的实际量级（同一次实测，合计篇数）：胡锡进观察 167、浙江宣传 117、
后沙月光 79、深圳本地宝 73、远方青木 52、英国那些事儿 17、熊猫反诈 13、
卢克文工作室 9、我们的东莞 3、占豪 2。

**要接近全量只有两条路**：扫码登录公众号后台（`mptext login`，仅限你有权限的号），
或移动端微信的网页会话（`profile_ext` 只在移动端成立）。

完整的测试方法、原始输出、边界与未验证项见
[**docs/wechat-pc-channel-findings.md**](docs/wechat-pc-channel-findings.md)。

项目里实际在用的全部文章获取通道（后台列表、主页栏目、搜狗、合集、`profile_ext`、
`/mp/author`、正文页面）、各自的接口与凭据、以及合并顺序，见
[**docs/wechat-article-channels.md**](docs/wechat-article-channels.md)。

会话文件：

- Windows：`%APPDATA%/mptext/session.json`
- macOS：`~/.config/mptext/session.json`

## 项目结构

```
crates/mptext-core/   # API 客户端与配置（CLI/GUI 共用）
crates/mptext-cli/    # 命令行工具
mptext-app/           # Tauri 2 桌面应用
```

## 说明

文章列表有两条免登录来源：`/mp/homepage` 栏目与搜狗移动搜索索引
（https://weixin.sogou.com/weixinwap?type=2&query=<keyword>&page=<page>）。
两者的覆盖都不完整，合计从个位数到一百多篇不等，**不等于公众号的实际总篇数**。

只要文字、不要图片和视频，用纯文本格式：

```bash
mptext export 远方青木 --format text
```

`text` 输出会剥掉图片、视频、播放器与脚本，只保留段落分隔的正文；
默认的 `markdown` 会保留 `![](图片链接)`（只写链接，不下载图片文件）。

这不是实现缺陷，是 PC 端可用的 HTTP 通道就这么多。
通道清单与合并顺序见 [docs/wechat-article-channels.md](docs/wechat-article-channels.md)，
实测依据、被证伪的路径与边界见 [docs/wechat-pc-channel-findings.md](docs/wechat-pc-channel-findings.md)。

## License

MIT
