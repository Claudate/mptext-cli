# MPider 的公众号下载方法记录

**仓库**：<https://github.com/areyoubugcoder/mpider>
**许可**：MIT（Copyright (c) 2026 areyoubugcoder）
**版本**：v1.0.0（2026 年正式开源发行），仅 Windows x64
**适配范围**：只验证过微信 **4.1.15.56 x64**；仓库里带两份 profile 指纹（`4.1.12.26`、`4.1.15.56`）
**本文性质**：读源码与项目文档得出的方法记录。**我没有在 Windows 上实跑它**，凡属作者自述或代码结构推断的地方都在正文里标了。

---

## 一、结论先行

它和本项目（mptext-cli）走的是**两条完全不同的路**：

| | 本项目现有通道 | MPider |
| --- | --- | --- |
| 列表来源 | 网页端接口（后台 `appmsgpublish`、主页栏目、搜狗、合集、`profile_ext`） | PC 微信客户端的**原生会话协议**（CGI `5814`，走 MMTLS 长连接） |
| 凭据 | 后台 Cookie / 抓包票据 / 免登录 | 从微信进程内存里取出 `psk` 与内层密钥，自己拼请求 |
| 卡点 | 第三方号固定 `200013`（权限模型）；PC 端票据换不出会话 | 不碰网页权限，直接以客户端身份请求 |
| 正文 | 裸 GET `/s` 页 | 裸 GET `/s` 页（与本项目同法） |
| 平台 | macOS 可用 | **仅 Windows x64** |

**它对本项目最有价值的一条信息**：本项目 `wechat-pc-channel-findings.md` 里的推断「那上万篇不来自任何 HTTP 接口」，在 MPider 这里得到了具体答案——那批数据来自 **CGI 5814 的原生调用**，被封装在 **MMTLS 记录**里，用**明文 HTTP POST 到 `/mmtls/<随机后缀>`** 承载（业务保护靠 MMTLS 与内层 GCM，不是 HTTPS）。所以本项目抓包抓不到它，原因不是「没抓到」，而是**它是 client 内部加密协议，不是网页接口**。

---

## 二、四段式流程

```text
① 监测（人在环，只登记不入库）
   开监测 → 用户在微信里手动打开该公众号主页 → 观察器比对模块指纹
   → 关联这一次请求/响应 → 认证完整会话 → DPAPI 落盘
   → 登记公众号身份 + 会话材料 + 首页游标

② 列表（程序独立发请求，不再依赖 Frida）
   首页 / 历史任务 → 预算闸门 → 请求先记账拿到 permit 才发
   → 双层认证解密 → 核对身份、业务码、游标 → 文章与检查点同一事务提交

③ 正文与图片（公开 GET，无需任何会话）
   GET 文章 URL → 存 original.html → 转 Markdown
   GET 图片 data-src → 落盘 → magic bytes 定扩展名

④ 导出（Markdown + 清洗后的离线 HTML + 图片 + 缺图清单）
```

关键设计：**Frida 只在 ① 出现**。材料取到之后，后续翻页全由独立的 Python 工作进程发请求，不再挂钩微信。这就是它比「一直挂着 Frida 驱动 UI」稳定的原因。

---

## 三、① 取会话材料（`native/session_agent.js`，136 行）

只观察、不调用微信业务函数。三步都做了失败即停的校验：

1. **版本指纹**：算 `Weixin.dll` 的 SHA256，与 `native/profiles/<版本>.json` 的 `module_sha256` 比对；再按 profile 里的 `rva` 读函数首 16 字节比对，并确认内存页可执行。不匹配直接停，**不允许只改版本号强行跑**。
2. **找密码结构**：在模块的只读段/读写段按字节模式扫密码结构表，回退 4 字节取 nid，只保留 **0x37f / 0x382 / 0x385**（对应 16/24/32 字节密钥），且初始化与变换函数指针必须落在模块内可执行区。这样偏移变了也能重新定位，不用赌硬编码地址。
3. **取材料**：hook initializer 拿 24 字节 `inner_enc_key` / `inner_dec_key`；hook 主函数 `onLeave` 从输出结构体取 32 字节 `psk`；hook 导出的 `send`，抓 `POST /mmtls/` 且 body ≤ 8KB 的原始报文当模板。

拿到的材料（`session-vault.dat`，DPAPI 保护，仅当前 Windows 用户可解）：

| 字段 | 长度 | 用途 |
| --- | --- | --- |
| `psk` | 32B hex | 派生外层收发密钥 |
| `inner_enc_key` / `inner_dec_key` | 24B hex | 内层 AES-192-GCM |
| `inner_dec_candidates` | 最多 8 个 24B | 响应解密逐个试，以 GCM 认证成功者为准 |
| `ch_template` / `hs_template` / `alert_template` | 161B / 20B / 7B | 拼新握手的三个记录模板 |
| `req_template` | protobuf hex | 业务结构模板，翻页只改 JSON 里的参数 |
| `inner_body_header` | 二进制包头 | 翻页时要重算其中的两个载荷长度 |
| `host` / `path` | 字符串 | 独立请求的目标主机与业务路由 |

挑哪条候选报文当列表模板，靠内层明文里是否含 `bizh5ext`（`62697a6835657874`）判断。

---

## 四、② 自己拼列表请求（`native/session_protocol.py`，448 行）

这是全项目技术含量最高的一段。列表**不是** HTTP JSON：

```text
POST http://<host>:80/mmtls/<随机4hex>
└ 记录层 19f104(ClientHello) / 19f104(扩展握手) / 17f104(加密应用数据) / 15f104(alert)
  └ len + path + len + host + len + inner
    └ 包头 + AES-192-GCM(密文) + 12B nonce + 16B tag
      └ 认证解密后：zlib → protobuf → JSON { BizUserName, ActionType, Offset }
```

两层密钥全部从 `psk` 用 **HKDF-Expand**（作者文档特意强调：**不能擅自再加一次 Extract**）派生：

```text
# 外层发送
info     = b"early data key expansion" || SHA256(ClientHello)
material = HKDF-Expand-SHA256(key=PSK, info, length=28)
send_key = material[0:16]      # AES-128-GCM
send_iv  = material[16:28]
# 记录 nonce = 12B IV 与记录序号整数异或；AAD = 8B 大端序号 || 5B 记录头

# 外层接收（摘要必须含本次扩展握手明文，不能只用两端 Hello）
transcript  = ClientHello || 客户端扩展握手明文 || ServerHello记录体
info        = b"handshake key expansion" || SHA256(transcript)
receive_key = HKDF-Expand-SHA256(key=PSK, info, length=28)[0:16]
receive_iv  = 同上[16:28]
```

**翻页不是重编码整个报文**：递归遍历 protobuf，找到唯一含 `BizUserName` 的 JSON 子消息，改写 `ActionType=1` 与 `Offset`，保留其它字段，再把各层 protobuf 长度、内层包头里两个载荷长度字段（索引 1、2）的变长整数重算回去。`build_request()` 还会更新 ClientHello 的 32 字节随机数、两个 Hello 里同一个时间戳，以及新的 12 字节内层 nonce。**发之前先在本地认证回解一次**，确认自拼封装与明文模板一致才真正发出去。

### 游标：只用服务端给的，不解码

- 游标是 `MsgList.PagingInfo.Offset`，**不透明字符串**；不解码含义、不按篇数推算。
- `IsEnd=1` 才停。
- 未结束却没有游标、或新游标等于本次请求的游标 → **暂停**（判定未推进）。
- 换号、认证失败、结构变化 → 暂停；**不自动重试**。
- 首页游标与任务进度分开存：恢复时重新取会话会带来一个新的首页游标，但**采集进度来自数据库**里的已提交检查点，不会被新首页游标覆盖（`native_jobs.cursor` 存真值，`native_call_log` 只存短哈希）。

### 请求控制（作者给出的默认值，明说不是安全额度）

| 项 | 默认 |
| --- | --- |
| 列表间隔 | 90–120 秒 |
| 列表滚动预算 | 24 小时 60 次 |
| 正文 / 图片间隔 | 各 1 秒 |
| 正文 / 图片并发 | 各 4 |

⇒ 理论上限约 **60 页/天 × 20 篇 ≈ 1200 篇/天**（按每页 20 计的粗估，**我没实测**）。

---

## 五、③ 正文与图片：和本项目一样的裸 GET

`crates/core/src/workbench/archive.rs`：

```rust
reqwest::Client::builder()
    .no_proxy()
    .timeout(Duration::from_secs(30))
    .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/130.0.0.0 Safari/537.36")
```

- 直接 `client.get(网址)`，**不带 Cookie、不带 Referer**，靠 URL 自带 `chksm` 就够。
- `429` → 全局冷却。
- 遇到 `ParseError::VerifyPage`（“环境异常”页）**不立刻判定限流**：拿一条「已知有效对照 URL」再取一次；对照也出验证页才冷却并暂停任务。理由很实在——实测存在无效 `sn` 的短链稳定返回验证页，没有对照会把整批误判成被限流。
- 图片取 `#js_content img` 的 `data-src`，URL 的 sha256 当文件名，校验 magic bytes（png/jpg/gif/webp/avif）定扩展名。
- 全文 `original.html` + `ammonia` 清洗 + CSP，图片转 base64 内联做离线 HTML。
- 所有请求先进**请求账本**（拿 `request_id` 换 permit 才发），调大并发也绕不过同类间隔。

---

## 六、可行性评估

### 分维度结论

| 维度 | 判定 | 依据 |
| --- | --- | --- |
| 技术能否跑通 | ✅ 成立 | 作者发布 v1.0.0，并称做过真机采集与更新验收；代码链路完整、有离线回归（用虚构材料，不发真实请求） |
| 能否绕过第三方号 `200013` | ✅ 是它最大价值 | 它根本不走网页后台接口，走客户端原生 CGI 5814 |
| 在 macOS 上能否用 | ❌ 不可用 | 客户端只发 Windows x64；原生采集明确声明 macOS/Linux 不支持 |
| 移植到 macOS 的成本 | 🔴 极高 | 需重做 Mach-O 逆向：模块名、函数定位、结构表扫描、指纹全部重来；MMTLS 那 448 行大概率也要重写 |
| 版本升级后能否维持 | 🔴 差 | 每个微信版本要重新做逆向并补 profile；作者明确「不能只改版本号强行适配」 |
| 稳定性 | ⚠️ 有已知坑 | 作者自述 **Frida 附加有导致微信退出的记录**，长期稳定性「仍需验证」 |
| 账号风险 | ⚠️ 作者明说不承诺 | 「所有需要使用个人微信号的客户端都可能封号，MPider 也不例外」；默认间隔「不是平台认可的安全额度，也不是防封承诺」 |
| 覆盖完整性 | ⚠️ 有取舍 | 不采视频、音频、评论、阅读量、互动数据 |

### 生态环境（作者态度值得记一笔）

README 用了很长篇幅反对「账号池」：把个人微信有效凭证上传云端，由第三方以你的身份发请求，风险却由你的账号承担。它自己也声明**不上传凭证、本地运行**，并公开源码让人核查。这一点它做到了（MIT + 完整源码 + 构建说明）。

### 一句话判断

**方法本身可行，但可行性绑死在「Windows x64 + 指定微信版本 + 敢挂 Frida」这三个条件上。** 对我们（macOS）来说，它是一个**高价值的情报来源**，不是一个可执行的方案。

---

## 七、对本项目可复用的部分

可以拿过来的（都是平台无关的）：

1. **正文三态判定**：`429` / 验证页 / 无效 `sn` 分开处理，并用「已知有效对照 URL」二次确认才判定限流。本项目 `detail.rs` 已有类似逻辑，可以直接对照这份实现补齐边界。
2. **检查点事务化**：首页游标 ≠ 任务游标 ≠ 检查点，三者分开存；恢复时只从数据库读进度，绝不被新取的首页游标覆盖。本项目断点续采的薄弱点正在这里。
3. **请求账本**：先记账拿 permit 再发，并发调大也绕不过间隔。比单纯的 `tokio::sleep` 可靠。
4. **身份逐篇核对**：响应的 `AccountInfo.UserName` 必须等于请求的 `BizUserName`；文章 URL 的 `__biz` 逐篇核对后才入库。防串号。
5. **拦截 UI 层 `ret=0` 的伪造**：独立请求没有原生桥返回码，账本里保持空，不能伪造成成功。这是个很克制的设计。

不能复用的：

- MMTLS 那一整套（取密钥 + 两层封装 + 翻页改写）。它锁死 Windows 模块布局。
- Frida 取会话材料的方式。macOS 版微信是 universal binary，`Weixin` 是 Mach-O，函数定位与结构表扫描全部要重做，**本仓一个字节都不通用**。

若真要走 macOS 原生路：需要独立立项，先解决「能不能在 macOS 微信里定位并挂钩对应的密码结构」这个问题，再谈 MMTLS。这一步的风险（签名校验、SIP、W^X、附加被检测）比 Windows 高得多。

---

## 八、如果要自己试它（Windows）

```text
环境：Windows 10/11 x64、WebView2 Runtime、已登录微信 4.1.15.56 x64
装：GitHub Releases 的 MPider_1.0.0_x64-setup.exe（NSIS，支持应用内更新）
用：① 开监测 → 手动打开目标公众号主页 → ② 建首页/历史任务 → ③ 下载全部正文 → ④ 导出
```

注意：首次登记公众号与更新会话时，**桌面必须解锁且可交互**（Frida 要有窗口可附）。

---

## 九、未验证 / 本文不做结论

- 我没有在 Windows 上实跑 MPider，也没有核实它在 2026-10 当下是否仍能正常采集。
- 「1200 篇/天」是按作者默认预算的算术粗估，未实测。
- 它能否拿到某号客户端显示的全部条数（本项目里那条 10415 的量级）：作者未承诺，我也没验证。它只采图文。
- 微信 4.1.15.56 之后的新版本是否还有 profile 更新：看仓库 issues / Releases。
- macOS / Linux 原生采集：作者明确不支持，且未给路线图。

## 变更记录

- 2026-10-29：首版。读 `areyoubugcoder/mpider` v1.0.0 源码（`native/`、`crates/core/src/`）与 `docs/wechat-session-protocol.md` 后整理。
