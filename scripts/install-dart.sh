#!/usr/bin/env bash
# install-dart.sh — 安装 Dart SDK（免 sudo，装到家目录）
#
# 已验证链接（2026-07-29）：
#   https://storage.googleapis.com/dart-archive/channels/stable/release/latest/sdk/dartsdk-linux-x64-release.zip
#   → HTTP 200，application/zip，~233MB
#
# 用途：Flutter/Dart 绑定测试。
# 验证安装：dart --version
set -euo pipefail

INSTALL_DIR="$HOME/.local/dart-sdk"
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

echo "▶ 下载 Dart SDK（stable，x64）..."
URL="https://storage.googleapis.com/dart-archive/channels/stable/release/latest/sdk/dartsdk-linux-x64-release.zip"
curl -fL "$URL" -o "$TMP_DIR/dart.zip"

echo "▶ 解压到 $INSTALL_DIR ..."
rm -rf "$INSTALL_DIR"
mkdir -p "$(dirname "$INSTALL_DIR")"
unzip -q "$TMP_DIR/dart.zip" -d "$(dirname "$INSTALL_DIR")"

echo "▶ 验证："
"$INSTALL_DIR/bin/dart" --version

cat <<EOF

✅ Dart SDK 安装完成：$INSTALL_DIR

请将以下行加入 ~/.bashrc（或 ~/.zshrc）：

    export PATH="$INSTALL_DIR/bin:\$PATH"

然后执行：source ~/.bashrc

EOF
