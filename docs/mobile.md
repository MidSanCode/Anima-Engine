# 移动端接入方案（Android / iOS）

> 结论先行：**核心就是同一个 `anima` 库换 ABI 交付**。
> Android 交付 `.aar`（jniLibs 打包），iOS 交付 `.xcframework`（staticlib）。
> 引擎零代码改动 —— C ABI 不分平台，`docs/ffi-contract.md` 的契约原样适用。
> 工作流已就绪：`.github/workflows/build.yml` 的 `android` / `ios` job 一键产出。

---

## 1. Android

### 1.1 交付形态

`.aar` 压缩包，内含：

```text
anima-android.aar
├── AndroidManifest.xml        空壳（AGP 8+ 由宿主侧 namespace 命名）
├── classes.jar                占位（暂无 Java API）
└── jni/
    ├── arm64-v8a/libanima.so      真机（主流）
    ├── armeabi-v7a/libanima.so    老设备
    ├── x86_64/libanima.so         模拟器
    └── x86/libanima.so            老模拟器
```

宿主侧接入：

```kotlin
// build.gradle.kts
implementation(files("libs/anima-android.aar"))

// 运行时
System.loadLibrary("anima")
// 之后按 anima.h 用 JNI 调 am_engine_new / am_call / …
// 或包一层 external fun amCall(engine: Long, method: String, params: String): String
```

### 1.2 GPU 与画面（当前阶段的取舍）

* CI 产物用 `--no-default-features`：**不含 wgpu**。体积小、兼容面广（无 GPU 驱动差异），
  数据层/求值/物理/动作在手机上完整可用。
* 画面路线（推荐）：引擎 `runtime.scene`（JSON）→ 宿主 Kotlin/Java 侧用
  OpenGL ES / Vulkan 绘制三角形。`runtime.scene` 的 drawables 已排好绘制顺序，
  顶点/UV/混合模式/遮罩信息齐全，宿主渲染器只是把 `am-render` 的管线复刻一遍。
* 画面路线（后期）：把 `gpu` feature 打开，wgpu 走 Vulkan/GLES 后端直接离屏渲染，
  通过 `SurfaceTexture` / `AHardwareBuffer` 共享给 `GLTexture`/`ImageReader`。
  这一层的兼容性风险（老 GPU 驱动）较高，放在有真机矩阵之后。

### 1.3 已知约束

* `minSdkVersion 21`（`cargo-ndk -p 21`；wgpu 的 GLES 后端实际要求更高，无 GPU 路线不受影响）。
* `libanima.so` 若出现 `text relocations` 报错（老旧 NDK 产物问题），需 API ≥ 23；
  r27c 产物不会。
* 16 KB page size（Android 15+ 对 arm64 的要求）：NDK r27+ 默认对齐 16 KB，
  r27c 产物满足；如换 NDK 版本需要复核。
* Play 上架：纯 native 库无额外要求；不需要 `android:extractNativeLibs` 调整
  （AGP 默认 `useLegacyPackaging=false`，so 直接从 APK 映射加载）。

### 1.4 Android 上 Flutter（editor/viewer 的移动版）

编辑器/查看器仓库的 CI 直接消费本仓库发布的 `.aar`，**没有中间插件层**：

1. CI 下载 `anima-engine-android.aar`，解出 `jni/<abi>/libanima.so`，放到
   `android/app/src/main/jniLibs/<abi>/`（`src/main/jniLibs` 是 AGP 默认源目录，
   随 APK 打包；`.aar` 里的 `classes.jar` 是空壳，不需要 `implementation` 它）；
2. Dart FFI 在 Android 上 `DynamicLibrary.open("libanima.so")` —— 契约与 Windows 完全一致；
3. 画面：短期用「引擎 JSON scene + Flutter 侧 `CustomPainter`/着色器」，
   中期切换 `Texture` + 外部纹理（Android 的外部纹理桥走
   `FlutterDesktopTextureRegistrar` 的 Android 等价物：`TexturePlugin` + `SurfaceTexture`，
   与 Windows 的 D3D11 桥是两套独立实现，接口已在 ffi-contract §9.2 预留）。

---

## 2. iOS

### 2.1 交付形态

`.xcframework`，**staticlib**（App Store 禁止 App 内嵌 dylib，必须用 `.a` 静态链接）：

```text
anima.xcframework
├── Info.plist
├── ios-arm64/libanima.a + Headers/anima.h                     真机
└── ios-arm64_x86_64-simulator/libanima.a + Headers/anima.h    模拟器（arm64 + Intel 双架构已 lipo）
```

宿主侧接入（Xcode）：

1. 把 `anima.xcframework` 拖进工程，General → Frameworks → Embed **Do Not Embed**
   （静态库不嵌入）；
2. Swift 桥接头里 `#include "anima.h"`；
3. `am_engine_new()` 直接调用 —— 链接器把引擎编进 App 主二进制。

### 2.2 GPU 与画面

* iOS 产物**保留 GPU 后端**（wgpu Metal）——iOS 是渲染平台，且 Metal 驱动质量高、
  碎片化小，是移动端最可靠的 wgpu 目标。真机自绘完全可行。
* 画面路线：`am_engine` + `renderer.render` 离屏渲染 → 像素回读 →
  `MACH_PORT`/`IOSurface` 共享给 Flutter `Texture`（后期），
  短期 `decodeImageFromPixels` / `Image.memory` 兜底路径与桌面一致。
* 模拟器：wgpu Metal 在模拟器上可用（Xcode 15+），但性能不代表真机。

### 2.3 已知约束

* **Bitcode 已废弃**（Xcode 14+），rustc 产物无需处理。
* `aarch64-apple-ios-sim` 只服务 Apple Silicon 的模拟器；Intel 模拟器是
  `x86_64-apple-ios`，两者必须 lipo 成一个模拟器 slice，`xcodebuild -create-xcframework`
  才接受（同一 platform + variant 不允许两个 slice）。
* 链接期符号冲突：引擎静态链进主二进制，若宿主也链接同名符号（不太可能，
  `am_` 前缀足够独特）需 `-ObjC`/链接顺序调整。无 ObjC 运行时依赖。
* App Store 审核：静态库 + 无动态加载，无特殊风险；隐私清单（PrivacyInfo.xcprivacy）
  是宿主 App 的责任，引擎本身不采集任何数据。

### 2.4 iOS 上 Flutter

与 Android 对称：

1. CI 下载 `anima-engine-ios.xcframework.zip`，解出 `anima.xcframework` 放到 app 仓库的
   `ios/` 下，并往 `ios/Flutter/{Debug,Release}.xcconfig` 追加 `-force_load`（静态库必须显式
   链接：App Store 不允许内嵌 dylib，所以不能像桌面那样「放进 bundle 就完事」）；
2. Dart FFI 在 iOS 上 `DynamicLibrary.process()`（静态链接进主二进制时）或
   `DynamicLibrary.open("libanima.a")` 失败时回落 `DynamicLibrary.executable()`；
   注意 iOS 上静态符号要用 `DynamicLibrary.process()` 查找；
3. 画面同 §2.2：短期像素路径，中期 `Texture` + IOSurface 桥。

---

## 3. 三端共同注意点

| 事项 | 说明 |
| --- | --- |
| 契约一致性 | 三端都走同一份 `anima.h` / `ffi-contract.md`；`system.capabilities` 探测决定功能开关，**不**按平台硬编码 |
| 确定性 | 物理/动作为固定步长，三端同输入同输出（已有测试锁死）；不要在宿主侧额外推进时间 |
| 线程 | 一个 `am_engine` 句柄只许一个线程用；移动端建议专设一个渲染/求值线程，UI 线程只做调用转发 |
| 内存 | `am_string_free` / `am_engine_free` 必须成对；JNI/Swift 侧都建议封装 RAII 风格的句柄类 |
| GPU 关闭时的语义 | `renderer.*` 返回 `-32000`，宿主必须优雅降级（`system.capabilities.renderer == false` → 自绘路径） |
| 库体积 | release + `--no-default-features`（无 GPU）约 2–4 MB/ABI；开 GPU 后约 8–15 MB/ABI（wgpu） |

## 4. 落地顺序建议

1. **先桌面**（Windows 已验证）→ 移动端接入前保证契约稳定；
2. **Android .aar**：无 GPU 路线先跑通（数据 + scene JSON + 宿主自绘），
   风险最低、能尽早验证 FFI 在 ARM 上的正确性；
3. **iOS XCFramework**：直接带 Metal，真机渲染一步到位；
4. **Flutter 外部纹理桥**：Android（SurfaceTexture）与 iOS（IOSurface）分开做 PoC，
   接口统一到 app 仓库的 `lib/core/engine/` 抽象层（`AmEngine` 接口，见
   `docs/ffi-contract.md`）；
5. 最后做 **Android GPU 路线**（Vulkan/GLES 离屏 + AHardwareBuffer），
   需要真机矩阵驱动调优。

---

## 5. 交付与消费

本仓库的 `.github/workflows/build.yml` 在六个平台构建，勾选 `publish_release` 后发布到
Releases，**资产名固定、不含版本号**，于是宿主仓库可以直接用「永远指向最新发布」的直链，
配一次长期有效（`<owner>/<repo>` 换成实际值）：

| 平台 | 资产名 |
| --- | --- |
| Windows | `anima-engine-windows-x64.zip` |
| Linux | `anima-engine-linux-x64.tar.gz` |
| macOS | `anima-engine-macos-universal.tar.gz` |
| Android | `anima-engine-android.aar` |
| iOS | `anima-engine-ios.xcframework.zip` |
| Web | `anima-engine-web.zip` |

```text
https://github.com/midsancode/anima-engine/releases/latest/download/<资产名>
```

宿主仓库把上面这些地址配成 `ENGINE_{WINDOWS,LINUX,MACOS,ANDROID,IOS,WEB}_URL` 仓库变量，
构建时自动注入对应产物。Web 端消费的是 wasm-bindgen 的 ES 模块
（`anima_wasm.js` + `anima_wasm_bg.wasm`），调用面与 C ABI **同构**：同样是
「方法名 + JSON 参数 → JSON 信封」，所以宿主侧两套后端共用同一份上层代码。
