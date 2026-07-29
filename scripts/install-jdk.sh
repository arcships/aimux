#!/usr/bin/env bash
# install-jdk.sh — 安装 Eclipse Temurin JDK 21 LTS（免 sudo，装到家目录）
#
# 已验证链接（2026-07-29）：
#   https://api.adoptium.net/v3/binary/latest/21/ga/linux/x64/jdk/hotspot/normal/eclipse?project=jdk
#   → 307 重定向到 github release，最终 ~207MB，HTTP 200
#
# 用途：Kotlin 绑定测试需要 JVM。
# 验证安装：$HOME/.local/jdk21/bin/java -version
set -euo pipefail

INSTALL_DIR="$HOME/.local/jdk21"
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

echo "▶ 下载 Temurin JDK 21 LTS（x64）..."
URL="https://api.adoptium.net/v3/binary/latest/21/ga/linux/x64/jdk/hotspot/normal/eclipse?project=jdk"
curl -fL "$URL" -o "$TMP_DIR/jdk.tar.gz"

echo "▶ 解压到 $INSTALL_DIR ..."
rm -rf "$INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
tar -xzf "$TMP_DIR/jdk.tar.gz" -C "$INSTALL_DIR" --strip-components=1

echo "▶ 验证："
"$INSTALL_DIR/bin/java" -version

cat <<EOF

✅ JDK 安装完成：$INSTALL_DIR

请将以下行加入 ~/.bashrc（或 ~/.zshrc）：

    export JAVA_HOME="$INSTALL_DIR"
    export PATH="\$JAVA_HOME/bin:\$PATH"

然后执行：source ~/.bashrc

EOF
