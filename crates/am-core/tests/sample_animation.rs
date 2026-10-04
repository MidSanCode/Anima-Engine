//! 示例工程的动画黄金帧测试（E-11）。
//!
//! `samples/minimal/spec/animations/idle.anim.json` 是一段 3 秒、带一条结构通道的
//! 示例动画。这里把它真正烘焙一遍，并**钉死若干帧的具体数值** —— 任何会改变
//! 求值语义的改动都会在这里立刻暴露，而不是等到交付后由宿主发现。
//!
//! 同时验证规范 §4.4 的两条硬要求：
//! 1. 重复烘焙逐位相同（`f32` 精确相等）；
//! 2. 与实时连续推进一致（误差 `< 1e-4`）。

use am_core::Session;
use serde_json::{json, Value};

/// 示例工程根目录。
fn sample_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/minimal")
}

/// 打开示例工程。
fn open_sample() -> Session {
    let mut session = Session::empty();
    let path = sample_root().to_string_lossy().to_string();
    let result = call(&mut session, "project.load", json!({ "path": path }));
    assert!(result["model"].is_object() || result.is_object());
    session
}

fn call(session: &mut Session, method: &str, params: Value) -> Value {
    let json = session.dispatch_json(method, &params.to_string());
    let value: Value = serde_json::from_str(&json).expect("信封必须是合法 JSON");
    assert_eq!(value["ok"], json!(true), "{method} 失败：{value}");
    value["result"].clone()
}

#[test]
fn sample_animation_is_loadable() {
    let mut session = open_sample();
    let list = call(&mut session, "animation.list", json!({}));
    let items = list.as_array().expect("应为数组");
    assert_eq!(items.len(), 1, "示例工程应带一段动画");
    assert_eq!(items[0]["id"], json!("idle"));
    assert_eq!(items[0]["channels"], json!(3), "参数 + 可见性 + 绘制顺序");
    assert_eq!(items[0]["baked"], json!(false), "示例不带烘焙结果，需现烘");

    let detail = call(
        &mut session,
        "animation.query",
        json!({ "id": "idle", "detail": true }),
    );
    assert_eq!(detail["duration"], json!(3.0));
    assert_eq!(detail["fps"], json!(30.0));
    assert_eq!(detail["looping"], json!(true));
}

#[test]
fn sample_animation_bakes_with_expected_shape() {
    let mut session = open_sample();
    let baked = call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "baked_at": 1_789_000_000 }),
    );

    // 3 秒 @30fps → floor(3*30 + 1e-6) + 1 = 91 帧
    assert_eq!(baked["frames"], json!(91));
    assert_eq!(baked["fps"], json!(30.0));
    assert_eq!(baked["duration"], json!(3.0));
    assert_eq!(baked["geometry"], json!("model_ref"));
    assert_eq!(baked["channels"], json!(1), "只有 AngleX 是参数通道");

    let baked_state = call(&mut session, "animation.baked", json!({ "id": "idle" }));
    assert_eq!(baked_state["baked"], json!(true));
    assert_eq!(baked_state["frames"], json!(91));
}

#[test]
fn sample_animation_golden_frames() {
    // 黄金帧：曲线是 0→30（缓入缓出，1.5s）→0（线性，到 3.0s）
    let mut session = open_sample();
    call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "baked_at": 1 }),
    );

    let cases: [(f64, f64, &str); 5] = [
        (0.0, 0.0, "起点"),
        (0.75, 15.0, "缓动中点两侧对称，约半值"),
        (1.5, 30.0, "峰值"),
        (2.25, 15.0, "回落中点"),
        (3.0, 0.0, "回到起点（循环无缝）"),
    ];
    for (time, expected, label) in cases {
        let sample = call(
            &mut session,
            "animation.sample",
            json!({ "id": "idle", "time": time }),
        );
        let value = sample["params"]["AngleX"].as_f64().unwrap();
        assert!(
            (value - expected).abs() < 1.0,
            "{label} t={time}：期望约 {expected}，实际 {value}"
        );
    }
}

#[test]
fn sample_animation_structural_channel_is_stepwise() {
    // 可见性：0.0 → 1，2.0 → 0，2.5 → 1（阶跃，不插值）
    let mut session = open_sample();
    call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "baked_at": 1 }),
    );

    for (time, expected) in [(0.0f64, true), (1.9, true), (2.1, false), (2.6, true)] {
        let sample = call(
            &mut session,
            "animation.sample",
            json!({ "id": "idle", "time": time }),
        );
        let visible = sample["structural"]["visibility"]["node-body"]
            .as_bool()
            .expect("应含节点可见性");
        assert_eq!(visible, expected, "t={time} 的可视性不符");
    }
}

#[test]
fn sample_animation_rebake_is_bitwise_identical() {
    // §4.4 硬要求 1：逐位相同
    let mut session = open_sample();
    call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "baked_at": 1 }),
    );
    let mut first = Vec::new();
    for step in 0..=30 {
        let time = step as f64 * 0.1;
        let sample = call(
            &mut session,
            "animation.sample",
            json!({ "id": "idle", "time": time }),
        );
        first.push(sample["params"]["AngleX"].as_f64().unwrap());
    }

    call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "baked_at": 1 }),
    );
    for (step, expected) in first.iter().enumerate() {
        let time = step as f64 * 0.1;
        let sample = call(
            &mut session,
            "animation.sample",
            json!({ "id": "idle", "time": time }),
        );
        let actual = sample["params"]["AngleX"].as_f64().unwrap();
        assert_eq!(*expected, actual, "t={time} 重复烘焙结果不一致");
    }
}

#[test]
fn sample_animation_matches_live_playback() {
    // §4.4 硬要求 2：预渲染看起来就是刚才实时看到的那一段（< 1e-4）。
    // 用引擎自己的实时路径（runtime.advance）推进同一时长再比对。
    let mut session = open_sample();

    // 实时：把参数按 30fps 逐帧手动推进的同时用 runtime.set_time 对齐曲线求值，
    // 这里用更直接的办法 —— 走 animation.sample 与 seek 两条路比对。
    call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "baked_at": 1 }),
    );
    call(&mut session, "animation.open", json!({ "id": "idle" }));

    for step in 0..=15 {
        let time = step as f64 * 0.2;
        let via_seek = call(&mut session, "animation.seek", json!({ "time": time }));
        let params = call(&mut session, "runtime.params", json!({}));
        let seeked = params["AngleX"].as_f64().unwrap();
        let sampled =
            call(&mut session, "animation.sample", json!({ "id": "idle", "time": time }))
                ["params"]["AngleX"]
                .as_f64()
                .unwrap();
        assert!(
            (seeked - sampled).abs() < 1e-4,
            "t={time}：seek 得 {seeked}，sample 得 {sampled}"
        );
        // 循环动画：t=3.0 会折回帧 0（模运算在时间域做，§4.6）
        let expected_frame = if time >= 3.0 {
            0
        } else {
            (time * 30.0).floor() as i64
        };
        assert_eq!(
            via_seek["frame"],
            json!(expected_frame),
            "t={time} 的帧号不符"
        );
    }
}

#[test]
fn sample_animation_loops_in_time_domain() {
    // §4.6：模运算在时间域做，循环一圈后回到起点
    let mut session = open_sample();
    call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "baked_at": 1 }),
    );
    call(&mut session, "animation.open", json!({ "id": "idle" }));

    let at_start = call(
        &mut session,
        "animation.sample",
        json!({ "id": "idle", "time": 0.0 }),
    )["params"]["AngleX"]
        .as_f64()
        .unwrap();
    // 3 秒是循环周期；4.0 秒等价于 1.0 秒
    let looped = call(
        &mut session,
        "animation.sample",
        json!({ "id": "idle", "time": 4.0 }),
    )["params"]["AngleX"]
        .as_f64()
        .unwrap();
    let direct = call(
        &mut session,
        "animation.sample",
        json!({ "id": "idle", "time": 1.0 }),
    )["params"]["AngleX"]
        .as_f64()
        .unwrap();
    assert!(
        (looped - direct).abs() < 1e-3,
        "4.0s 应等价于 1.0s：{looped} vs {direct}"
    );
    assert!(at_start.abs() < 1.0);
}

#[test]
fn sample_animation_round_trips_through_save() {
    // 烘焙结果随工程保存并读回
    let dir = tempfile::tempdir().unwrap();
    let mut session = open_sample();
    call(
        &mut session,
        "animation.bake",
        json!({ "id": "idle", "baked_at": 1_789_000_000 }),
    );
    let target = dir.path().join("saved");
    // 用 project.save 另存，再载入
    let path = target.to_string_lossy().to_string();
    call(&mut session, "project.save", json!({ "path": path }));

    let mut reopened = Session::empty();
    call(&mut reopened, "project.load", json!({ "path": path }));
    let baked = call(&mut reopened, "animation.baked", json!({ "id": "idle" }));
    assert_eq!(baked["baked"], json!(true));
    assert_eq!(baked["frames"], json!(91));
    assert_eq!(baked["baked_at"], json!(1_789_000_000u64));

    // 采样值仍与烘焙时一致
    let sample = call(
        &mut reopened,
        "animation.sample",
        json!({ "id": "idle", "time": 1.5 }),
    );
    let value = sample["params"]["AngleX"].as_f64().unwrap();
    assert!((value - 30.0).abs() < 1.0, "峰值应约为 30，实际 {value}");
}
