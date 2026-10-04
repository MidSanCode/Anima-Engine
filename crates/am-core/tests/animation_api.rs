//! `animation.*` 方法面的集成测试（规范 §3 / §9）。
//!
//! 这些测试全部走 [`Session::dispatch`] 的 JSON 协议 —— 也就是宿主真正用的那条路。
//! 它们验证的是**契约**，不是实现细节：
//!
//! * 模式互斥（`-32010`）、未烘焙（`-32011`）、不存在（`-32012`）
//! * 烘焙 → `seek` → 读参数 的完整闭环
//! * 重复烘焙逐位一致（跨 JSON 边界）
//! * `animation.sample` 不改变会话状态

use am_core::{Session, ENGINE_VERSION};
use serde_json::{json, Value};

/// 造一个「AngleX 从 0 到 30、1 秒」的会话。
fn session_with_animation() -> Session {
    let mut model = am_model::Model::new("demo");
    model
        .parameters
        .push(am_model::Parameter::new("AngleX", "Angle X", -180.0, 180.0, 0.0));
    let mut session = Session::new(model);

    call(&mut session, "animation.new", json!({ "id": "idle", "name": "待机" }));
    call(
        &mut session,
        "animation.channel.add",
        json!({ "id": "idle", "target": "AngleX" }),
    );
    call(
        &mut session,
        "animation.key.set",
        json!({ "id": "idle", "target": "AngleX", "time": 0.0, "value": 0.0 }),
    );
    call(
        &mut session,
        "animation.key.set",
        json!({ "id": "idle", "target": "AngleX", "time": 1.0, "value": 30.0 }),
    );
    call(
        &mut session,
        "animation.set_meta",
        json!({ "id": "idle", "duration": 1.0, "fps": 30.0 }),
    );
    session
}

/// 调用一个方法，成功则返回 `result`；失败直接 panic（测试里就是要它成功）。
fn call(session: &mut Session, method: &str, params: Value) -> Value {
    match try_call(session, method, params) {
        Ok(value) => value,
        Err(err) => panic!("{method} 应当成功，实际失败：{err}"),
    }
}

/// 调用一个方法，返回 `Result`（用于验证错误路径）。
fn try_call(session: &mut Session, method: &str, params: Value) -> Result<Value, Value> {
    let json = session.dispatch_json(method, &params.to_string());
    let value: Value = serde_json::from_str(&json).expect("信封必须是合法 JSON");
    if value["ok"] == json!(true) {
        Ok(value["result"].clone())
    } else {
        Err(value["error"].clone())
    }
}

/// 取错误码。
fn error_code(error: &Value) -> i64 {
    error["code"].as_i64().unwrap_or(0)
}

#[test]
fn every_advertised_animation_method_is_callable() {
    // 能力清单里声明的每个 animation.* 都必须真的能被分发 ——
    // 否则宿主探测到能力、调用却拿到 `-32601`，是「说不支持又不说不支持」的坏状态。
    let mut session = Session::empty();
    let capabilities = call(&mut session, "system.capabilities", json!({}));
    let advertised: Vec<String> = capabilities["methods"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .filter(|m| m.starts_with("animation."))
        .map(|m| m.to_string())
        .collect();

    assert!(!advertised.is_empty(), "能力清单应包含 animation.*");

    for method in &advertised {
        // 有的方法不需要参数（如 animation.mode）会直接成功，这是正常的；
        // 这里只关心它**没有被当成未知方法**。
        if let Err(error) = try_call(&mut session, method, json!({})) {
            assert_ne!(
                error_code(&error),
                -32601,
                "{method} 声明在能力清单里却无法分发"
            );
        }
    }
}

#[test]
fn undeclared_animation_method_reports_method_not_found() {
    let mut session = Session::empty();
    let error = try_call(&mut session, "animation.no_such_method", json!({})).unwrap_err();
    assert_eq!(error_code(&error), -32601);
}

// ------------------------------------------------------------ 能力探测

#[test]
fn animation_methods_are_advertised() {
    let mut session = Session::empty();
    let capabilities = call(&mut session, "system.capabilities", json!({}));
    let methods: Vec<String> = capabilities["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    for expected in [
        "animation.mode",
        "animation.open",
        "animation.bake",
        "animation.seek",
        "animation.sample",
        "animation.baked",
        "animation.list",
    ] {
        assert!(
            methods.contains(&expected.to_string()),
            "能力清单缺少 {expected}"
        );
    }
}

#[test]
fn engine_version_is_exposed() {
    let mut session = Session::empty();
    let version = call(&mut session, "system.version", json!({}));
    assert_eq!(version["version"], json!(ENGINE_VERSION));
}

// ------------------------------------------------------------ 生命周期

#[test]
fn new_list_query_delete_round_trip() {
    let mut session = session_with_animation();

    let list = call(&mut session, "animation.list", json!({}));
    let items = list.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], json!("idle"));
    assert_eq!(items[0]["name"], json!("待机"));
    assert_eq!(items[0]["baked"], json!(false));

    let query = call(&mut session, "animation.query", json!({ "id": "idle" }));
    assert_eq!(query["channels"], json!(1));

    let detail = call(
        &mut session,
        "animation.query",
        json!({ "id": "idle", "detail": true }),
    );
    assert_eq!(detail["channels"].as_array().unwrap().len(), 1);

    call(&mut session, "animation.delete", json!({ "id": "idle" }));
    let after = call(&mut session, "animation.list", json!({}));
    assert_eq!(after.as_array().unwrap().len(), 0);
}

#[test]
fn duplicate_id_is_rejected() {
    let mut session = session_with_animation();
    let error = try_call(
        &mut session,
        "animation.new",
        json!({ "id": "idle", "name": "重复" }),
    )
    .unwrap_err();
    assert_eq!(error_code(&error), -32602);
}

#[test]
fn bad_id_is_rejected_but_auto_id_works() {
    let mut session = Session::empty();
    let error = try_call(
        &mut session,
        "animation.new",
        json!({ "id": "Bad Id", "name": "x" }),
    )
    .unwrap_err();
    assert_eq!(error_code(&error), -32602);

    // 不传 id → 从名字推导
    let created = call(&mut session, "animation.new", json!({ "name": "Walk Loop" }));
    assert_eq!(created["id"], json!("walk-loop"));
}

#[test]
fn unknown_animation_returns_not_found_code() {
    let mut session = Session::empty();
    let error = try_call(&mut session, "animation.query", json!({ "id": "ghost" })).unwrap_err();
    assert_eq!(error_code(&error), -32012, "应为 ANIMATION_NOT_FOUND");
    assert_eq!(error["hint"], json!("animation.list"));
}

// ------------------------------------------------------------ 模式互斥

#[test]
fn opening_unbaked_animation_is_rejected() {
    let mut session = session_with_animation();
    let error = try_call(&mut session, "animation.open", json!({ "id": "idle" })).unwrap_err();
    assert_eq!(error_code(&error), -32011, "应为 NOT_BAKED");
    assert_eq!(error["hint"], json!("animation.bake"));
}

#[test]
fn mode_is_live_by_default() {
    let mut session = Session::empty();
    let mode = call(&mut session, "animation.mode", json!({}));
    assert_eq!(mode["mode"], json!("live"));
    assert_eq!(mode["open"], Value::Null);
}

#[test]
fn runtime_advance_is_blocked_in_prerender_mode() {
    let mut session = session_with_animation();
    // 先烘焙再打开
    call(&mut session, "animation.bake", json!({ "id": "idle", "baked_at": 1 }));
    call(&mut session, "animation.open", json!({ "id": "idle" }));

    // 实时推进必须被拒绝（§2.1）
    let error = try_call(&mut session, "runtime.advance", json!({ "dt": 0.016 })).unwrap_err();
    assert_eq!(error_code(&error), -32010, "应为 MODE_CONFLICT");
    assert_eq!(error["hint"], json!("animation.close"));

    // 关掉之后恢复可用
    call(&mut session, "animation.close", json!({}));
    let advanced = call(&mut session, "runtime.advance", json!({ "dt": 0.016 }));
    assert!(advanced["frame"].as_u64().unwrap() >= 1);
}

#[test]
fn runtime_scene_still_works_in_prerender_mode() {
    // 规范 §3：预渲染模式下查询类方法仍然可用
    let mut session = session_with_animation();
    call(&mut session, "animation.bake", json!({ "id": "idle", "baked_at": 1 }));
    call(&mut session, "animation.open", json!({ "id": "idle", "time": 0.5 }));

    let params = call(&mut session, "runtime.params", json!({}));
    // 0.5 秒处 AngleX 应为 15（线性 0→30）
    let value = params["AngleX"].as_f64().unwrap();
    assert!((value - 15.0).abs() < 1e-3, "期望约 15，实际 {value}");

    let scene = call(&mut session, "runtime.scene", json!({}));
    assert!(scene.is_object());
}

// ------------------------------------------------------------ 烘焙与 seek

#[test]
fn bake_then_seek_reads_baked_values() {
    let mut session = session_with_animation();
    let baked = call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "baked_at": 1_789_000_000 }),
    );
    assert_eq!(baked["frames"], json!(31), "1 秒 @30fps → 31 帧");
    assert_eq!(baked["channels"], json!(1));
    assert_eq!(baked["geometry"], json!("model_ref"));
    assert_eq!(baked["baked_at"], json!(1_789_000_000u64));

    call(&mut session, "animation.open", json!({ "id": "idle" }));

    // 首帧
    let start = call(&mut session, "animation.seek", json!({ "time": 0.0 }));
    assert_eq!(start["frame"], json!(0));
    let params = call(&mut session, "runtime.params", json!({}));
    assert!(params["AngleX"].as_f64().unwrap().abs() < 1e-3);

    // 末帧
    call(&mut session, "animation.seek", json!({ "time": 1.0 }));
    let params = call(&mut session, "runtime.params", json!({}));
    let value = params["AngleX"].as_f64().unwrap();
    assert!((value - 30.0).abs() < 1e-2, "期望约 30，实际 {value}");
}

#[test]
fn seek_is_idempotent_and_not_accumulative() {
    // 反复 seek 到同一时间必须给同样的结果（apply_prerender_frame 会先 reset）
    let mut session = session_with_animation();
    call(&mut session, "animation.bake", json!({ "id": "idle", "baked_at": 1 }));
    call(&mut session, "animation.open", json!({ "id": "idle" }));

    call(&mut session, "animation.seek", json!({ "time": 0.5 }));
    let first = call(&mut session, "runtime.params", json!({}))["AngleX"]
        .as_f64()
        .unwrap();
    for _ in 0..5 {
        call(&mut session, "animation.seek", json!({ "time": 0.5 }));
    }
    let last = call(&mut session, "runtime.params", json!({}))["AngleX"]
        .as_f64()
        .unwrap();
    assert!(
        (first - last).abs() < 1e-6,
        "重复 seek 不应累积：首次 {first}，末次 {last}"
    );
}

#[test]
fn repeat_bake_is_bitwise_identical_across_json() {
    // §4.4 硬要求 1 —— 跨 JSON 边界再验一次
    let mut session = session_with_animation();
    let first = call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "baked_at": 1 }),
    );
    let second = call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "baked_at": 1 }),
    );
    assert_eq!(first["frames"], second["frames"]);
    assert_eq!(first["points"], second["points"]);
    assert_eq!(first["bytes"], second["bytes"]);

    // 逐帧比对参数值
    call(&mut session, "animation.open", json!({ "id": "idle" }));
    let mut samples = Vec::new();
    for step in 0..=10 {
        let time = step as f64 * 0.1;
        let sample = call(&mut session, "animation.sample", json!({ "time": time }));
        samples.push(sample["params"]["AngleX"].as_f64().unwrap());
    }
    call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "baked_at": 1 }),
    );
    for (step, expected) in samples.iter().enumerate() {
        let time = step as f64 * 0.1;
        let sample = call(&mut session, "animation.sample", json!({ "time": time }));
        let actual = sample["params"]["AngleX"].as_f64().unwrap();
        assert_eq!(*expected, actual, "t={time} 处重复烘焙结果不一致");
    }
}

#[test]
fn baked_status_reflects_bake() {
    let mut session = session_with_animation();
    let before = call(&mut session, "animation.baked", json!({ "id": "idle" }));
    assert_eq!(before["baked"], json!(false));
    assert_eq!(before["frames"], json!(0));

    call(&mut session, "animation.bake", json!({ "id": "idle", "baked_at": 42 }));
    let after = call(&mut session, "animation.baked", json!({ "id": "idle" }));
    assert_eq!(after["baked"], json!(true));
    assert_eq!(after["frames"], json!(31));
    assert_eq!(after["baked_at"], json!(42));
}

// ------------------------------------------------------------ sample

#[test]
fn sample_does_not_mutate_session_state() {
    let mut session = session_with_animation();
    call(&mut session, "animation.bake", json!({ "id": "idle", "baked_at": 1 }));

    let before_mode = call(&mut session, "animation.mode", json!({}));
    assert_eq!(before_mode["mode"], json!("live"), "sample 不应切模式");

    // live 模式下也能直接采样（不打开动画）
    let sample = call(
        &mut session,
        "animation.sample",
        json!({ "id": "idle", "time": 0.5 }),
    );
    let value = sample["params"]["AngleX"].as_f64().unwrap();
    assert!((value - 15.0).abs() < 1e-3, "期望约 15，实际 {value}");

    let after_mode = call(&mut session, "animation.mode", json!({}));
    assert_eq!(after_mode["mode"], json!("live"), "sample 后仍应是 live");
}

#[test]
fn sample_before_bake_is_rejected() {
    let mut session = session_with_animation();
    let error = try_call(
        &mut session,
        "animation.sample",
        json!({ "id": "idle", "time": 0.5 }),
    )
    .unwrap_err();
    assert_eq!(error_code(&error), -32011);
}

#[test]
fn sample_clamps_beyond_range() {
    let mut session = session_with_animation();
    call(&mut session, "animation.bake", json!({ "id": "idle", "baked_at": 1 }));
    // 超出区间应钳到末帧（不外推）
    let sample = call(
        &mut session,
        "animation.sample",
        json!({ "id": "idle", "time": 99.0 }),
    );
    let value = sample["params"]["AngleX"].as_f64().unwrap();
    assert!((value - 30.0).abs() < 1e-2, "应钳到末帧 30，实际 {value}");
}

// ------------------------------------------------------------ 通道与关键帧

#[test]
fn channel_add_rejects_unknown_target() {
    let mut session = Session::empty();
    call(&mut session, "animation.new", json!({ "id": "idle", "name": "I" }));
    let error = try_call(
        &mut session,
        "animation.channel.add",
        json!({ "id": "idle", "target": "NoSuchParam" }),
    )
    .unwrap_err();
    assert_eq!(error_code(&error), -32602, "手滑建通道应被拦下");
}

#[test]
fn structural_channel_requires_existing_node() {
    let mut session = Session::empty();
    call(&mut session, "animation.new", json!({ "id": "idle", "name": "I" }));
    let error = try_call(
        &mut session,
        "animation.channel.add",
        json!({ "id": "idle", "kind": "visibility", "target": "no-node" }),
    )
    .unwrap_err();
    assert_eq!(error_code(&error), -32602);
}

#[test]
fn key_lifecycle_set_move_remove() {
    let mut session = session_with_animation();

    // 移动关键帧 1.0 → 0.8
    call(
        &mut session,
        "animation.key.move",
        json!({ "id": "idle", "target": "AngleX", "from": 1.0, "to": 0.8 }),
    );
    let detail = call(
        &mut session,
        "animation.query",
        json!({ "id": "idle", "detail": true }),
    );
    let keys = detail["channels"][0]["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 2);
    assert!((keys[1]["time"].as_f64().unwrap() - 0.8).abs() < 1e-6);

    // 删除
    call(
        &mut session,
        "animation.key.remove",
        json!({ "id": "idle", "target": "AngleX", "time": 0.8 }),
    );
    let detail = call(
        &mut session,
        "animation.query",
        json!({ "id": "idle", "detail": true }),
    );
    assert_eq!(detail["channels"][0]["keys"].as_array().unwrap().len(), 1);

    // 删不存在的关键帧 → 报错
    let error = try_call(
        &mut session,
        "animation.key.remove",
        json!({ "id": "idle", "target": "AngleX", "time": 5.0 }),
    )
    .unwrap_err();
    assert_eq!(error_code(&error), -32602);
}

#[test]
fn set_easing_applies_to_selected_or_all_keys() {
    let mut session = session_with_animation();

    // 只改 0.0 处
    call(
        &mut session,
        "animation.channel.set_easing",
        json!({ "id": "idle", "target": "AngleX", "time": 0.0, "easing": { "type": "step" } }),
    );
    let detail = call(
        &mut session,
        "animation.query",
        json!({ "id": "idle", "detail": true }),
    );
    let keys = detail["channels"][0]["keys"].as_array().unwrap();
    assert_eq!(keys[0]["easing"]["type"], json!("step"));
    assert_eq!(keys[1]["easing"]["type"], json!("linear"), "另一帧不该被改");

    // 不带 time → 全部
    let updated = call(
        &mut session,
        "animation.channel.set_easing",
        json!({ "id": "idle", "target": "AngleX", "easing": { "type": "ease_in_out" } }),
    );
    assert_eq!(updated["updated"], json!(2));
}

#[test]
fn curve_set_replaces_and_sorts_keys() {
    let mut session = session_with_animation();
    let result = call(
        &mut session,
        "animation.curve.set",
        json!({
            "id": "idle",
            "target": "AngleX",
            "keys": [
                { "time": 1.0, "value": 30.0 },
                { "time": 0.0, "value": 0.0 },
                { "time": 0.5, "value": 20.0 }
            ]
        }),
    );
    assert_eq!(result["keys"], json!(3));

    let detail = call(
        &mut session,
        "animation.query",
        json!({ "id": "idle", "detail": true }),
    );
    let keys = detail["channels"][0]["keys"].as_array().unwrap();
    let times: Vec<f64> = keys.iter().map(|k| k["time"].as_f64().unwrap()).collect();
    assert_eq!(times, vec![0.0, 0.5, 1.0], "关键帧必须按时间升序");
}

#[test]
fn channel_remove_reports_missing_channel() {
    let mut session = session_with_animation();
    let error = try_call(
        &mut session,
        "animation.channel.remove",
        json!({ "id": "idle", "target": "AngleX" }),
    );
    assert!(error.is_ok(), "存在的通道应删除成功");

    let error = try_call(
        &mut session,
        "animation.channel.remove",
        json!({ "id": "idle", "target": "AngleX" }),
    )
    .unwrap_err();
    assert_eq!(error_code(&error), -32602);
}

// ------------------------------------------------------------ overlay

#[test]
fn overlay_can_be_set_by_reference() {
    let mut session = session_with_animation();
    let result = call(
        &mut session,
        "animation.set_param_ref",
        json!({ "id": "idle", "motion": "breath", "weight": 0.5 }),
    );
    assert_eq!(result["overlay"]["motions"][0]["id"], json!("breath"));
    assert_eq!(result["overlay"]["motions"][0]["weight"], json!(0.5));

    // 移除
    let result = call(
        &mut session,
        "animation.set_param_ref",
        json!({ "id": "idle", "motion": "breath", "remove": true }),
    );
    assert_eq!(result["overlay"]["motions"].as_array().unwrap().len(), 0);
}

#[test]
fn overlay_partial_update() {
    let mut session = session_with_animation();
    let result = call(
        &mut session,
        "animation.overlay.set",
        json!({ "id": "idle", "physics": true, "auto_effects": true }),
    );
    assert_eq!(result["overlay"]["physics"], json!(true));
    assert_eq!(result["overlay"]["auto_effects"], json!(true));
}

// ------------------------------------------------------------ 参数校验

#[test]
fn bake_rejects_bad_fps_and_range() {
    let mut session = session_with_animation();
    let error = try_call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "fps": 0.0, "baked_at": 1 }),
    )
    .unwrap_err();
    assert_eq!(error_code(&error), -32602);

    let error = try_call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "range": { "start": 2.0, "end": 1.0 }, "baked_at": 1 }),
    )
    .unwrap_err();
    assert_eq!(error_code(&error), -32602);
}

#[test]
fn bake_rejects_bad_geometry_mode() {
    let mut session = session_with_animation();
    let error = try_call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "geometry": "hologram", "baked_at": 1 }),
    )
    .unwrap_err();
    assert_eq!(error_code(&error), -32602);
}

#[test]
fn set_meta_validates_fps() {
    let mut session = session_with_animation();
    let error = try_call(
        &mut session,
        "animation.set_meta",
        json!({ "id": "idle", "fps": 999.0 }),
    )
    .unwrap_err();
    assert_eq!(error_code(&error), -32602);
}

#[test]
fn play_pause_stop_cycle() {
    let mut session = session_with_animation();
    call(&mut session, "animation.bake", json!({ "id": "idle", "baked_at": 1 }));
    call(&mut session, "animation.open", json!({ "id": "idle" }));

    let playing = call(&mut session, "animation.play", json!({ "speed": 2.0 }));
    assert_eq!(playing["playing"], json!(true));
    assert_eq!(playing["speed"], json!(2.0));

    let paused = call(&mut session, "animation.pause", json!({}));
    assert_eq!(paused["playing"], json!(false));

    let resumed = call(&mut session, "animation.resume", json!({}));
    assert_eq!(resumed["playing"], json!(true));

    let stopped = call(&mut session, "animation.stop", json!({}));
    assert_eq!(stopped["playing"], json!(false));
    assert_eq!(stopped["time"], json!(0.0), "stop 应回到区间起点");
}

#[test]
fn set_speed_rejects_non_finite() {
    let mut session = session_with_animation();
    call(&mut session, "animation.bake", json!({ "id": "idle", "baked_at": 1 }));
    call(&mut session, "animation.open", json!({ "id": "idle" }));
    // JSON 里表示不出 NaN，用字符串试探应当直接失败
    let error = try_call(&mut session, "animation.set_speed", json!({ "speed": "fast" })).unwrap_err();
    assert_eq!(error_code(&error), -32602);
}

#[test]
fn playback_methods_require_prerender_mode() {
    let mut session = session_with_animation();
    let error = try_call(&mut session, "animation.pause", json!({})).unwrap_err();
    assert_eq!(error_code(&error), -32010);

    let error = try_call(&mut session, "animation.set_speed", json!({ "speed": 2.0 })).unwrap_err();
    assert_eq!(error_code(&error), -32010);
}

// ------------------------------------------------------------ 持久化

#[test]
fn animation_survives_project_save_load() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo");
    am_format::Project::create("demo", &root, am_format::CreateOptions::default()).unwrap();

    let mut session = session_with_animation();
    call(&mut session, "animation.bake", json!({ "id": "idle", "baked_at": 1 }));
    call(&mut session, "project.save", json!({ "path": root.to_string_lossy() }));

    // 重新打开
    let mut reopened = Session::empty();
    call(
        &mut reopened,
        "project.load",
        json!({ "path": root.to_string_lossy() }),
    );
    let list = call(&mut reopened, "animation.list", json!({}));
    let items = list.as_array().unwrap();
    assert_eq!(items.len(), 1, "动画应随工程保存");
    assert_eq!(items[0]["id"], json!("idle"));
    assert_eq!(items[0]["baked"], json!(true), "烘焙结果也应保留");

    // 烘焙轨里的值仍可采样
    let sample = call(
        &mut reopened,
        "animation.sample",
        json!({ "id": "idle", "time": 0.5 }),
    );
    let value = sample["params"]["AngleX"].as_f64().unwrap();
    assert!((value - 15.0).abs() < 1e-3, "重开后应能采到 15，实际 {value}");
}

#[test]
fn deleted_animation_stays_deleted_after_reload() {
    // 只写不删会导致「删掉的动画重开又回来」——这是编辑器侧修过的同类缺陷
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo");
    am_format::Project::create("demo", &root, am_format::CreateOptions::default()).unwrap();

    let mut session = Session::empty();
    call(&mut session, "animation.new", json!({ "id": "idle", "name": "I" }));
    call(&mut session, "animation.new", json!({ "id": "walk", "name": "W" }));
    call(&mut session, "project.save", json!({ "path": root.to_string_lossy() }));

    call(&mut session, "animation.delete", json!({ "id": "walk" }));
    call(&mut session, "project.save", json!({ "path": root.to_string_lossy() }));

    let mut reopened = Session::empty();
    call(
        &mut reopened,
        "project.load",
        json!({ "path": root.to_string_lossy() }),
    );
    let list = call(&mut reopened, "animation.list", json!({}));
    let items = list.as_array().unwrap();
    assert_eq!(items.len(), 1, "被删的动画不应复活");
    assert_eq!(items[0]["id"], json!("idle"));
}
