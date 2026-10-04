# Anima Engine

[English](#english) · [中文](#中文)

A cross-platform engine for **creating and playing back deformable 2D character models**.
Written in Rust; exposes a single JSON method protocol over a C ABI (and an identical
wasm binding), so every host — desktop app, browser page, or game integration — speaks
the same contract.

面向**可变形 2D 角色模型**的创建与播放引擎。Rust 实现，只对外暴露一层 JSON 方法协议
（C ABI 与 wasm 绑定完全同构），因此桌面应用、浏览器页面、游戏集成等所有宿主都使用同一份契约。

---

## English

### What it is

Anima Engine is the **core** of a 2D character authoring and playback pipeline. It owns
the model, the evaluation, and the rendering — not the UI. Editors and viewers drive it
entirely through method calls, and every structural edit goes through the engine's own
command system so that undo/redo, the on-screen result, and the saved file can never
disagree.

Capabilities:

* **Model** — part tree, drawables, warp and rotation deformers, keyforms bound to parameters.
* **Evaluation** — parameter blending with the four blend modes, keyform interpolation,
  deformer cascading, draw order, opacity inheritance.
* **Rendering** — offscreen GPU rendering (premultiplied alpha, masking, layer compositing),
  pixel readback, optional per-platform texture sharing.
* **Editing** — a command stream with undo/redo; one `batch` is one undo step; a rejected
  command leaves the model untouched.
* **Dynamics** — deterministic physics (pendulum / vertex chain) and a motion player
  (curves, looping, cross-fade).
* **Animation modes** — a **live** mode (`runtime.advance`, stateful physics) and a
  **prerender** mode: a baked, stateless parameter track with a real timeline, speed
  scaling, keyframes and structural channels, so playback is seekable and bit-identical
  across hosts. Specification: [`../docs/animation-mode.md`](../docs/animation-mode.md).
* **Storage** — the `.amproj` project format: directory mode plus a packaged archive,
  with per-asset checksums and a validator.
* **Interop** — a stable C ABI (`am_call`), a matching wasm binding, JSON Schemas, and a
  CLI for validation, rendering, and packaging.

### Repository layout

The repository root **is** the Cargo workspace root (there is no nested `engine/` directory):

```text
├── crates/
│   ├── am-math      2D math (Vec2 / Mat3 / Rect / affine)
│   ├── am-format    .amproj read, write, validate, package, sha256
│   ├── am-model     description-layer data structures
│   ├── am-eval      parameter evaluation, keyform interpolation, deformer cascade
│   ├── am-render    offscreen GPU rendering (blend modes, masks, layer compositing)
│   ├── am-doc       command system + undo/redo
│   ├── am-physics   deterministic physics
│   ├── am-motion    motion playback (curves, looping, cross-fade)
│   ├── am-core      session facade + JSON method dispatch (47 methods)
│   ├── am-ffi       C ABI (cdylib/staticlib) + bindings/include/anima.h
│   ├── am-wasm      wasm-bindgen binding
│   └── am-cli       `anima` command-line tool
├── schemas/         JSON Schemas (envelope / format / model / spec / command)
├── docs/
│   ├── ffi-contract.md   the contract hosts integrate against
│   ├── format.md         .amproj format specification
│   └── mobile.md         Android / iOS delivery and integration
├── crates/am-ffi/bindings/include/anima.h   C ABI header
├── .github/workflows/build.yml              six-platform build + release publishing
└── samples/minimal                          minimal runnable project
```

### Quick start

```bash
# Full test suite (about 300 cases, including GPU rendering tests)
cargo test --workspace

# Only the parts that need no GPU (CI / headless machines)
cargo test --workspace --no-default-features

# Build the C ABI library that hosts link against
cargo build -p am-ffi --release
#   → target/release/anima.dll (Windows) / libanima.so / libanima.dylib

# Command-line tool
cargo run -p am-cli -- version
cargo run -p am-cli -- validate samples/minimal
cargo run -p am-cli -- render   samples/minimal -o preview.png --width 256 --height 256
```

Hosts should call `system.capabilities` at startup and only invoke methods it reports,
rather than hard-coding a method list. See `docs/ffi-contract.md` for every method,
parameter, and data structure.

### Design constraints

Read these before changing code:

1. **Layering is one-way.** `am-format` does not depend on `am-model` (the format layer
   only understands files, not semantics); `am-eval` performs no I/O; only `am-core`
   knows about everything.
2. **Every structural edit goes through `am-doc` commands** — otherwise the undo stack
   and the rendered result diverge.
3. **Determinism.** No randomness and no wall-clock reads in physics or motion; time
   advances only via `runtime.advance`.
4. **Ids are never reused.** Once a node, texture, or parameter is deleted, its id is
   permanently retired.
5. **Methods are additive.** Existing methods keep their shape; breaking changes require
   a version bump (`docs/ffi-contract.md` §10).
6. **GPU is a feature.** With `gpu` disabled (it is on by default), `renderer.*` returns
   `-32000` and everything else keeps working.
7. **Docs track code.** Change the protocol, change `docs/ffi-contract.md` — it is the
   single source of truth for hosts.

### Coordinates and time

* Canvas origin is at the **center**, Y axis points **up**, one unit equals one canvas pixel.
* Texture UV origin is at the **top-left**, range `[0, 1]`.
* Rotation angles are in **radians**.
* Time is in **seconds**; physics uses a fixed 60 Hz step.

### Testing strategy

| Layer | Location | What it covers |
| --- | --- | --- |
| Unit tests | `src/**/tests` in each crate | math, format, evaluation, commands, physics, motion |
| GPU integration | `crates/am-render/tests/render.rs` | real device rendering with per-pixel assertions |
| Protocol contract | `crates/am-core/tests/api.rs` | 21 cases through `dispatch_json`, locking the envelope and method shapes |
| Schema conformance | `crates/am-format/tests/schemas.rs` | the schemas constrain real project files and real call payloads |
| C ABI | `crates/am-ffi/tests/ffi.rs` | the engine driven only through the C ABI, including rendering and callbacks |
| CLI | `crates/am-cli/src/main.rs` | create → validate → render → export |
| Sample project | `samples/minimal` | a hand-written project exercising the format and render path |

GPU tests create a real adapter and take roughly 30 seconds; they share one device
through a `OnceLock<Mutex<Renderer>>`.

### Current status

The engine core is complete and passes the full test suite: format, evaluation, rendering,
editing, physics, motion, C ABI, CLI, and wasm. All delivery artifacts exist — C ABI
dynamic and static libraries, an Android `.aar`, an iOS `.xcframework`, and a
wasm-bindgen ES module — built on six platforms by `.github/workflows/build.yml` and
published to Releases under stable asset names for anonymous download.

Still outstanding is **host-side presentation**: the Windows external texture bridge,
Android `SurfaceTexture` / iOS `IOSurface` bindings, automatic mesh generation, and
layered bitmap import.

### License

GNU Lesser General Public License v2.1 — see [LICENSE](LICENSE).

---

## 中文

### 这是什么

Anima Engine 是 2D 角色创作与播放链路的**核心**：它负责模型、求值与渲染，不负责界面。
编辑器与查看器完全通过方法调用来驱动它；所有结构修改都走引擎自己的命令系统，因此
撤销栈、画面与落盘文件三者永远不会互相矛盾。

能力范围：

* **模型** —— 部件树、绘制对象、变形变形器与旋转变形器、绑定到参数的关键形。
* **求值** —— 四种混合模式的参数混合、关键形插值、变形器级联、绘制顺序、不透明度继承。
* **渲染** —— 离屏 GPU 渲染（预乘 alpha、遮罩、层合成）、像素读回、可选的平台纹理共享。
* **编辑** —— 带撤销/重做的命令流；一个 `batch` 只占一步撤销；被拒绝的命令不改变模型。
* **动态** —— 确定性物理（摆锤 / 顶点链）与动作播放（曲线、循环、交叉淡化）。
* **动画模式** —— **实时模式**（`runtime.advance`，有状态物理）与**预渲染模式**：
  把表演烘焙成一条**无状态**的参数轨，带真正的时间轴、变速、关键帧与结构通道，
  因此可任意 `seek`、跨宿主逐位一致。规范见 [`../docs/animation-mode.md`](../docs/animation-mode.md)。
* **存储** —— `.amproj` 工程格式：目录模式与打包归档，逐资源校验和与校验器。
* **对外** —— 稳定的 C ABI（`am_call`）、同构的 wasm 绑定、JSON Schema，以及用于校验、
  渲染与打包的命令行工具。

### 仓库结构

仓库根目录**就是** Cargo 工作区根（不存在再套一层的 `engine/` 子目录）：

```text
├── crates/
│   ├── am-math      二维数学（Vec2 / Mat3 / Rect / 仿射）
│   ├── am-format    .amproj 读写、校验、打包、sha256
│   ├── am-model     描述层数据结构
│   ├── am-eval      参数求值、关键形插值、变形器级联
│   ├── am-render    离屏 GPU 渲染（混合模式、遮罩、层合成）
│   ├── am-doc       命令系统 + 撤销/重做
│   ├── am-physics   确定性物理
│   ├── am-motion    动作播放（曲线、循环、交叉淡化）
│   ├── am-core      会话门面 + JSON 方法分发（47 个方法）
│   ├── am-ffi       C ABI（cdylib/staticlib）+ bindings/include/anima.h
│   ├── am-wasm      wasm-bindgen 绑定
│   └── am-cli       anima 命令行工具
├── schemas/         JSON Schema（envelope / format / model / spec / command）
├── docs/
│   ├── ffi-contract.md   宿主对接的唯一契约
│   ├── format.md         .amproj 格式规范
│   └── mobile.md         Android / iOS 交付与接入
├── crates/am-ffi/bindings/include/anima.h   C ABI 头文件
├── .github/workflows/build.yml              六平台构建 + 发布为 Releases
└── samples/minimal                          最小可运行工程
```

### 快速开始

```bash
# 全量测试（约 300 个用例，含 GPU 渲染测试）
cargo test --workspace

# 只跑不依赖 GPU 的部分（CI / 无显卡环境）
cargo test --workspace --no-default-features

# 构建宿主链接用的 C ABI 库
cargo build -p am-ffi --release
#   → target/release/anima.dll（Windows）/ libanima.so / libanima.dylib

# 命令行工具
cargo run -p am-cli -- version
cargo run -p am-cli -- validate samples/minimal
cargo run -p am-cli -- render   samples/minimal -o preview.png --width 256 --height 256
```

宿主启动时应先调用 `system.capabilities`，只调用它报告存在的方法，而不要硬编码方法清单。
全部方法、参数与数据结构见 `docs/ffi-contract.md`。

### 设计约束（改代码前请先读）

1. **分层不可逆**：`am-format` 不依赖 `am-model`（格式层只管文件，不懂语义）；
   `am-eval` 不做任何 I/O；只有 `am-core` 知道全部东西。
2. **一切结构修改都走 `am-doc` 命令**，否则撤销栈与画面会不一致。
3. **确定性**：物理与动作里禁止随机数、禁止读墙上时钟；时间只由 `runtime.advance` 推进。
4. **id 只增不复用**：删除节点/纹理/参数后，其 id 永久作废。
5. **方法只增不改**：破坏性变更必须升版本（见 `docs/ffi-contract.md` §10）。
6. **GPU 是 feature**：`gpu`（默认开）关掉后 `renderer.*` 返回 `-32000`，其余功能不受影响。
7. **文档与代码同步**：改了协议就改 `docs/ffi-contract.md`，它是宿主的唯一依据。

### 坐标与时间约定

* 画布原点在**中心**，Y 轴向上，单位 = 画布像素。
* 纹理 UV 原点在**左上角**，范围 `[0, 1]`。
* 旋转角单位是**弧度**。
* 时间单位是**秒**，物理默认固定 60 Hz 步长。

### 测试策略

| 层次 | 位置 | 覆盖内容 |
| --- | --- | --- |
| 单元测试 | 各 crate `src/**/tests` | 数学、格式、求值、命令、物理、动作 |
| GPU 集成测试 | `crates/am-render/tests/render.rs` | 真实设备渲染 + 逐像素断言 |
| 协议契约测试 | `crates/am-core/tests/api.rs` | 21 个用例走 `dispatch_json`，锁死信封与方法形状 |
| Schema 一致性测试 | `crates/am-format/tests/schemas.rs` | schema 约束真实工程文件与真实调用数据 |
| C ABI 测试 | `crates/am-ffi/tests/ffi.rs` | 只通过 C ABI 驱动引擎，含渲染与事件回调 |
| CLI 测试 | `crates/am-cli/src/main.rs` | 建工程 → 校验 → 渲染 → 导出 |
| 示例工程 | `samples/minimal` | 手工编写的工程，覆盖格式与渲染链路 |

GPU 测试会真的创建适配器，单次约 30 秒；测试间用 `OnceLock<Mutex<Renderer>>` 复用设备。

### 当前状态

引擎核心（格式 / 求值 / 渲染 / 编辑 / 物理 / 动作 / C ABI / CLI / wasm）已完成并通过全量测试。
对外交付物已经齐了：C ABI 动态库与静态库、Android `.aar`、iOS `.xcframework`、
wasm-bindgen ES 模块，由 `.github/workflows/build.yml` 在六个平台上构建，发布到 Releases
时使用固定资产名，供宿主匿名直链拉取。

尚未完成的是**宿主侧的画面桥**：Windows 外部纹理桥、Android `SurfaceTexture` /
iOS `IOSurface` 绑定，以及网格自动生成与分层位图导入。

### 许可证

GNU Lesser General Public License v2.1 —— 见 [LICENSE](LICENSE)。
