#!/usr/bin/env bash
# 快速启动 mptext 桌面版。
#
#   ./start.sh          # 启动图形界面
#   ./start.sh --cli    # 编译并运行命令行版
#
set -euo pipefail

cd "$(dirname "$0")"

if [ "${1:-}" = "--cli" ]; then
  # 指定 --bin：crates/mptext-cli/src/bin/ 下还有探针小工具，不指定会有歧义。
  cargo run -p mptext-cli --bin mptext -- "${@:2}"
  exit
fi

cd mptext-app
[ -d node_modules ] || npm install
exec npm run tauri dev
