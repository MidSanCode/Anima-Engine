# 已构建产物校验清单（`anima.dll` 等）

> 这份文件记录**某一批 `engine/target/` 产物的 SHA-256**，用于核对产物是否被
> 换掉或损坏。
>
> **2026-10-04 更新：构建环境已恢复正常。** 早先那条「本机无法重新构建」的
> 说明已经不成立（原因与排查过程见下节），`cargo build` / `cargo test` 现在
> 都能在离线模式跑通。因此 `target/` 在理论上已可再生，`cargo clean` 不再
> 需要当作禁忌 —— 但**非必要不清**，重建整套 wgpu 依赖较慢。

## 环境恢复记录（2026-10-04）

排查「下载依赖失败」时确认了三件事：

1. **网络本身是通的**。`https://index.crates.io/config.json` 与
   `https://static.crates.io/crates/zip/zip-0.6.6.crate` 直连均返回 **200**，
   TLS 握手正常。
2. **早先的 `schannel: SEC_E_NO_CREDENTIALS (0x8009030e)` 是暂时的**。当时
   连 `Invoke-WebRequest` 都失败；复查时同一命令返回 **200**，Schannel 事件
   日志里没有任何证书/凭据错误。`CryptSvc` / `KeyIso` / `WinHttpAutoProxySvc`
   三个服务均在正常运行，系统时间也没有偏差。
3. **代理（v2rayN + xray）没有被重启过**，进程已连续运行 3 天。系统代理
   （WinINET `ProxyEnable`）是**关闭**的，但这不影响 cargo —— 它可以直连。
   注意 `127.0.0.1:10808` 是 **SOCKS5**（握手回 `05 00`）；把它当 HTTP 代理用
   会超时，这是配置错误而非网络故障。

结论：这是一次**外部的、暂时性的 TLS 失败**，不是本机证书库损坏。恢复后
`cargo fetch --locked` 退出码 0，补齐了全部缺失 crate（含 `zip`）。

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

* 下面列出的这批产物是 **`0.1.0` 的构建**，**不含**动画模式（`animation.*`）
  的任何内容。编辑器要联调动画功能，必须重新构建引擎。
* 重新构建前先跑 `cargo fetch --locked` 确认依赖齐全（离线可用
  `cargo build --workspace --offline` 验证）。
* `CARGO_HOME` 位于 `F:\cache\cargo`（用户级环境变量）。换终端或换用户时
  若缓存目录变化，需要重新 `cargo fetch`。

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
