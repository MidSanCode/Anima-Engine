//! 端到端测试：**只**通过 C ABI 驱动引擎，验证宿主（编辑器 / 查看器）看到的契约。

use anima::*;
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr;
use std::sync::{Mutex, OnceLock};

fn call(engine: *mut am_engine, method: &str, params: Value) -> Value {
    let method = CString::new(method).unwrap();
    let params = CString::new(params.to_string()).unwrap();
    let raw = am_call(engine, method.as_ptr(), params.as_ptr());
    assert!(!raw.is_null(), "am_call 不应返回 NULL");
    let text = unsafe { CStr::from_ptr(raw).to_str().unwrap().to_string() };
    am_string_free(raw);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("信封不是合法 JSON：{text} ({e})"))
}

fn ok(engine: *mut am_engine, method: &str, params: Value) -> Value {
    let envelope = call(engine, method, params);
    assert_eq!(envelope["ok"], Value::Bool(true), "{method} 失败：{envelope}");
    envelope["result"].clone()
}

fn error_code(engine: *mut am_engine, method: &str, params: Value) -> i64 {
    let envelope = call(engine, method, params);
    assert_eq!(envelope["ok"], Value::Bool(false), "{method} 不应成功");
    envelope["error"]["code"].as_i64().unwrap()
}

#[test]
fn version_is_available_without_an_engine() {
    let ptr = am_version();
    assert!(!ptr.is_null());
    let text = unsafe { CStr::from_ptr(ptr).to_str().unwrap() };
    assert!(!text.is_empty(), "版本号不能为空");
}

#[test]
fn engine_lifecycle_and_ping() {
    let engine = am_engine_new();
    assert!(!engine.is_null());
    let pong = ok(engine, "system.ping", json!({}));
    assert_eq!(pong["pong"], Value::Bool(true));
    assert_eq!(error_code(engine, "nope", json!({})), -32601);
    am_engine_free(engine);
}

#[test]
fn project_round_trip_through_the_c_abi() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo");

    let engine = am_engine_new();
    ok(engine, "project.new", json!({ "name": "demo", "width": 256, "height": 256 }));
    ok(
        engine,
        "doc.command",
        json!({ "command": {
            "op": "node_create",
            "kind": "part",
            "name": "Root",
        }}),
    );
    ok(
        engine,
        "doc.command",
        json!({ "command": {
            "op": "node_create",
            "kind": "drawable",
            "name": "Body",
            "rect": { "min": [-64.0, -64.0], "max": [64.0, 64.0] },
        }}),
    );
    ok(
        engine,
        "doc.command",
        json!({ "command": { "op": "texture_add", "asset": "assets/images/0.png" } }),
    );

    let model = ok(engine, "doc.model", json!({}));
    let nodes = model["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2);
    let drawable = nodes
        .iter()
        .find(|n| n["name"] == Value::String("Body".into()))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        engine,
        "doc.command",
        json!({ "command": { "op": "drawable_set_texture", "node": drawable, "texture": 0 } }),
    );

    let saved = ok(engine, "project.save", json!({ "path": root.to_string_lossy() }));
    assert_eq!(saved["path"].as_str().unwrap(), root.to_string_lossy());
    assert!(root.join("spec").join("model.json").is_file());
    am_engine_free(engine);

    // 重新打开
    let path = CString::new(root.to_string_lossy().to_string()).unwrap();
    let reopened = am_engine_new_from_file(path.as_ptr());
    assert!(!reopened.is_null(), "重新载入失败");
    let model = ok(reopened, "doc.model", json!({}));
    assert_eq!(model["name"], Value::String("demo".into()));
    assert_eq!(model["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(model["textures"].as_array().unwrap().len(), 1);

    // 撤销历史是新的（载入即干净状态）
    let history = ok(reopened, "doc.history", json!({}));
    assert_eq!(history["can_undo"], Value::Bool(false));
    am_engine_free(reopened);
}

#[test]
fn simulation_runs_through_the_c_abi() {
    let engine = am_engine_new();
    ok(
        engine,
        "doc.command",
        json!({ "command": { "op": "parameter_add", "id": "AngleX", "name": "Angle X",
                             "min": -30.0, "max": 30.0, "default": 0.0 } }),
    );
    ok(engine, "runtime.set_param", json!({ "id": "AngleX", "value": 12.0 }));
    let params = ok(engine, "runtime.params", json!({}));
    assert_eq!(params["AngleX"].as_f64().unwrap(), 12.0);

    for _ in 0..10 {
        ok(engine, "runtime.advance", json!({ "dt": 1.0 / 60.0 }));
    }
    let state = ok(engine, "runtime.state", json!({}));
    assert_eq!(state["frame"].as_u64().unwrap(), 10);

    ok(engine, "runtime.pause", json!({}));
    ok(engine, "runtime.advance", json!({ "dt": 1.0 }));
    assert_eq!(ok(engine, "runtime.state", json!({}))["frame"].as_u64().unwrap(), 10);
    am_engine_free(engine);
}

#[test]
fn renderer_renders_and_frame_copy_works() {
    let engine = am_engine_new();
    let info = ok(engine, "renderer.info", json!({}));
    assert_eq!(info["initialized"], Value::Bool(false));

    let init = call(engine, "renderer.init", json!({ "width": 64, "height": 64 }));
    if init["ok"] == Value::Bool(false) {
        // 没有可用图形适配器的环境：跳过渲染断言
        eprintln!("[skip] 无法初始化渲染器：{}", init["error"]["message"]);
        am_engine_free(engine);
        return;
    }

    ok(engine, "doc.command", json!({ "command": {
        "op": "node_create", "kind": "drawable", "name": "Quad",
        "rect": { "min": [-16.0, -16.0], "max": [16.0, 16.0] },
    }}));
    let rendered = ok(engine, "renderer.render", json!({}));
    assert_eq!(rendered["width"].as_u64().unwrap(), 64);

    // 只查询长度
    let needed = am_frame_copy(engine, ptr::null_mut(), 0);
    assert_eq!(needed, 64 * 64 * 4);

    // 缓冲区不足：不写入，返回所需长度
    let mut small = [0u8; 8];
    let needed = am_frame_copy(engine, small.as_mut_ptr(), small.len());
    assert_eq!(needed, 64 * 64 * 4);
    assert!(small.iter().all(|b| *b == 0), "缓冲区不足时不应写入");

    // 正常复制
    let mut buffer = vec![0u8; needed];
    let written = am_frame_copy(engine, buffer.as_mut_ptr(), buffer.len());
    assert_eq!(written, needed);
    // 没有上传纹理，画面应为全透明
    assert!(buffer.iter().all(|b| *b == 0), "无纹理时画面应全透明");

    am_engine_free(engine);
}

// -------------------------------------------------------------- 事件回调

fn events() -> &'static Mutex<Vec<(String, String)>> {
    static EVENTS: OnceLock<Mutex<Vec<(String, String)>>> = OnceLock::new();
    EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

extern "C" fn record_event(event: *const c_char, payload: *const c_char, user_data: *mut c_void) {
    assert!(!user_data.is_null(), "user_data 应原样传回");
    let event = unsafe { CStr::from_ptr(event).to_str().unwrap().to_string() };
    let payload = unsafe { CStr::from_ptr(payload).to_str().unwrap().to_string() };
    events().lock().unwrap().push((event, payload));
}

#[test]
fn event_callback_receives_errors() {
    events().lock().unwrap().clear();
    let engine = am_engine_new();
    let mut marker = 0u8;
    am_set_event_callback(engine, Some(record_event), &mut marker as *mut u8 as *mut c_void);

    let _ = call(engine, "runtime.set_param", json!({ "id": "ghost", "value": 1.0 }));
    let recorded = events().lock().unwrap().clone();
    assert_eq!(recorded.len(), 1, "应恰好收到一个 error 事件：{recorded:?}");
    assert_eq!(recorded[0].0, "error");
    assert!(recorded[0].1.contains("-32602"));

    // 取消注册后不再收到
    am_set_event_callback(engine, None, ptr::null_mut());
    let _ = call(engine, "runtime.set_param", json!({ "id": "ghost", "value": 1.0 }));
    assert_eq!(events().lock().unwrap().len(), 1);

    am_engine_free(engine);
}

#[test]
fn null_handles_are_safe() {
    assert!(am_engine_new_from_file(ptr::null()).is_null());
    am_engine_free(ptr::null_mut());
    am_string_free(ptr::null_mut());
    am_set_event_callback(ptr::null_mut(), None, ptr::null_mut());
    assert_eq!(am_frame_copy(ptr::null_mut(), ptr::null_mut(), 0), 0);
    let envelope = call(ptr::null_mut(), "system.ping", json!({}));
    assert_eq!(envelope["ok"], Value::Bool(false));
}
