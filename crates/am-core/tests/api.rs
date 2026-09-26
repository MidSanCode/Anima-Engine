//! 契约测试：**只**通过 JSON 字符串调用引擎，验证 FFI 协议的形状与稳定性。
//!
//! 编辑器与查看器看到的就是这一层，因此这里的用例等价于「接口回归测试」。

use am_core::{codes, Session, METHODS};
use am_model::{
    Canvas, Expression, ExpressionParam, Mesh, Model, Motion, MotionCurve, MotionKey, Node,
    Parameter, Spec, TextureRef,
};
use am_math::{Rect, Vec2};
use serde_json::{json, Value};

fn quad(size: f32) -> Mesh {
    Mesh::new(
        vec![
            Vec2::new(-size, -size),
            Vec2::new(size, -size),
            Vec2::new(size, size),
            Vec2::new(-size, size),
        ],
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

/// 一个带纹理、参数、动作与表情的示例会话。
fn demo_session() -> Session {
    let mut model = Model::new("demo");
    model.canvas = Canvas::new(512.0, 512.0);
    model.add_texture(TextureRef::new("assets/images/0.png"));
    model.add_parameter(Parameter::new("AngleX", "Angle X", -30.0, 30.0, 0.0));

    let part = model.add_node(Node::part("Root", None));
    let drawable = model.add_node(Node::drawable("Body", Some(part), quad(64.0)));
    model.node_mut(&drawable).unwrap().drawable.as_mut().unwrap().texture = Some(0);

    let mut motion = Motion::new("idle", "Idle");
    motion.duration = 1.0;
    motion.fade_in = 0.0;
    let mut curve = MotionCurve::new("AngleX");
    curve.insert_key(MotionKey::new(0.0, -10.0));
    curve.insert_key(MotionKey::new(1.0, 10.0));
    motion.curves.push(curve);

    let mut expression = Expression::new("smile", "Smile");
    expression.parameters.push(ExpressionParam::new("AngleX", 20.0));

    let mut spec = Spec::new(model);
    spec.motions.push(motion);
    spec.expressions.push(expression);
    Session::with_spec(spec)
}

/// 调用并返回 `result`（断言成功）。
fn ok(session: &mut Session, method: &str, params: Value) -> Value {
    let raw = session.dispatch_json(method, &params.to_string());
    let value: Value = serde_json::from_str(&raw).expect("信封必须是合法 JSON");
    assert_eq!(value["ok"], Value::Bool(true), "{method} 失败：{raw}");
    value["result"].clone()
}

/// 调用并返回错误码（断言失败）。
fn err_code(session: &mut Session, method: &str, params: Value) -> i32 {
    let raw = session.dispatch_json(method, &params.to_string());
    let value: Value = serde_json::from_str(&raw).expect("信封必须是合法 JSON");
    assert_eq!(value["ok"], Value::Bool(false), "{method} 不应成功：{raw}");
    value["error"]["code"].as_i64().expect("错误必须带 code") as i32
}

#[test]
fn ping_and_version_have_stable_shape() {
    let mut session = demo_session();
    let pong = ok(&mut session, "system.ping", json!({}));
    assert_eq!(pong["pong"], Value::Bool(true));
    assert!(pong["version"].is_string());

    let version = ok(&mut session, "system.version", json!({}));
    assert_eq!(version["format"], Value::String("amproj".into()));
    assert!(version["format_version"].is_number());
}

#[test]
fn capabilities_advertise_every_method() {
    let mut session = demo_session();
    let caps = ok(&mut session, "system.capabilities", json!({}));
    let listed: Vec<String> = caps["methods"]
        .as_array()
        .expect("methods 必须是数组")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(listed.len(), METHODS.len());
    for method in METHODS {
        assert!(listed.contains(&method.to_string()), "缺少方法声明：{method}");
    }
    assert!(caps["physics"].as_bool().unwrap());
    assert!(caps["motion"].as_bool().unwrap());
    assert!(caps["renderer"].is_boolean());
}

#[test]
fn every_declared_method_is_dispatchable() {
    // 声明了就必须能路由：要么成功，要么返回结构化错误，**不能**是「未知方法」
    let mut session = demo_session();
    let mut seen = std::collections::BTreeSet::new();
    for method in METHODS {
        assert!(seen.insert(*method), "方法清单有重复项：{method}");
        let raw = session.dispatch_json(method, "{}");
        let value: Value = serde_json::from_str(&raw).expect("信封必须是合法 JSON");
        if value["ok"] == Value::Bool(false) {
            let code = value["error"]["code"].as_i64().unwrap() as i32;
            assert_ne!(code, codes::METHOD_NOT_FOUND, "方法未实现：{method}");
        }
    }
}

#[test]
fn unknown_method_is_rejected() {
    let mut session = demo_session();
    assert_eq!(
        err_code(&mut session, "nope.nope", json!({})),
        codes::METHOD_NOT_FOUND
    );
}

#[test]
fn malformed_params_are_rejected() {
    let mut session = demo_session();
    let raw = session.dispatch_json("system.ping", "{not json");
    let value: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["error"]["code"].as_i64().unwrap() as i32, codes::INVALID_REQUEST);
}

#[test]
fn empty_params_string_is_accepted() {
    let mut session = demo_session();
    let raw = session.dispatch_json("system.ping", "");
    let value: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["ok"], Value::Bool(true));
}

#[test]
fn project_new_resets_the_document() {
    let mut session = demo_session();
    ok(&mut session, "project.new", json!({ "name": "fresh", "width": 256, "height": 128 }));
    let model = ok(&mut session, "doc.model", json!({}));
    assert_eq!(model["name"], Value::String("fresh".into()));
    assert_eq!(model["canvas"]["width"].as_f64().unwrap(), 256.0);
    assert_eq!(model["nodes"].as_array().unwrap().len(), 0);
}

#[test]
fn commands_and_undo_go_through_json() {
    let mut session = demo_session();
    let before = ok(&mut session, "doc.model", json!({}))["nodes"].as_array().unwrap().len();

    ok(
        &mut session,
        "doc.command",
        json!({ "command": { "op": "node_create", "kind": "part", "name": "Extra" } }),
    );
    let after = ok(&mut session, "doc.model", json!({}))["nodes"].as_array().unwrap().len();
    assert_eq!(after, before + 1);

    let history = ok(&mut session, "doc.history", json!({}));
    assert_eq!(history["can_undo"], Value::Bool(true));
    assert_eq!(history["undo"][0], Value::String("node.create".into()));

    ok(&mut session, "doc.undo", json!({}));
    let undone = ok(&mut session, "doc.model", json!({}))["nodes"].as_array().unwrap().len();
    assert_eq!(undone, before);

    ok(&mut session, "doc.redo", json!({}));
    let redone = ok(&mut session, "doc.model", json!({}))["nodes"].as_array().unwrap().len();
    assert_eq!(redone, before + 1);
}

#[test]
fn invalid_command_reports_invalid_params() {
    let mut session = demo_session();
    assert_eq!(
        err_code(
            &mut session,
            "doc.command",
            json!({ "command": { "op": "node_delete", "node": "ghost" } })
        ),
        codes::INVALID_PARAMS
    );
    assert_eq!(err_code(&mut session, "doc.command", json!({})), codes::INVALID_PARAMS);
}

#[test]
fn parameters_round_trip_and_are_clamped() {
    let mut session = demo_session();
    let result = ok(&mut session, "runtime.set_param", json!({ "id": "AngleX", "value": 999.0 }));
    assert_eq!(result["value"].as_f64().unwrap(), 30.0, "应被钳制到参数上限");

    ok(&mut session, "runtime.set_param", json!({ "id": "AngleX", "normalized": 0.0 }));
    let params = ok(&mut session, "runtime.params", json!({}));
    assert_eq!(params["AngleX"].as_f64().unwrap(), -30.0);

    assert_eq!(
        err_code(&mut session, "runtime.set_param", json!({ "id": "ghost", "value": 1.0 })),
        codes::INVALID_PARAMS
    );
    ok(&mut session, "runtime.reset_params", json!({}));
    let params = ok(&mut session, "runtime.params", json!({}));
    assert_eq!(params["AngleX"].as_f64().unwrap(), 0.0);
}

#[test]
fn runtime_advance_and_pause() {
    let mut session = demo_session();
    let state = ok(&mut session, "runtime.advance", json!({ "dt": 0.5 }));
    assert_eq!(state["frame"].as_u64().unwrap(), 1);
    assert!((state["time"].as_f64().unwrap() - 0.5).abs() < 1e-6);

    ok(&mut session, "runtime.pause", json!({}));
    ok(&mut session, "runtime.advance", json!({ "dt": 0.5 }));
    assert_eq!(ok(&mut session, "runtime.state", json!({}))["frame"].as_u64().unwrap(), 1);

    ok(&mut session, "runtime.resume", json!({}));
    ok(&mut session, "runtime.advance", json!({ "dt": 0.5 }));
    assert_eq!(ok(&mut session, "runtime.state", json!({}))["frame"].as_u64().unwrap(), 2);
}

#[test]
fn scene_exposes_drawables_for_the_renderer() {
    let mut session = demo_session();
    let scene = ok(&mut session, "runtime.scene", json!({}));
    let drawables = scene["drawables"].as_array().unwrap();
    assert_eq!(drawables.len(), 1);
    assert_eq!(drawables[0]["vertices"].as_array().unwrap().len(), 4);
    assert_eq!(drawables[0]["indices"].as_array().unwrap().len(), 6);
    assert_eq!(scene["nodes"].as_object().unwrap().len(), 2);
}

#[test]
fn motions_play_through_json() {
    let mut session = demo_session();
    let list = ok(&mut session, "motion.list", json!({}));
    assert_eq!(list[0]["id"], Value::String("idle".into()));

    let state = ok(&mut session, "motion.play", json!({ "id": "idle", "looping": false }));
    assert_eq!(state["playing"], Value::Bool(true));

    ok(&mut session, "runtime.advance", json!({ "dt": 0.5 }));
    let params = ok(&mut session, "runtime.params", json!({}));
    assert!((params["AngleX"].as_f64().unwrap() - 0.0).abs() < 1.0, "动作应驱动参数");

    ok(&mut session, "motion.seek", json!({ "time": 1.0 }));
    ok(&mut session, "runtime.advance", json!({ "dt": 0.5 }));
    let params = ok(&mut session, "runtime.params", json!({}));
    assert!((params["AngleX"].as_f64().unwrap() - 10.0).abs() < 1e-3);

    ok(&mut session, "motion.stop", json!({}));
    assert_eq!(ok(&mut session, "motion.state", json!({}))["playing"], Value::Bool(false));

    assert_eq!(
        err_code(&mut session, "motion.play", json!({ "id": "ghost" })),
        codes::INVALID_PARAMS
    );
}

#[test]
fn expressions_apply_to_parameters() {
    let mut session = demo_session();
    let list = ok(&mut session, "expression.list", json!({}));
    assert_eq!(list[0]["id"], Value::String("smile".into()));

    ok(&mut session, "expression.set", json!({ "id": "smile", "weight": 1.0 }));
    let params = ok(&mut session, "runtime.params", json!({}));
    assert!((params["AngleX"].as_f64().unwrap() - 20.0).abs() < 1e-3);

    // 权重是相对于当前取值的插值：先归零再看半权重
    ok(&mut session, "runtime.reset_params", json!({}));
    ok(&mut session, "expression.set", json!({ "id": "smile", "weight": 0.5 }));
    let params = ok(&mut session, "runtime.params", json!({}));
    assert!((params["AngleX"].as_f64().unwrap() - 10.0).abs() < 1e-3);

    assert_eq!(
        err_code(&mut session, "expression.set", json!({ "id": "ghost" })),
        codes::INVALID_PARAMS
    );
}

#[test]
fn physics_methods_are_safe_without_settings() {
    let mut session = demo_session();
    let info = ok(&mut session, "physics.info", json!({}));
    assert_eq!(info["settings"].as_u64().unwrap(), 0);
    ok(&mut session, "physics.step", json!({ "dt": 1.0 / 60.0 }));
    ok(&mut session, "physics.reset", json!({}));
}

#[test]
fn diagnostics_report_counts() {
    let mut session = demo_session();
    let stats = ok(&mut session, "diagnostics.stats", json!({}));
    assert_eq!(stats["nodes"].as_u64().unwrap(), 2);
    assert_eq!(stats["parameters"].as_u64().unwrap(), 1);
    assert_eq!(stats["textures"].as_u64().unwrap(), 1);
    assert_eq!(stats["motions"].as_u64().unwrap(), 1);
    assert_eq!(stats["expressions"].as_u64().unwrap(), 1);
    assert_eq!(stats["drawables"].as_u64().unwrap(), 1);
}

#[test]
fn renderer_info_before_init_is_harmless() {
    let mut session = demo_session();
    let info = ok(&mut session, "renderer.info", json!({}));
    assert_eq!(info["initialized"], Value::Bool(false));
    // 未初始化时调用渲染相关方法必须给出明确错误，而不是 panic
    assert_eq!(
        err_code(&mut session, "renderer.render", json!({})),
        codes::UNSUPPORTED
    );
}

#[test]
fn project_save_and_load_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo");
    let mut session = demo_session();
    ok(
        &mut session,
        "project.save",
        json!({ "path": root.to_string_lossy() }),
    );
    assert!(root.join("spec").join("model.json").is_file());

    let mut other = Session::empty();
    let loaded = ok(
        &mut other,
        "project.load",
        json!({ "path": root.to_string_lossy() }),
    );
    assert_eq!(loaded["nodes"].as_u64().unwrap(), 2);
    assert_eq!(loaded["motions"].as_u64().unwrap(), 1);

    let model = ok(&mut other, "doc.model", json!({}));
    assert_eq!(model["name"], Value::String("demo".into()));
    let list = ok(&mut other, "motion.list", json!({}));
    assert_eq!(list[0]["id"], Value::String("idle".into()));
}

#[test]
fn project_spec_can_be_read_and_replaced() {
    let mut session = demo_session();
    let spec = ok(&mut session, "project.spec", json!({}));
    assert_eq!(spec["model"]["name"], Value::String("demo".into()));
    assert_eq!(spec["motions"].as_array().unwrap().len(), 1);

    ok(&mut session, "project.new", json!({ "name": "empty" }));
    ok(&mut session, "project.set_spec", json!({ "spec": spec }));
    let model = ok(&mut session, "doc.model", json!({}));
    assert_eq!(model["name"], Value::String("demo".into()));
    assert_eq!(ok(&mut session, "motion.list", json!({}))[0]["id"], Value::String("idle".into()));
}

#[test]
fn validate_reports_issues() {
    let mut session = demo_session();
    let report = ok(&mut session, "project.validate", json!({}));
    assert!(report["ok"].is_boolean());
    assert!(report["issues"].is_array());
}

#[test]
fn doc_set_model_replaces_everything() {
    let mut session = demo_session();
    let mut model = Model::new("other");
    model.canvas = Canvas::new(64.0, 64.0);
    model.add_node(Node::warp_deformer("W", None, 1, 1, Rect::from_min_max(Vec2::ZERO, Vec2::splat(4.0))));
    let value = serde_json::to_value(&model).unwrap();
    ok(&mut session, "doc.set_model", json!({ "model": value }));

    let back = ok(&mut session, "doc.model", json!({}));
    assert_eq!(back["name"], Value::String("other".into()));
    assert_eq!(ok(&mut session, "motion.list", json!({})).as_array().unwrap().len(), 0);
    assert_eq!(ok(&mut session, "diagnostics.stats", json!({}))["nodes"].as_u64().unwrap(), 1);
}
