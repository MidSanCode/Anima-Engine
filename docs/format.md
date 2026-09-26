# `.amproj` 工程格式规范 v1

> 实现：`crates/am-format`（格式层，**唯一权威实现**）+ `crates/am-model`（描述层数据结构）。
> 参考标准：LGDF v2.0（`temp/example/`，用户提供的参考实现）。
> 扩展名：目录模式无后缀；打包模式 `.amproj`（ZIP + Deflate）。
> 契约版本：`format = "amproj"`，`min_sdk = 1`。机器可校验的 schema 见 `schemas/`。

---

## 1. 设计原则

1. **目录模式是规范形态**，压缩包只是它的打包形式。任何时刻都可以解包成目录来编辑、
   再用目录模式重新打包，内容不变。
2. **人可读、工具友好**：JSON 用 TAB 缩进、UTF-8 无 BOM、键名 snake_case。
   手工改工程文件不应该破坏格式。
3. **资源与元数据分离**：`assets/` 放二进制素材，`metadata/` 放每个素材的档案
   （大小、sha256、尺寸等）。校验时两者必须一一对应。
4. **描述层不参与哈希**：`spec/` 是内容本身，`assets/` 的完整性由 `metadata/*.json`
   与包级 `.amproj.sha256` 保证。
5. **向后兼容**：新增字段一律可选；未知字段原样保留（`extra` 容器）。

---

## 2. 目录结构

```text
<project>/
├── info.json                 工程身份证（必需）
├── registry.json             资源登记表（必需）
├── assets/                   资源文件（路径必须注册在 registry.json）
│   └── images/0.png
├── metadata/                 资源档案，路径 = assets/ 去掉前缀后 + ".json"
│   └── images/0.png.json
├── spec/                     描述层（模型、物理、动作、表情…）
│   ├── model.json            结构模型（必需，引擎的最小可运行集合）
│   ├── physics.json          物理设定（可选）
│   ├── pose.json             姿势组（可选）
│   ├── model.settings.json   显示/默认项设置（可选）
│   ├── config.json           工程配置（可选）
│   ├── overview.md           说明文档（可选，不参与校验）
│   ├── steps.json            制作步骤（可选）
│   ├── motions/<id>.motion.json
│   └── expressions/<id>.exp.json
├── work/                     中间产物（不导出）
└── dist/                     导出产物（不导出）
```

导出（打包）时只包含 `assets/`、`metadata/`、`spec/` 三类前缀下的文件。

---

## 3. 文件规范

### 3.1 `info.json`

```jsonc
{
	"format": "amproj",          // 固定
	"min_sdk": 1,                // 读取者要求的最低 SDK 版本
	"name": "demo",              // ^[a-z0-9_-]+$，同时用作目录名
	"display_name": "示例模型",   // 可选，UI 显示名（可用任意语言）
	"description": "",
	"author": null,
	"author_sign": null,         // { "algorithm", "signature", "signed_at" }
	"license": null,
	"tags": [],
	"created_time": 1730000000,  // Unix 秒
	"last_update_time": 1730000000,
	"version": 1                 // 工程版本号，>= 1，每次保存自增
}
```

### 3.2 `registry.json`

```jsonc
{
	"registered_files": ["assets/images/0.png"],  // 必须是 assets/ 下的相对路径，排序去重
	"asset_count": 1,
	"timestamp_sign": null
}
```

### 3.3 `metadata/<assets 去掉前缀>.json`

> `assets/images/0.png` → `metadata/images/0.png.json`

```jsonc
{
	"path": "assets/images/0.png",
	"type": "image",             // image | audio | text | model | other
	"mime": "image/png",
	"format": "png",
	"size": 12345,
	"sha256": "…64 位小写十六进制…",
	"hash_algorithm": "sha256",
	"created_time": 1730000000,
	"last_update_time": 1730000000,
	"width": 512,                // 分类相关字段，直接平铺
	"height": 512
}
```

### 3.4 `spec/` 描述层

结构见 `docs/ffi-contract.md` §5.1。要点：

* `spec/model.json` 是引擎运行的最小集合，缺失视为工程损坏。
* `spec/motions/<id>.motion.json` 与 `spec/expressions/<id>.exp.json` **文件名即 id**
  （`^[a-z0-9_-]+$`），id 必须与文件内容里的 `id` 一致。
* 其余文件缺失时按默认值处理（物理为空、无动作、无表情）。

---

## 4. 编码约定

| 项目 | 规定 |
| --- | --- |
| 编码 | UTF-8，**不带 BOM** |
| 缩进 | **TAB**（`\t`） |
| 换行 | `\n` |
| 键名 | snake_case |
| 枚举 | 小写下划线字符串（`part` / `drawable` / `warp_deformer` / `rotation_deformer`） |
| id | `^[A-Za-z0-9_-]{1,128}$`（非空 ASCII 字母数字/下划线/连字符），全局唯一，删除后**不得复用**；惯例：参数用 PascalCase（`AngleX`），其余用 kebab/snake（`node-body`） |
| 名称 | 工程目录名与 `info.json` 的 `name` 更严格：`^[a-z0-9_-]+$`；`display_name` 不受限 |
| 时间 | Unix 秒（整数） |
| 颜色 | `#rrggbb` 或 `#rrggbbaa` |
| 数值 | 不使用 `NaN` / `Infinity` |

JSON 中的 `null` 与字段缺省等价（都是「使用默认值」）。

---

## 5. 打包（`.amproj`）

1. 校验通过后，按 `assets/`、`metadata/`、`spec/` 前缀收集文件；
2. 用 ZIP + Deflate 打包，**条目路径使用 `/`**、按字典序排列（保证可复现）；
3. 写出同名旁路文件 `<name>.amproj.sha256`，内容为包文件的 sha256 十六进制；
4. 导入时校验：包级 sha256（若存在旁路文件）→ `info.json` 的 `format`/`min_sdk`
   → 逐资源 sha256。

目录模式没有包级 sha256，资源完整性由 `metadata/*.json` 的 `sha256` 保证。

---

## 6. 校验

`anima validate <project>` / `project.validate` 会依次执行：

1. **格式层**（`am-format`）：文件存在性、JSON 可解析、路径规范、资源与元数据配对、
   sha256 格式与内容、UTF-8 合法性；
2. **模型层**（`am-model`）：节点树、网格、关键形、参数、纹理引用的一致性。

### 6.1 格式层问题码

| 码 | 含义 |
| --- | --- |
| `MISSING_INFO` / `BAD_INFO_JSON` | 缺少或无法解析 `info.json` |
| `FORMAT_MISMATCH` | `format` 不是 `amproj` |
| `MIN_SDK_TOO_NEW` | `min_sdk` 高于当前 SDK |
| `NAME_INVALID` | `name` 不满足 `^[a-z0-9_-]+$` |
| `VERSION_INVALID` / `TIME_ORDER` | 版本号非法 / `last_update_time` 早于 `created_time` |
| `MISSING_REGISTRY` | 缺少 `registry.json` |
| `DUP_REGISTERED` | `registered_files` 有重复项 |
| `NOT_UNDER_ASSETS` | 注册路径不在 `assets/` 下 |
| `MISSING_ASSET` | 注册了但文件不存在 |
| `NO_METADATA` | 资源缺少元数据文件 |
| `ORPHAN_METADATA` | 元数据没有对应资源 |
| `UNREGISTERED_ASSET` | 存在游离资源（未注册） |
| `BAD_METADATA_JSON` | 元数据无法解析 |
| `BAD_SHA256_FORMAT` | `sha256` 不是 64 位小写十六进制 |
| `INVALID_UTF8` | 文本资源不是合法 UTF-8 |
| `SPEC_JSON_INVALID` | `spec/` 下的 JSON 无法解析 |

### 6.2 模型层问题码

`MODEL_ID_INVALID`、`MODEL_NAME_EMPTY`、`NODE_ID_INVALID`、`NODE_ID_DUPLICATE`、
`NODE_NAME_EMPTY`、`NODE_DATA_MISSING`、`NODE_DATA_UNEXPECTED`、`NODE_SELF_PARENT`、
`NODE_PARENT_MISSING`、`NODE_PARENT_CYCLE`、`MESH_INVALID`、`MESH_NOT_FINITE`、
`TEXTURE_INDEX_OUT_OF_RANGE`、`MASK_NODE_MISSING`、`MASK_SELF_REFERENCE`、
`WARP_CONTROL_POINTS_INCOMPLETE`、`KEYFORM_ORPHAN_PARAMETER`、`KEYFORM_LIST_EMPTY`、
`KEYFORM_NOT_SORTED`、`KEYFORM_VALUE_NOT_FINITE`、`KEYFORM_VERTEX_COUNT_MISMATCH`、
`KEYFORM_CONTROL_POINT_COUNT_MISMATCH`、`KEYFORM_CONTROL_POINT_UNEXPECTED`、
`KEYFORM_ROTATION_UNEXPECTED`、`PARAMETER_ID_INVALID`、`PARAMETER_ID_DUPLICATE`、
`PARAMETER_NAME_EMPTY`、`PARAMETER_NAME_DUPLICATE`、`PARAMETER_RANGE_INVALID`、
`PARAMETER_RANGE_NOT_FINITE`、`PARAMETER_DEFAULT_OUT_OF_RANGE`、`PARAMETER_GROUP_ORPHAN`、
`TEXTURE_ID_DUPLICATE`、`TEXTURE_ASSET_EMPTY`、`TEXTURE_ASSET_NOT_IN_ASSETS`。

每个问题都带 `code`、`message`，以及（若有）`path`（相对工程根的文件或节点 id）。

---

## 7. 坐标与单位

* 画布原点在**中心**，Y 轴向上，单位 = 画布像素；
* 纹理 UV 原点在**左上角**，取值 `[0, 1]`；
* 网格 `vertices` 在画布坐标（局部坐标，受父级变形器影响），`uvs` 与 `vertices` 一一对应；
* 变形器控制点顺序为**行优先**：`index = row * (cols + 1) + col`；
* 旋转角单位为**弧度**。

---

## 8. 与参考标准的对应关系

| 参考标准 | Anima | 说明 |
| --- | --- | --- |
| 工程目录 + 压缩包 | 目录 + `.amproj` | 相同 |
| `info.json` / `registry.json` | 同名同义 | 字段做了精简与重命名 |
| `assets/` + `metadata/` | 同名同义 | sha256 校验策略一致 |
| 模型/物理/动作/表情描述 | `spec/` | Anima 自有的结构，语义可映射 |
| 第三方专有二进制模型 | 不支持 | 不做逆向，仅保留数据映射的可能 |

第三方向导入导出（把参考标准的工程读进来 / 导出去）属于后续工作，格式层已经
预留了 `spec/` 与 `assets/` 的清晰边界，落地时只需要新增一个转换器。
