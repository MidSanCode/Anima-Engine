# Anima Engine

Live2D 类 2D 角色创建与展示工具的**引擎**。Rust 实现，跨平台（Windows / macOS / Linux / Web），
对外只暴露一层 JSON 方法协议（C ABI 与 wasm 完全同构）。

仓库根目录**就是** Cargo 工作区根；下面直接列出本仓库的内容
（不存在 `engine/` 这种再套一层子目录）：

```text
├── crates/
│   ├── am-math      二维数学（Vec2 / Mat3 / Rect / 仿射）
│   ├── am-format    .amproj 读写、校验、打包、sha256
│   ├── am-model     描述层数据结构（模型 / 物理 / 动作 / 表情 / 设置）
│   ├── am-eval      参数求值、关键形插值、变形器级联、场景组装
│   ├── am-render    wgpu 离屏渲染（混合模式 / 遮罩 / 层合成 / 读回）
│   ├── am-doc       命令系统 + 撤销/重做
│   ├── am-physics   确定性物理（摆锤 / 顶点链）
│   ├── am-motion    动作播放（曲线 / 循环 / 淡入淡出）
│   ├── am-core      会话门面 + JSON 方法分发（47 个方法）
│   ├── am-ffi       C ABI（cdylib/staticlib）+ bindings/include/anima.h
│   ├── am-wasm      wasm-bindgen 绑定
│   └── am-cli       anima 命令行工具
├── schemas/         JSON Schema（envelope / format / model / spec / command）
├── docs/
│   ├── ffi-contract.md   ★ 编辑器/查看器对接的唯一契约
│   └── format.md         .amproj 格式规范
├── bindings/include/anima.h   C ABI 头文件（宿主直接 include）
├── .github/workflows/build.yml  六平台构建 + 发布为 Releases
└── samples/minimal       最小可运行工程
```

各平台产物形态与「编辑器/查看器怎么消费」见 `docs/mobile.md` 与
`docs/ffi-contract.md`。

## 快速开始

```bash
# 全量测试（约 300 个用例，含 GPU 渲染测试）
cargo test --workspace

# 只跑不依赖 GPU 的部分（CI / 无显卡环境）
cargo test --workspace --no-default-features

# 构建 C ABI 动态库（宿主 / 编辑器用）
cargo build -p am-ffi --release
#   → target/release/anima.dll（Windows）/ libanima.so / libanima.dylib

# 命令行工具
cargo run -p am-cli -- version
cargo run -p am-cli -- validate samples/minimal
cargo run -p am-cli -- render   samples/minimal -o preview.png --width 256 --height 256
```

## 设计约束（改代码前请先读）

1. **分层不可逆**：`am-format` 不依赖 `am-model`（格式层只管文件，不懂语义）；
   `am-eval` 不做任何 I/O；只有 `am-core` 知道全部东西。
2. **一切结构修改都走 `am-doc` 命令**，否则撤销栈与画面会不一致。
3. **确定性**：物理与动作里禁止随机数、禁止读墙上时钟；时间只由 `runtime.advance` 推进。
4. **id 只增不复用**：删除节点/纹理/参数后，其 id 永久作废。
5. **方法只增不改**：破坏性变更必须升版本（见 `docs/ffi-contract.md` §10）。
6. **GPU 是 feature**：`gpu`（默认开）关掉后 `renderer.*` 返回 `-32000`，其余功能不受影响。
7. **文档与代码同步**：改了协议就改 `docs/ffi-contract.md`，它是编辑器的唯一依据。

## 坐标与时间约定

* 画布原点在**中心**，Y 轴向上，单位 = 画布像素。
* 纹理 UV 原点在**左上角**，范围 `[0, 1]`。
* 旋转角单位是**弧度**。
* 时间单位是**秒**，物理默认固定 60 Hz 步长。

## 测试策略

| 层次 | 位置 | 说明 |
| --- | --- | --- |
| 单元测试 | 各 crate `src/**/tests` | 数学、格式、求值、命令、物理、动作 |
| GPU 集成测试 | `crates/am-render/tests/render.rs` | 真实设备渲染 + 逐像素断言（遮罩/混合/层） |
| 协议契约测试 | `crates/am-core/tests/api.rs` | 21 个用例走 `dispatch_json`，锁死信封与方法形状 |
| Schema 一致性测试 | `crates/am-format/tests/schemas.rs` | `schemas/*.schema.json` 约束真实工程文件与真实调用数据 |
| C ABI 测试 | `crates/am-ffi/tests/ffi.rs` | 只通过 C ABI 驱动引擎，含渲染与事件回调 |
| CLI 测试 | `crates/am-cli/src/main.rs` | 建工程 → 校验 → 渲染 → 导出 |
| 示例工程 | `samples/minimal` | 手工编写的 `.amproj`，用于校验格式与渲染链路 |

GPU 测试会真的创建适配器，单次约 30 秒；测试间用 `OnceLock<Mutex<Renderer>>` 复用设备。

## 当前状态

引擎核心（格式 / 求值 / 渲染 / 编辑 / 物理 / 动作 / FFI / CLI / wasm）已完成并通过
全量测试。对外交付物已经齐了：C ABI 动态库 + 静态库、Android `.aar`、iOS
`.xcframework`、wasm-bindgen ES 模块，由 `.github/workflows/build.yml` 在六个平台上
构建，勾选 `publish_release` 后按固定资产名发布到 Releases，供编辑器/查看器匿名直链拉取。

尚未完成的是**宿主侧的画面桥**：Windows 外部纹理桥、Android `SurfaceTexture` /
iOS `IOSurface` 绑定，以及网格自动生成与 PSD 导入。
