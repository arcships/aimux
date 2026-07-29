#!/usr/bin/env bash
# install-kotlin.sh — 安装 Kotlin 编译器 + Gradle（免 sudo，装到家目录）
#
# 已验证链接（2026-07-29）：
#   Kotlin 2.4.10 编译器: https://github.com/JetBrains/kotlin/releases/download/v2.4.10/kotlin-compiler-2.4.10.zip
#     → 302 重定向到 github release-assets，最终 ~87MB，HTTP 200
#   Gradle 8.14.3: https://services.gradle.org/distributions/gradle-8.14.3-bin.zip
#     → 307→302 重定向到 github release-assets，最终 ~137MB，HTTP 200
#
# 前置：需要 JDK（先运行 install-jdk.sh）
# 验证安装：
#   kotlinc -version
#   gradle -version
set -euo pipefail

if ! command -v java &>/dev/null; then
  echo "❌ 未找到 java。请先运行 scripts/install-jdk.sh 并 source ~/.bashrc" >&2
  exit 1
fi

KOTLIN_DIR="$HOME/.local/kotlin"
GRADLE_DIR="$HOME/.local/gradle"
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

echo "▶ 下载 Kotlin 编译器 2.4.10..."
curl -fL "https://github.com/JetBrains/kotlin/releases/download/v2.4.10/kotlin-compiler-2.4.10.zip" \
  -o "$TMP_DIR/kotlin.zip"

echo "▶ 解压到 $KOTLIN_DIR ..."
rm -rf "$KOTLIN_DIR"
mkdir -p "$KOTLIN_DIR"
unzip -q "$TMP_DIR/kotlin.zip" -d "$KOTLIN_DIR"
# kotlinc 解压后顶层是 kotlinc/，上移一层
mv "$KOTLIN_DIR/kotlinc/"* "$KOTLIN_DIR/" 2>/dev/null && rmdir "$KOTLIN_DIR/kotlinc" 2>/dev/null || true

echo "▶ 下载 Gradle 8.14.3..."
curl -fL "https://services.gradle.org/distributions/gradle-8.14.3-bin.zip" \
  -o "$TMP_DIR/gradle.zip"

echo "▶ 解压到 $GRADLE_DIR ..."
rm -rf "$GRADLE_DIR"
mkdir -p "$GRADLE_DIR"
unzip -q "$TMP_DIR/gradle.zip" -d "$GRADLE_DIR"
mv "$GRADLE_DIR/gradle-8.14.3/"* "$GRADLE_DIR/" 2>/dev/null && rmdir "$GRADLE_DIR/gradle-8.14.3" 2>/dev/null || true

echo "▶ 验证："
"$KOTLIN_DIR/bin/kotlinc" -version
"$GRADLE_DIR/bin/gradle" -version

cat <<EOF

✅ Kotlin + Gradle 安装完成。

请将以下行加入 ~/.bashrc（或 ~/.zshrc）：

    export KOTLIN_HOME="$KOTLIN_DIR"
    export GRADLE_HOME="$GRADLE_DIR"
    export PATH="\$KOTLIN_HOME/bin:\$GRADLE_HOME/bin:\$PATH"

然后执行：source ~/.bashrc

EOF
