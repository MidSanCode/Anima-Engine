/*
 * anima.h —— Anima 引擎 C 接口（版本 0.1.0）
 *
 * 这个头文件是引擎与宿主（编辑器 / 查看器 / 命令行）之间的**冻结契约**。
 * 改动这里必须同步修改 crates/am-ffi/src/lib.rs 与 docs/ffi-contract.md。
 *
 * 调用约定
 * --------
 *   * 一个 am_engine* 只能被一个线程使用；不同引擎可以在不同线程并行。
 *   * 所有返回 char* 的函数都返回 UTF-8 JSON 信封，调用方必须用
 *     am_string_free 释放：
 *
 *       { "ok": true,  "result": { ... } }
 *       { "ok": false, "error": { "code": -32602, "message": "..." } }
 *
 *   * 传空指针 / 空字符串 / 非法 UTF-8 都会得到结构化错误，不会崩溃。
 *
 * 方法命名空间
 * ------------
 *   system.*      版本与能力探测（编辑器必须先调用 system.capabilities）
 *   project.*     工程读写（.amproj / 目录）
 *   doc.*         结构编辑命令与撤销
 *   runtime.*     参数、时间、暂停
 *   motion.*      动作播放
 *   expression.*  表情
 *   physics.*     物理
 *   renderer.*    离屏渲染（需要 GPU 后端）
 *   diagnostics.* 统计信息
 *
 * 完整方法清单见 system.capabilities 的 methods 字段。
 */

#ifndef ANIMA_H
#define ANIMA_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#if defined(_WIN32)
#  define ANIMA_API __declspec(dllimport)
#else
#  define ANIMA_API
#endif

/** 不透明引擎句柄。 */
typedef struct am_engine am_engine;

/**
 * 事件回调。
 *
 * @param event      事件名（"ready" / "error" / "frame" ...）
 * @param payload    UTF-8 JSON，仅在回调期间有效，不要保存指针
 * @param user_data  注册时传入的指针
 *
 * 注意：不要在回调里再次调用 am_call（同一引擎不可重入）。
 */
typedef void (*am_event_cb)(const char* event, const char* payload, void* user_data);

/** 引擎版本号（静态字符串，无需释放）。 */
ANIMA_API const char* am_version(void);

/** 创建空引擎；失败返回 NULL。 */
ANIMA_API am_engine* am_engine_new(void);

/**
 * 从 `.amproj` 压缩包或工程目录载入引擎。
 * 失败返回 NULL，可用 am_last_error() 取原因。
 */
ANIMA_API am_engine* am_engine_new_from_file(const char* path);

/** 释放引擎；NULL 安全。释放后句柄不可再用。 */
ANIMA_API void am_engine_free(am_engine* engine);

/**
 * 调用一个方法。
 *
 * @param method      方法名，例如 "runtime.advance"
 * @param params_json UTF-8 JSON 对象，可为 NULL 或空串（等价于 {}）
 * @return UTF-8 JSON 信封；必须用 am_string_free 释放。永不返回 NULL（除非内存耗尽）。
 */
ANIMA_API char* am_call(am_engine* engine, const char* method, const char* params_json);

/** 释放 am_call 返回的字符串；NULL 安全。 */
ANIMA_API void am_string_free(char* text);

/**
 * 复制最近一帧的像素（RGBA8，第 0 行是画面顶部）。
 *
 * @param out       目标缓冲区；为 NULL 时只查询所需字节数
 * @param capacity  缓冲区容量（字节）
 * @return 实际写入的字节数；返回 0 表示还没有渲染过任何一帧；
 *         返回值大于 capacity 表示缓冲区不足，此时不会写入任何数据。
 */
ANIMA_API size_t am_frame_copy(am_engine* engine, uint8_t* out, size_t capacity);

/** 注册事件回调；callback 传 NULL 取消注册。 */
ANIMA_API void am_set_event_callback(am_engine* engine, am_event_cb callback, void* user_data);

/** 最近一次错误消息（线程本地，无需释放；可能返回 NULL）。 */
ANIMA_API const char* am_last_error(void);

#ifdef __cplusplus
}
#endif

#endif /* ANIMA_H */
