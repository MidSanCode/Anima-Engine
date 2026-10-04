# 已构建产物校验清单（`anima.dll` 等）

> 这份文件记录**当前工作区 `engine/target/` 里那一批已构建产物**的 SHA-256。
> 它存在的唯一理由：**本机当前无法重新构建引擎**（TLS 证书凭证损坏 + 本地
> cargo 缓存缺 161 个 crate，含 `zip`/`wgpu`/`image`），所以 `target/` 里的
> 这批二进制是**暂时不可再生的**。
>
> 一旦 `cargo build -p am-ffi --release` 恢复正常，本文件即可删除。

## 为什么会有这份清单

清理工作区时删掉了 `temp/engine-artifacts-keep/`（95 MB 的产物备份）。
删除前逐个核验过：**6 个产物与 `engine/target/` 中的同名文件 SHA-256 完全一致**，
因此备份是冗余的。这份清单保留了「当时那批产物是什么」的可追溯记录，
供日后核对 `target/` 是否被换掉或损坏。

## 如何核验

```powershell
cd F:\exeliang\Anima\engine
foreach ($e in (Get-Content docs\artifacts-manifest.json -Raw | ConvertFrom-Json)) {
    $leaf = ($e.path -split '\\')[-1]
    $src  = @("target\release\$leaf",
              "target\wasm32-unknown-unknown\release\$leaf") |
            Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $src) { "缺失: $leaf"; continue }
    $ok = (Get-FileHash $src -Algorithm SHA256).Hash -eq $e.sha256
    "{0,-20} {1}" -f $leaf, $(if ($ok) { '一致' } else { '不一致' })
}
```

## 重要的操作提醒

* **不要对 `engine/` 运行 `cargo clean`**，否则这批产物会被清掉，而在 TLS/缓存
  修好之前无法重建。
* 这批产物是 `0.1.0` 的构建，**不含**动画模式（`animation.*`）的任何内容。
  编辑器若要联调动画功能，仍需先恢复构建环境。

## 产物清单

| 文件 | 位置 | 说明 |
| --- | --- | --- |
| `anima.dll` | `target/release/` | C ABI 动态库（编辑器/查看器 FFI 用） |
| `anima.dll.lib` | `target/release/` | Windows 导入库 |
| `anima.exe` | `target/release/` | CLI（`anima` 命令） |
| `anima.lib` | `target/release/` | 静态库（iOS 用） |
| `anima.pdb` | `target/release/` | 调试符号 |
| `anima_wasm.wasm` | `target/wasm32-unknown-unknown/release/` | Web 绑定 |

逐文件 SHA-256 见同目录 `artifacts-manifest.json`。

## 版本历史

| 版本 | 日期 | 说明 |
| --- | --- | --- |
| v1 | 2026-10 | 清理 `temp/engine-artifacts-keep/` 时建立；记录 6 个已构建产物的校验和 |
