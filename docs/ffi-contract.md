# Anima 引擎接口契约（FFI / JSON-RPC 风格）

> 本文档是**编辑器与查看器使用引擎的唯一依据**。
> 引擎侧实现：`crates/am-ffi/src/lib.rs`（C ABI）与 `crates/am-core/src/lib.rs`（方法分发）。
> 头文件：`crates/am-ffi/bindings/include/anima.h`；机器可校验的 schema 见 `schemas/`。
>
> 契约版本：引擎 `0.1.0`，格式 `amproj v1`。
>
> **即将落地（v0.2.0）**：新增**预渲染模式**与 `animation.*` 方法面（含烘焙、变速、
> 关键帧、结构通道）。规范见工作区根目录 [`../../docs/animation-mode.md`](../../docs/animation-mode.md)，
> 其中 §3 是方法面草案、§4 是数据结构、§9 是新增错误码。实现落地后本文档会同步补 §4.10。

---

## 1. 构建与产物

本仓库根目录就是 Cargo 工作区根（没有多余的 `engine/` 子目录）。

```bash
cargo build -p am-ffi --release          # Windows: target/release/anima.dll + anima.dll.lib
cargo test  --workspace                  # 全量测试（约 300 个用例）
```

| 平台 | 产物 | 说明 |
| --- | --- | --- |
| Windows | `anima.dll`、`anima.dll.lib` | `crate-type = ["cdylib", "staticlib", "rlib"]` |
| macOS / Linux | `libanima.dylib` / `libanima.so` | 同上 |
| Android | `anima-android.aar`（内含 `jni/<abi>/libanima.so`） | `--no-default-features`，无 GPU |
| iOS | `anima.xcframework`（内含静态库 `libanima.a`） | `staticlib`；App Store 不允许内嵌 dylib |
| Web | `anima_wasm.js` + `anima_wasm_bg.wasm`（wasm-bindgen ES 模块） | 方法名与参数完全一致 |

Android / iOS / Web 这三种产物由 `.github/workflows/build.yml` 构建，勾选
`publish_release` 后按固定资产名发布到 Releases，宿主仓库直接匿名直链消费
（见 `docs/mobile.md` §5）。

**关闭渲染后端**（体积敏感、或不需要引擎自绘）：

```bash
cargo build -p am-ffi --release --no-default-features   # renderer.* 返回 -32000
```

---

## 2. C ABI

```c
typedef struct am_engine am_engine;
typedef void (*am_event_cb)(const char* event, const char* payload, void* user_data);

const char* am_version(void);                                     /* 静态字符串 */
am_engine*  am_engine_new(void);                                  /* 空引擎 */
am_engine*  am_engine_new_from_file(const char* path);            /* 目录或 .amproj */
void        am_engine_free(am_engine* engine);

char*       am_call(am_engine*, const char* method, const char* params_json);
void        am_string_free(char* text);

size_t      am_frame_copy(am_engine*, uint8_t* out, size_t capacity);
void        am_set_event_callback(am_engine*, am_event_cb, void* user_data);
const char* am_last_error(void);                                  /* 线程本地 */
```

规则：

1. `am_call` **永远**返回 UTF-8 JSON 信封，必须用 `am_string_free` 释放。
2. `params_json` 允许 `NULL` 或空串（等价于 `{}`）。
3. 空指针、非法 UTF-8 都返回结构化错误，不会崩溃；引擎内部 panic 被 `catch_unwind`
   拦下并转成 `-32603`。
4. `am_engine*` **不是**线程安全的：一个句柄只能被一个线程使用（不同句柄可并行）。
5. `am_frame_copy`：`out == NULL` 或 `capacity` 不足时只返回所需字节数，**不写入**任何数据。

### 事件

| 事件 | 时机 | payload |
| --- | --- | --- |
| `ready` | `am_engine_new_from_file` 载入成功 | `{}` |
| `error` | 任何 `am_call` 返回失败信封 | 完整错误信封 |

回调期间不要再次调用 `am_call`（同一引擎不可重入）。

---

## 3. 信封与错误码

```json
{ "ok": true,  "result": { } }
{ "ok": false, "error": { "code": -32602, "message": "参数不存在：AngleX" } }
```

| code | 含义 |
| --- | --- |
| `-32600` | 请求非法（JSON 解析失败、指针为空、编码非法） |
| `-32601` | 方法不存在 |
| `-32602` | 参数非法（缺字段、类型不符、id 不存在、命令执行失败） |
| `-32603` | 引擎内部错误 |
| `-32000` | 平台不支持（例如本构建没有渲染后端、渲染器未初始化） |

**宿主必须先调用 `system.capabilities`**，据 `methods` 与 `renderer` 决定可用功能，
不要硬编码方法清单。

---

## 4. 方法参考

### 4.1 `system.*`

| 方法 | 参数 | 结果 |
| --- | --- | --- |
| `system.ping` | – | `{ "pong": true, "version": "0.1.0" }` |
| `system.version` | – | `{ "name", "version", "format": "amproj", "format_version": 1 }` |
| `system.capabilities` | – | `{ "renderer": bool, "physics": bool, "motion": bool, "expressions": bool, "wasm": bool, "gpu_backend": string\|null, "methods": [string] }` |

### 4.2 `project.*`

| 方法 | 参数 | 结果 |
| --- | --- | --- |
| `project.new` | `{ "name": "demo", "width": 1024, "height": 1024 }` | `{ name, width, height }`，清空文档与撤销历史 |
| `project.load` | `{ "path": "D:/p/demo" }`（目录或 `.amproj`） | `{ path, nodes, parameters, motions, expressions }` |
| `project.save` | `{ "path": "D:/p/demo" }` | `{ path }`；目录不存在则按 `info.json` 新建 |
| `project.validate` | – | `{ "ok": bool, "issues": [ { "code", "message", "path" } ] }` |
| `project.spec` | – | 完整描述层（见 §5.1） |
| `project.set_spec` | `{ "spec": { … } }` | `{ "ok": true }`，整体替换并清空撤销历史 |

### 4.3 `doc.*`

| 方法 | 参数 | 结果 |
| --- | --- | --- |
| `doc.command` | `{ "command": { "op": "node_create", … } }` | `{ "effects": [...], "revision": n }` |
| `doc.undo` / `doc.redo` | – | `{ "label": string\|null, "revision": n }` |
| `doc.history` | – | `{ "undo": [label], "redo": [label], "revision", "dirty", "can_undo", "can_redo" }` |
| `doc.model` | – | 结构模型 JSON（§5.2） |
| `doc.set_model` | `{ "model": { … } }` | `{ "ok": true }`，替换模型（撤销历史清空） |
| `doc.evaluate` | – | `{ "drawables": n, "nodes": n, "canvas": {…} }` |

**所有结构编辑都必须走 `doc.command`**，否则撤销栈会与画面不一致。
命令清单见 §5.4，`effects` 见 §5.5。

### 4.4 `runtime.*`

| 方法 | 参数 | 结果 |
| --- | --- | --- |
| `runtime.set_param` | `{ "id": "AngleX", "value": 20 }` 或 `{ "id": "AngleX", "normalized": 0.5 }` | `{ id, value }`（自动钳制到参数范围） |
| `runtime.params` | – | `{ "AngleX": 20.0, … }` |
| `runtime.reset_params` | – | `{ "ok": true }` |
| `runtime.advance` | `{ "dt": 0.0167 }`（缺省 1/60） | `{ "time": 1.25, "frame": 75 }` |
| `runtime.set_time` | `{ "time": 2.0 }` | `{ "time": 2.0 }`（只改时钟，不推进物理） |
| `runtime.pause` / `runtime.resume` | – | `{ "paused": bool }` |
| `runtime.state` | – | `{ time, frame, paused, params, motion, expression }` |
| `runtime.scene` | – | 求值后的场景（§5.3） |

**推进顺序固定为**：动作 → 物理 → 时钟。`runtime.advance` 是唯一推进时间的入口。

### 4.5 `motion.*`

| 方法 | 参数 | 结果 |
| --- | --- | --- |
| `motion.list` | – | `[ { id, name, duration, looping } ]` |
| `motion.play` | `{ "id": "idle", "looping": false, "speed": 1.0, "fade_in": 0.2, "from_time": 0 }`（后四项可选） | 播放状态 |
| `motion.stop` / `motion.pause` / `motion.resume` | – | 状态 |
| `motion.seek` | `{ "time": 0.5 }` | 播放状态 |
| `motion.state` | – | `{ id, time, playing, paused, speed, fade }` |

### 4.6 `expression.*`

| 方法 | 参数 | 结果 |
| --- | --- | --- |
| `expression.list` | – | `[ { id, name, parameters } ]` |
| `expression.set` | `{ "id": "smile", "weight": 1.0 }` | `{ id, weight }`；权重是相对**当前取值**的插值 |

### 4.7 `physics.*`

| 方法 | 参数 | 结果 |
| --- | --- | --- |
| `physics.info` | – | `{ enabled, settings, fps, steps, settled }` |
| `physics.step` | `{ "dt": 0.0167 }` | `{ "steps": n }`（单步推进，供编辑器逐帧调试） |
| `physics.reset` | – | `{ "ok": true }` |

### 4.8 `renderer.*`（需要 GPU 后端）

| 方法 | 参数 | 结果 |
| --- | --- | --- |
| `renderer.info` | – | `{ initialized, supported, width, height, format, adapter, textures }` |
| `renderer.init` | `{ "width": 512, "height": 512 }` | `{ width, height }`；失败返回 `-32000` |
| `renderer.resize` | `{ "width": 512, "height": 512 }` | `{ width, height }` |
| `renderer.set_view` | `{ "center": [x, y], "zoom": 1.5 }`（都可选） | `{ center, zoom }` |
| `renderer.set_texture` | `{ "index": 0, "path": "assets/images/0.png" }` | `{ index, path }` |
| `renderer.clear_textures` | – | `{ "ok": true }` |
| `renderer.render` | – | `{ width, height, generation }`；像素留在引擎里，用 `am_frame_copy` 取 |
| `renderer.frame` | – | `{ width, height, bytes, stride, generation, format: "rgba8_unorm" }` |
| `renderer.save_png` | `{ "path": "out.png" }` | `{ path, width, height }` |

渲染管线细节：输出 `Rgba8Unorm`，**预乘 alpha**，第 0 行是画面顶部。
`renderer.render` 会自动上传模型中登记但尚未上传的纹理（按 `textures[i].asset` 路径）。

### 4.9 `diagnostics.*`

| 方法 | 结果 |
| --- | --- |
| `diagnostics.stats` | `{ nodes, parameters, textures, motions, expressions, physics, drawables, revision, dirty, frame }` |

---

## 5. 数据结构

### 5.1 描述层（`project.spec` / `spec/` 目录）

```jsonc
{
  "model":       { /* §5.2 */ },
  "physics":     { "enabled": true, "fps": 60.0, "gravity": { "x": 0.0, "y": -1.0 },
                   "wind": { "x": 0.0, "y": 0.0 }, "settings": [ … ] },
  "pose":        { "groups": [ { "id", "name", "parts": [ { "node", "visible" } ] } ] },
  "settings":    { "display_name", "default_motion", "default_expression", "physics": true,
                   "auto_blink": {…}, "auto_breath": {…}, "lip_sync": {…},
                   "parameter_defaults": { "AngleX": 0.0 }, "motion_groups": {…}, "expression_groups": {…} },
  "motions":     [ { "id", "name", "duration", "looping", "fps", "fade_in", "fade_out", "curves": [ … ] } ],
  "expressions": [ { "id", "name", "fade_in", "fade_out", "parameters": [ { "parameter", "value", "blend", "weight" } ] } ],
  "config":      { … }   // 可选
}
```

### 5.2 结构模型（`doc.model`）

```jsonc
{
  "version": 1,
  "id": "model-…",
  "name": "demo",
  "canvas": { "width": 1024.0, "height": 1024.0, "origin": { "x": 0.0, "y": 0.0 }, "pixels_per_unit": 1.0 },
  "textures": [ { "id": "tex-…", "asset": "assets/images/0.png", "width": 512, "height": 512, "atlas": false } ],
  "nodes": [
    {
      "id": "node-…", "name": "Body", "parent": null, "visible": true, "locked": false,
      "draw_order": 0, "kind": "drawable",            // part | drawable | warp_deformer | rotation_deformer
      "drawable": { "texture": 0, "uv_rect": null,
                    "mesh": { "vertices": [ { "x": -64.0, "y": -64.0 } ],
                              "uvs": [ { "x": 0.0, "y": 0.0 } ], "indices": [0, 1, 2] },
                    "opacity": 1.0, "blend": "normal", "masks": ["node-…"], "inverted_mask": false, "culling": false },
      "warp": null, "rotation": null,
      "keyforms": { "AngleX": [ { "value": 10.0, "blend": "normal", "vertices": [ { "x": 0.0, "y": 0.0 } ],
                                 "opacity": null, "control_points": null, "rotation": null, "draw_order": null } ] }
    }
  ],
  "parameters": [
    { "id": "AngleX", "name": "Angle X", "group": null, "min": -30.0, "max": 30.0, "default": 0.0,
      "keys": [-30.0, 0.0, 30.0], "is_blend_shape": false, "repeat": false, "auto": false,
      "weight": 1.0, "comment": null }
  ],
  "parameter_groups": [ { "id": "group-…", "name": "头部" } ]
}
```

**id 是不可变字符串**（非空 ASCII 字母数字/`_`/`-`，≤128 字节；参数惯例 PascalCase 如
`AngleX`，其余惯例 kebab/snake 如 `node-body`），永远不要复用已删除的 id；
引用一律用 id，不要用下标（只有 `drawable.texture` 例外，它是纹理下标）。

### 5.3 场景（`runtime.scene`）

```jsonc
{
  "canvas": { … },
  "drawables": [
    { "node": "node-…", "name": "Body", "texture": 0, "uv_rect": null,
      "vertices": [ { "x": 0.0, "y": 0.0 } ], "uvs": [ { "x": 0.0, "y": 0.0 } ], "indices": [0,1,2],
      "opacity": 1.0, "blend": "normal", "masks": ["node-…"], "inverted_mask": false,
      "culling": false, "visible": true, "draw_order": 0 }
  ],
  "nodes": {
    "node-…": { "id", "name", "kind", "parent", "visible", "opacity",
                "world_affine": { "a": 1.0, "b": 0.0, "c": 0.0, "d": 1.0, "tx": 0.0, "ty": 0.0 },
                "control_points_parent": [ { "x": 0.0, "y": 0.0 } ],
                "control_points_canvas": [ { "x": 0.0, "y": 0.0 } ],
                "rotation_pivot_canvas": { "x": 0.0, "y": 0.0 },
                "rotation_handle_canvas": { "x": 0.0, "y": 0.0 },
                "deformed": false, "draw_order": 0 }
  }
}
```

> **向量与矩阵的 JSON 形状**：二维向量序列化为 `{ "x": …, "y": … }`，
> 仿射矩阵序列化为 `{ "a","b","c","d","tx","ty" }`（2×3）。
> 输入侧（命令参数）同时接受 `[x, y]` 数组写法，输出侧一律是对象。
> `renderer.set_view` 的 `center` 是例外：输入输出都用 `[x, y]`。

* `drawables` 已按绘制顺序排好，宿主**按数组顺序**绘制即可。
* `vertices` 已是画布坐标（含变形），`uvs` 是归一化纹理坐标，`indices` 是三角形索引。
* `nodes` 是部件树/变形器的可视信息，供编辑器画控件（旋转手柄、变形器网格）。
* `blend`：`normal` / `multiply` / `screen` / `additive`。

### 5.4 编辑命令（`doc.command`）

统一形状 `{ "op": "<name>", … }`：

| op | 字段 |
| --- | --- |
| `set_canvas` | `width`, `height`, `origin?` |
| `node_create` | `kind`(`part`/`drawable`/`warp_deformer`/`rotation_deformer`), `name`, `parent?`, `rect?`, `rows?`, `cols?` |
| `node_delete` | `node` |
| `node_rename` | `node`, `name` |
| `node_reparent` | `node`, `parent?`（成环返回 `-32602`） |
| `node_set_visible` | `node`, `visible` |
| `node_set_locked` | `node`, `locked` |
| `node_set_draw_order` | `node`, `draw_order` |
| `node_set_rotation` | `node`, `angle?`, `position?`, `scale?`, `origin?`, `handle_length?` |
| `mesh_set` | `node`, `vertices`, `uvs`, `indices` |
| `mesh_set_vertices` / `mesh_set_uvs` / `mesh_set_indices` | `node`, 对应数组 |
| `warp_set_control_points` | `node`, `points`（长度必须等于 `(rows+1)*(cols+1)`） |
| `warp_resize` | `node`, `rows`, `cols` |
| `drawable_set_texture` | `node`, `texture?`（`null` 表示取消贴图） |
| `drawable_set_uv_rect` | `node`, `uv_rect?` |
| `drawable_set_blend` | `node`, `blend` |
| `drawable_set_opacity` | `node`, `opacity` |
| `drawable_set_masks` | `node`, `masks`, `inverted` |
| `drawable_set_culling` | `node`, `culling` |
| `keyform_record` | `node`, `parameter`, `value`, `blend?`, `vertices?`, `opacity?`, `control_points?`, `rotation?`, `draw_order?` |
| `keyform_remove` | `node`, `parameter`, `value` |
| `parameter_add` | `id?`, `name`, `min`, `max`, `default`, `group?`, `weight?`, `repeat` |
| `parameter_remove` | `parameter` |
| `parameter_set_range` | `parameter`, `min`, `max`, `default` |
| `texture_add` | `asset`, `id?`, `width`, `height`, `atlas` |
| `texture_remove` | `index`（删除后所有绘制对象的纹理下标会自动前移） |
| `batch` | `commands: [...]`, `label?` —— **整批只占一步撤销** |

约定：

* 顶点/控制点数量不符、参数不存在、成环等都会返回 `-32602`，并且**模型保持不变**。
* 连续操作（拖动滑杆）请用 `batch` 合并成一步撤销。
* 参数取值变化**不**进入撤销栈（它是运行时状态，不是结构编辑）。

### 5.5 `effects`

`["structure"]`、`{ "node": "node-…" }`、`["parameters"]`、`["textures"]`、`["canvas"]`、
`["physics"]`、`["motions"]`、`["expressions"]`。
编辑器可据此只刷新受影响的区域；拿不准时重新调用 `runtime.scene` 即可。

---

## 6. 坐标系与时间

* 原点在**画布中心**，**Y 轴向上**，单位 = 画布像素（`canvas.width/height` 决定可见范围）。
* 视图（`renderer.set_view`）：`center` 是画布坐标下的视口中心，`zoom` 是缩放倍数（1 = 1 像素对应 1 画布单位）。
* 时间单位是**秒**（`f64` 内部，JSON 里是数字）；默认 60 fps。
* 物理使用固定步长（默认 1/60 秒），单次 `advance` 最多推进 8 步，超出部分丢弃 —— 保证确定性。

---

## 7. 生命周期与线程

```text
am_engine_new() ──► 一组 am_call(...) ──► am_engine_free()
                        │
                        └── renderer.init ──► renderer.render ──► am_frame_copy
```

* 一个句柄同一时刻只被一个线程使用；建议「一个窗口一个引擎句柄」。
* `am_call` 不是可重入的：不要在事件回调里调用。
* 引擎不拥有任何窗口/GL 上下文：它自己创建 wgpu 设备并渲染到离屏纹理。

---

## 8. 版本与能力探测

```jsonc
// system.capabilities 示例
{
  "renderer": true, "physics": true, "motion": true, "expressions": true, "wasm": false,
  "gpu_backend": "NVIDIA GeForce RTX 4060 (Vulkan)",
  "methods": ["system.ping", "…"]
}
```

宿主必须：

1. 启动时调用一次 `system.capabilities`；
2. 只调用 `methods` 里存在的方法；
3. `renderer == false` 时使用自己的绘制路径（例如 Web 端用 Canvas 画 `runtime.scene`）。

---

## 9. Flutter 接入

### 9.1 Dart FFI

```dart
final lib = DynamicLibrary.open('anima.dll');
final _new    = lib.lookupFunction<Pointer<Void> Function(), Pointer<Void> Function()>('am_engine_new');
final _call   = lib.lookupFunction<Pointer<Utf8> Function(Pointer<Void>, Pointer<Utf8>, Pointer<Utf8>),
                                   Pointer<Utf8> Function(Pointer<Void>, Pointer<Utf8>, Pointer<Utf8>)>('am_call');
final _free   = lib.lookupFunction<Void Function(Pointer<Utf8>), void Function(Pointer<Utf8>)>('am_string_free');
final _efree  = lib.lookupFunction<Void Function(Pointer<Void>), void Function(Pointer<Void>)>('am_engine_free');

Map<String, dynamic> call(Pointer<Void> engine, String method, [Object? params]) {
  final m = method.toNativeUtf8();
  final p = jsonEncode(params ?? const {}).toNativeUtf8();
  final raw = _call(engine, m, p);
  final text = raw.toDartString();
  _free(raw); malloc.free(m); malloc.free(p);
  return jsonDecode(text) as Map<String, dynamic>;
}
```

建议在 Dart 侧封装成 `AnimaEngine` 类：`call()` 检查 `ok`，失败时抛带 `code`/`message` 的异常，
并按 `methods` 做能力探测。

### 9.2 显示引擎画面

引擎渲染到自己的离屏纹理，宿主负责把它显示出来：

* **Windows（推荐，P1）**：共享纹理。引擎用 `wgpu` 的 D3D12 后端渲染到
  `ID3D11Texture2D`/共享 NT 句柄，Flutter 侧通过
  `FlutterDesktopTextureRegistrarRegisterExternalTexture` 注册。
  相关接口（`am_renderer_texture_info`）仍在设计，落地前先用下面的路径。
* **通用兜底（P0，立即可用）**：`renderer.render` + `am_frame_copy` 取 RGBA 像素，
  再喂给 Flutter 的 `ui.decodeImageFromPixels` / `Image.memory`。
  512×512 在 60 fps 下带宽约 63 MB/s，够用但不适合 4K。

### 9.3 与编辑器的职责边界

| 事项 | 归属 |
| --- | --- |
| 模型结构、求值、物理、动作、渲染 | 引擎（本仓库） |
| 撤销栈、命令生成 | 引擎（`doc.*`），编辑器只负责发命令 |
| 画布控件、属性面板、时间轴 UI | 编辑器 |
| 工程文件读写 | 引擎（`project.*`），编辑器不要自己解析 |
| 贴图素材导入 | 编辑器调用 `project.spec` / `doc.command` 写入 |

---

## 10. 兼容性政策

* **方法只增不改**：已有方法的参数与结果形状保持兼容；破坏性变更必须升 `version`。
* 新增字段一律可选（`null` 或缺省 = 使用默认值），宿主解析时忽略未知字段。
* 枚举新增取值时，宿主必须有 `default` 分支。
* 引擎版本可在 `system.version` / `am_version()` 取得；工程文件里记录 `min_sdk`。
