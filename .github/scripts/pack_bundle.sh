#!/usr/bin/env bash
# 把 out/ 里已有的单平台资产拼成一个合集包 anima-engine-all.zip。
#
# 为什么要有它：宿主仓库（编辑器/查看器）接入引擎时，六个平台各要一个地址，
# 很容易漏配一个、然后那个平台悄悄退化成内置实现。有了合集包，宿主只要配
# **一个** ENGINE_URL 就够了 —— 平台归属由宿主侧的解析器按目录结构判断
# （见 app 仓库的 .github/scripts/engine_inject.py）。
#
# 约定布局（解析器认的就是这个结构）：
#   windows/anima.dll
#   linux/libanima.so
#   macos/libanima.dylib
#   android/jni/<abi>/libanima.so      （整份 .aar 内容，含 classes.jar / manifest）
#   ios/anima.xcframework/...
#   web/anima_wasm.js + anima_wasm_bg.wasm
#
# 只打包**本次确实构建出来**的平台，缺的不硬凑（比如只勾了 web，就只含 web）。
# 用法: pack_bundle.sh <out 目录> <版本号> <构建号>
set -euo pipefail

out="${1:?缺 out 目录}"
version="${2:?缺版本号}"
build="${3:?缺构建号}"

work="$(mktemp -d)"
bundle="$work/bundle"
mkdir -p "$bundle"

included=()

# 从归档里找第一个匹配的文件；
find_in() { # $1=解到哪 $2=名字
  find "$1" -name "$2" -type f -print -quit
}

# ---- Windows（asset 里是 stage 目录的整份内容，只要动态库）----
if [ -f "$out/anima-engine-windows-x64.zip" ]; then
  d="$work/w"
  mkdir -p "$d"
  unzip -qq -o "$out/anima-engine-windows-x64.zip" -d "$d"
  f="$(find_in "$d" anima.dll)"
  if [ -n "$f" ]; then
    mkdir -p "$bundle/windows"
    cp "$f" "$bundle/windows/anima.dll"
    included+=(windows)
  fi
fi

# ---- Linux / macOS（tar.gz）----
if [ -f "$out/anima-engine-linux-x64.tar.gz" ]; then
  d="$work/l"
  mkdir -p "$d"
  tar -xzf "$out/anima-engine-linux-x64.tar.gz" -C "$d"
  f="$(find_in "$d" libanima.so)"
  if [ -n "$f" ]; then
    mkdir -p "$bundle/linux"
    cp "$f" "$bundle/linux/libanima.so"
    included+=(linux)
  fi
fi

if [ -f "$out/anima-engine-macos-universal.tar.gz" ]; then
  d="$work/m"
  mkdir -p "$d"
  tar -xzf "$out/anima-engine-macos-universal.tar.gz" -C "$d"
  f="$(find_in "$d" libanima.dylib)"
  if [ -n "$f" ]; then
    mkdir -p "$bundle/macos"
    cp "$f" "$bundle/macos/libanima.dylib"
    included+=(macos)
  fi
fi

# ---- Android（.aar 本身就是 zip，整份放进去）----
if [ -f "$out/anima-engine-android.aar" ]; then
  mkdir -p "$bundle/android"
  unzip -qq -o "$out/anima-engine-android.aar" -d "$bundle/android"
  included+=(android)
fi

# ---- iOS（zip 里是 anima.xcframework 目录）----
if [ -f "$out/anima-engine-ios.xcframework.zip" ]; then
  d="$work/i"
  mkdir -p "$d"
  unzip -qq -o "$out/anima-engine-ios.xcframework.zip" -d "$d"
  x="$(find "$d" -maxdepth 3 -name 'anima.xcframework' -type d -print -quit)"
  if [ -n "$x" ]; then
    mkdir -p "$bundle/ios"
    cp -R "$x" "$bundle/ios/anima.xcframework"
    included+=(ios)
  fi
fi

# ---- Web（wasm-bindgen 产物整份）----
if [ -f "$out/anima-engine-web.zip" ]; then
  mkdir -p "$bundle/web"
  unzip -qq -o "$out/anima-engine-web.zip" -d "$bundle/web"
  included+=(web)
fi

if [ ${#included[@]} -eq 0 ]; then
  echo "::warning::没有任何单平台资产，跳过合集包"
  rm -rf "$work"
  exit 0
fi

# 清单：便于拿到合集包时一眼看出里面有什么、是哪次构建。
cat > "$bundle/manifest.json" <<EOF
{
  "engine": "anima",
  "version": "$version",
  "build": "$build",
  "platforms": [$(printf '"%s",' "${included[@]}" | sed 's/,$//')],
  "layout": {
    "windows": "windows/anima.dll",
    "linux": "linux/libanima.so",
    "macos": "macos/libanima.dylib",
    "android": "android/jni/<abi>/libanima.so",
    "ios": "ios/anima.xcframework",
    "web": "web/anima_wasm.js + web/anima_wasm_bg.wasm"
  }
}
EOF

rm -f "$out/anima-engine-all.zip"
(cd "$bundle" && zip -qr "$out/anima-engine-all.zip" .)
echo "合集包平台：$(printf '%s ' "${included[@]}")"
unzip -l "$out/anima-engine-all.zip" | sed -n '3,40p'
rm -rf "$work"
