# mptext

[mptext.top](https://down.mptext.top) 公众号文章下载工具，提供 **图形界面桌面版** 与 **命令行版**，支持 Windows 与 macOS。

## 桌面版（推荐）

图形界面，无需记命令：

- 设置页配置 / 验证 API 密钥
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

首次使用请在应用内「设置」页填入 Auth Key（在 [down.mptext.top](https://down.mptext.top) 登录获取）。

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

## 配置 Token（必填）

```bash
# 桌面版：应用内「设置」页保存
# CLI：
mptext config set-token "你的密钥"
mptext auth
```

配置文件路径：

- Windows：`%APPDATA%/mptext/config.toml`
- macOS：`~/.config/mptext/config.toml`

## 项目结构

```
crates/mptext-core/   # API 客户端与配置（CLI/GUI 共用）
crates/mptext-cli/    # 命令行工具
mptext-app/           # Tauri 2 桌面应用
```

## License

MIT
