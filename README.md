# mptext-cli

[mptext.top](https://down.mptext.top) 公众号文章 API 的命令行工具。支持搜索公众号、拉取文章列表、单篇/批量下载为 Markdown/HTML 等格式。

## 安装

```bash
git clone git@github.com:Claudate/mptext-cli.git
cd mptext-cli
cargo install --path .
# 或: cargo build --release && cp target/release/mptext ~/.local/bin/
```

## 配置 Token（必填）

在 [down.mptext.top](https://down.mptext.top) 登录后获取 **Auth Key**，任选一种方式配置：

```bash
# 方式 1：环境变量（推荐）
export MPTEXT_AUTH_KEY="你的密钥"

# 方式 2：每次命令行传入
mptext --token "你的密钥" auth
```

验证密钥：

```bash
mptext auth
# 输出「API 密钥有效」即 OK
```

## 用法

### 搜索公众号

```bash
mptext search "作者昵称"
# 输出: 昵称\tfakeid
```

### 查看文章列表

```bash
mptext articles --fakeid MzAxxxx --size 20
```

### 下载单篇

```bash
mptext download "https://mp.weixin.qq.com/s/xxxx" -o article.md
# format: markdown(默认) | html | text | json
mptext download "https://mp.weixin.qq.com/s/xxxx" --format html -o article.html
```

### 批量下载

```bash
mptext fetch --fakeid MzAxxxx --limit 10 --output-dir ./articles
```

### 通过文章 URL 反查公众号

```bash
mptext account "https://mp.weixin.qq.com/s/xxxx"
```

## 与 Write-T 配合

下载的 `.md` 可直接粘贴到 Write-T **文章导入**，或配合 Write-T 的语料清洗/切块预览。

## API 说明

默认请求 `https://down.mptext.top`，可通过全局参数 `--base-url` 覆盖（私有化部署时使用）。

## License

MIT
