#!/usr/bin/env bash
# install-swift.sh — 安装 Swift 6.3.3（免 sudo，装到家目录）
#
# 已验证链接（2026-07-29）：
#   https://download.swift.org/swift-6.3.3-release/ubuntu2404/swift-6.3.3-RELEASE/swift-6.3.3-RELEASE-ubuntu24.04.tar.gz
#   → HTTP 200，~1.07GB
#
# 系统：Linux Mint 22 (wilma, 基于 Ubuntu 24.04) x86_64
# Swift 运行时依赖（libcurl4 / libpython3.12 / libxml2）已验证本机存在。
#
# 注意：下载 ~1GB，可能需要几分钟。
# 验证安装：swift --version
set -euo pipefail

INSTALL_DIR="$HOME/.local/swift"
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

echo "▶ 下载 Swift 6.3.3（x86_64，ubuntu24.04）~1GB ..."
URL="https://download.swift.org/swift-6.3.3-release/ubuntu2404/swift-6.3.3-RELEASE/swift-6.3.3-RELEASE-ubuntu24.04.tar.gz"
curl -fL "$URL" -o "$TMP_DIR/swift.tar.gz"

echo "▶ 解压到 $INSTALL_DIR ..."
rm -rf "$INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
tar -xzf "$TMP_DIR/swift.tar.gz" -C "$INSTALL_DIR" --strip-components=1

echo "▶ 验证："
"$INSTALL_DIR/usr/bin/swift" --version

cat <<EOF

✅ Swift 安装完成：$INSTALL_DIR

请将以下行加入 ~/.bashrc（或 ~/.zshrc）：

    export PATH="$INSTALL_DIR/usr/bin:\$PATH"

然后执行：source ~/.bashrc

EOF
