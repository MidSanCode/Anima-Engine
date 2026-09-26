# JSON Schema

`docs/ffi-contract.md` 与 `docs/format.md` 是人读的规范；本目录是机器可校验的形式，
由 `crates/am-format/tests/schemas.rs` 在 CI 中用真实文件与真实调用数据约束。

| 文件 | 约束对象 |
| --- | --- |
| `envelope.schema.json` | `am_call` 返回的信封（`{"ok":true,"result":…}` / `{"ok":false,"error":{code,message}}`） |
| `format.schema.json` | `info.json` / `registry.json` / `metadata/*.json`（definitions: `info` / `registry` / `asset_metadata`） |
| `model.schema.json` | `spec/model.json` —— 结构模型 |
| `spec.schema.json` | 描述层整体（`project.spec`；`model` 只查必填字段，完整结构用 model.schema.json 校验） |
| `command.schema.json` | `doc.command` 的 `command` 字段（28 种 op + `batch` 递归） |

## 使用

```bash
# 跑 schema 一致性测试
cargo test -p am-format --test schemas

# 在编辑器/工具里校验任意工程
anima validate <project>
```

## 约定

* draft-07；`additionalProperties` 不设 false（Rust 侧用 `extra` 平铺保留未知字段，
  前向兼容优先）。
* `id` 一律 `^[a-z0-9_-]+$`；`sha256` 一律 64 位小写十六进制。
* 向量是双形的：输出 `{ "x": …, "y": … }`，输入也接受 `[x, y]`。
* 新增字段时：先改 Rust 结构体与 `docs/`，再同步 schema —— 测试会兜底。
