//! `am-ffi`：C ABI（编辑器与查看器通过它使用引擎）。
//!
//! # 契约（冻结）
//!
//! ```c
//! am_engine*  am_engine_new(void);
//! am_engine*  am_engine_new_from_file(const char* path);   // 载入 .amproj / 工程目录
//! void        am_engine_free(am_engine* engine);
//!
//! char*       am_call(am_engine* engine, const char* method, const char* params_json);
//! void        am_string_free(char* s);
//!
//! size_t      am_frame_copy(am_engine* engine, uint8_t* out, size_t capacity);
//! const char* am_version(void);
//! ```
//!
//! `am_call` 永远返回一个 UTF-8 JSON 字符串（必须用 `am_string_free` 释放）：
//!
//! ```json
//! { "ok": true,  "result": { ... } }
//! { "ok": false, "error": { "code": -32602, "message": "..." } }
//! ```
//!
//! # 线程与安全
//!
//! * 一个 `am_engine*` 只能被一个线程使用（内部状态非同步）。
//! * 传入空指针、空字符串或非法 UTF-8 都会返回结构化错误，**不会** panic。
//! * 所有跨 FFI 的 panic 都被 `catch_unwind` 拦截，转成 `-32603` 错误，
//!   避免未定义行为。
//!
//! 头文件：`bindings/include/anima.h`（与本文档一一对应，手工维护）。

#![allow(non_camel_case_types)]

use am_core::Session;
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

/// 不透明引擎句柄。
pub struct am_engine {
    session: Session,
    callback: Option<am_event_cb>,
    user_data: *mut c_void,
}

// 句柄由调用方保证单线程使用；裸指针不参与 Send/Sync 语义推导。
unsafe impl Send for am_engine {}

/// 事件回调：`event` 与 `payload` 都是 UTF-8 C 字符串，仅在回调期间有效。
pub type am_event_cb =
    extern "C" fn(event: *const c_char, payload: *const c_char, user_data: *mut c_void);

const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");

/// 引擎版本（静态字符串，无需释放）。
#[no_mangle]
pub extern "C" fn am_version() -> *const c_char {
    VERSION.as_ptr() as *const c_char
}

/// 创建一个空引擎。
#[no_mangle]
pub extern "C" fn am_engine_new() -> *mut am_engine {
    let engine = am_engine {
        session: Session::empty(),
        callback: None,
        user_data: ptr::null_mut(),
    };
    Box::into_raw(Box::new(engine))
}

/// 从 `.amproj` 压缩包或工程目录创建引擎；失败返回 `NULL`。
#[no_mangle]
pub extern "C" fn am_engine_new_from_file(path: *const c_char) -> *mut am_engine {
    let Some(path) = (unsafe { cstr(path) }) else {
        return ptr::null_mut();
    };
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<Session, String> {
        let project = am_format::Project::open(path).map_err(|e| e.to_string())?;
        let spec = am_core::read_spec(&project).map_err(|e| e.to_string())?;
        Ok(Session::with_spec(spec))
    }));
    match result {
        Ok(Ok(session)) => {
            let mut engine = Box::new(am_engine {
                session,
                callback: None,
                user_data: ptr::null_mut(),
            });
            engine.emit("ready", "{}");
            Box::into_raw(engine)
        }
        Ok(Err(message)) => {
            set_thread_error(&message);
            ptr::null_mut()
        }
        Err(_) => {
            set_thread_error("载入工程时发生内部错误");
            ptr::null_mut()
        }
    }
}

/// 释放引擎；`NULL` 安全。
#[no_mangle]
pub extern "C" fn am_engine_free(engine: *mut am_engine) {
    if engine.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(engine));
    }
}

/// 调用一个方法，返回 JSON 信封（调用方负责 [`am_string_free`]）。
#[no_mangle]
pub extern "C" fn am_call(
    engine: *mut am_engine,
    method: *const c_char,
    params_json: *const c_char,
) -> *mut c_char {
    let Some(method) = (unsafe { cstr(method) }) else {
        return into_c_string(error_envelope(-32600, "method 不是合法的 UTF-8 字符串"));
    };
    let params = if params_json.is_null() {
        ""
    } else {
        match unsafe { cstr(params_json) } {
            Some(text) => text,
            None => return into_c_string(error_envelope(-32600, "params 不是合法的 UTF-8 字符串")),
        }
    };
    if engine.is_null() {
        return into_c_string(error_envelope(-32600, "engine 为空指针"));
    }
    let engine_ref = unsafe { &mut *engine };

    let outcome = catch_unwind(AssertUnwindSafe(|| engine_ref.session.dispatch_json(method, params)));
    match outcome {
        Ok(raw) => {
            // 失败调用同时触发 error 事件，方便宿主统一上报
            if raw.starts_with(r#"{"ok":false"#) {
                engine_ref.emit("error", &raw);
            }
            into_c_string(raw)
        }
        Err(_) => into_c_string(error_envelope(-32603, "引擎内部错误")),
    }
}

/// 释放 `am_call` 返回的字符串；`NULL` 安全。
#[no_mangle]
pub extern "C" fn am_string_free(text: *mut c_char) {
    if text.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(text));
    }
}

/// 把最近一帧的像素复制到 `out`（最多 `capacity` 字节），返回实际字节数。
///
/// 返回 0 表示还没有渲染过任何一帧；返回值大于 `capacity` 表示缓冲区不够，
/// 此时不写入任何数据。
#[no_mangle]
pub extern "C" fn am_frame_copy(
    engine: *mut am_engine,
    out: *mut u8,
    capacity: usize,
) -> usize {
    if engine.is_null() {
        return 0;
    }
    let engine_ref = unsafe { &mut *engine };
    let pixels = engine_ref.frame_pixels();
    if pixels.is_empty() {
        return 0;
    }
    if out.is_null() || capacity < pixels.len() {
        return pixels.len();
    }
    unsafe {
        ptr::copy_nonoverlapping(pixels.as_ptr(), out, pixels.len());
    }
    pixels.len()
}

/// 注册事件回调（`callback` 传 `NULL` 表示取消）。
#[no_mangle]
pub extern "C" fn am_set_event_callback(
    engine: *mut am_engine,
    callback: Option<am_event_cb>,
    user_data: *mut c_void,
) {
    if engine.is_null() {
        return;
    }
    let engine_ref = unsafe { &mut *engine };
    engine_ref.callback = callback;
    engine_ref.user_data = user_data;
}

/// 最近一次错误消息（线程本地，无需释放）。
#[no_mangle]
pub extern "C" fn am_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| slot.get())
}

impl am_engine {
    #[cfg(feature = "gpu")]
    fn frame_pixels(&self) -> &[u8] {
        self.session.frame_pixels()
    }

    #[cfg(not(feature = "gpu"))]
    fn frame_pixels(&self) -> &[u8] {
        &[]
    }

    /// 触发宿主回调（回调内部再调用 `am_call` 属于未定义行为，文档中已声明）。
    fn emit(&mut self, event: &str, payload: &str) {
        let Some(callback) = self.callback else {
            return;
        };
        let Ok(event) = CString::new(event) else {
            return;
        };
        let Ok(payload) = CString::new(payload) else {
            return;
        };
        callback(event.as_ptr(), payload.as_ptr(), self.user_data);
    }
}

thread_local! {
    static LAST_ERROR: std::cell::Cell<*const c_char> = const { std::cell::Cell::new(ptr::null()) };
    // 每个线程保留一份字符串，保证指针在下次设置前一直有效
    static LAST_ERROR_OWNED: std::cell::RefCell<CString> =
        std::cell::RefCell::new(CString::new("").unwrap());
}

fn set_thread_error(message: &str) {
    let text = CString::new(message).unwrap_or_else(|_| CString::new("内部错误").unwrap());
    LAST_ERROR_OWNED.with(|cell| {
        let mut owned = cell.borrow_mut();
        *owned = text;
        LAST_ERROR.with(|slot| slot.set(owned.as_ptr() as *const c_char));
    });
}

fn error_envelope(code: i32, message: &str) -> String {
    format!(
        r#"{{"ok":false,"error":{{"code":{code},"message":{}}}}}"#,
        serde_json::Value::String(message.to_string())
    )
}

fn into_c_string(text: String) -> *mut c_char {
    match CString::new(text) {
        Ok(value) => value.into_raw(),
        Err(_) => CString::new(error_envelope(-32603, "结果包含非法字符"))
            .map(|v| v.into_raw())
            .unwrap_or(ptr::null_mut()),
    }
}

/// 读取 C 字符串；空指针或非法 UTF-8 返回 `None`。
unsafe fn cstr<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(engine: *mut am_engine, method: &str, params: &str) -> String {
        let method = CString::new(method).unwrap();
        let params = CString::new(params).unwrap();
        let raw = am_call(engine, method.as_ptr(), params.as_ptr());
        assert!(!raw.is_null());
        let text = unsafe { CStr::from_ptr(raw).to_str().unwrap().to_string() };
        am_string_free(raw);
        text
    }

    #[test]
    fn null_engine_is_reported() {
        let method = CString::new("system.ping").unwrap();
        let raw = am_call(ptr::null_mut(), method.as_ptr(), ptr::null());
        assert!(!raw.is_null());
        let text = unsafe { CStr::from_ptr(raw).to_str().unwrap().to_string() };
        am_string_free(raw);
        assert!(text.contains("engine 为空指针"), "got {text}");

        // method 为空指针时也要给出结构化错误
        let raw = am_call(ptr::null_mut(), ptr::null(), ptr::null());
        let text = unsafe { CStr::from_ptr(raw).to_str().unwrap().to_string() };
        am_string_free(raw);
        assert!(text.contains("method 不是合法的 UTF-8 字符串"), "got {text}");
    }

    #[test]
    fn round_trip_ping() {
        let engine = am_engine_new();
        let text = call(engine, "system.ping", "");
        assert!(text.contains("\"pong\":true"), "got {text}");
        am_engine_free(engine);
    }

    #[test]
    fn version_is_static() {
        let ptr = am_version();
        assert!(!ptr.is_null());
        let text = unsafe { CStr::from_ptr(ptr).to_str().unwrap() };
        assert!(!text.is_empty());
    }

    #[test]
    fn invalid_utf8_params_are_rejected() {
        let engine = am_engine_new();
        let method = CString::new("system.ping").unwrap();
        let bad = [0xffu8, 0x00];
        let raw = am_call(engine, method.as_ptr(), bad.as_ptr() as *const c_char);
        let text = unsafe { CStr::from_ptr(raw).to_str().unwrap().to_string() };
        am_string_free(raw);
        assert!(text.contains("-32600"), "got {text}");
        am_engine_free(engine);
    }

    #[test]
    fn frame_copy_before_render_returns_zero() {
        let engine = am_engine_new();
        let mut buffer = [0u8; 16];
        let written = am_frame_copy(engine, buffer.as_mut_ptr(), buffer.len());
        assert_eq!(written, 0);
        am_engine_free(engine);
    }

    #[test]
    fn loading_a_missing_project_returns_null() {
        let path = CString::new("Z:\\definitely\\missing\\path.amproj").unwrap();
        let engine = am_engine_new_from_file(path.as_ptr());
        assert!(engine.is_null());
        let message = am_last_error();
        assert!(!message.is_null());
    }

    #[test]
    fn freeing_null_is_safe() {
        am_engine_free(ptr::null_mut());
        am_string_free(ptr::null_mut());
    }
}
