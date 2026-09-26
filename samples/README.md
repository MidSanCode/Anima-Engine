# 示例工程

## `minimal/`

最小可运行的 `.amproj` 工程，用来验证「格式 → 引擎 → 渲染」整条链路：

```text
minimal/
├── info.json                       工程身份证
├── registry.json                   资源登记
├── assets/images/0.png             1×1 PNG（示例贴图）
├── metadata/images/0.png.json      资源档案（sha256 / 尺寸）
└── spec/
    ├── model.json                  画布 + 部件树 + 一个绘制对象 + 一个参数
    └── model.settings.json         显示名等设置
```

命令行验证：

```bash
cd engine
cargo build -p am-cli

target/debug/anima validate samples/minimal
target/debug/anima info     samples/minimal
target/debug/anima render   samples/minimal -o preview.png --width 256 --height 256
target/debug/anima export   samples/minimal minimal.amproj
```

`render` 会输出一张 256×256 的 PNG：画布中央一个 256×256 的方块
（贴图是 1×1 的纯色，被拉伸覆盖整个网格）。

用编辑器打开：

```bash
# 编辑器（另一个仓库）
flutter run -d windows -- --project ../engine/samples/minimal
```

> 提示：`metadata/` 的目录结构**镜像** `assets/` 去掉 `assets/` 前缀后的路径。
> `assets/images/0.png` 的档案是 `metadata/images/0.png.json`。
