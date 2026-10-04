//! Schema 一致性测试：`schemas/*.schema.json` 必须能约束真实文件与真实调用数据。
//!
//! 这是「文档即代码」的落点——schema 漂移会直接挂测试。

use jsonschema::Validator;
use serde_json::{json, Value};
use std::path::PathBuf;

fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas")
}

fn load_schema(name: &str) -> Value {
    let path = schema_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取 {} 失败: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("解析 {} 失败: {e}", path.display()))
}

/// 编译整个文档，但把根 `$ref` 指到 `#/definitions/<name>`（保持文档内引用解析正确）。
fn compile_subschema(name: &str, definition: &str) -> Validator {
    let mut root = load_schema(name);
    let object = root.as_object_mut().expect("schema 根必须是对象");
    object.insert("$ref".into(), json!(format!("#/definitions/{definition}")));
    jsonschema::validator_for(&root).expect("schema 编译失败")
}

fn compile(name: &str) -> Validator {
    jsonschema::validator_for(&load_schema(name)).expect("schema 编译失败")
}

fn read_json(rel: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读取 {} 失败: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("解析 {} 失败: {e}", path.display()))
}

#[test]
fn all_schemas_are_valid_json() {
    for name in [
        "envelope.schema.json",
        "format.schema.json",
        "model.schema.json",
        "command.schema.json",
        "spec.schema.json",
    ] {
        let schema = load_schema(name);
        assert_eq!(schema["$schema"], "http://json-schema.org/draft-07/schema#", "{name}");
    }
}

#[test]
fn sample_project_files_match_the_format_schema() {
    let info = compile_subschema("format.schema.json", "info");
    let registry = compile_subschema("format.schema.json", "registry");
    let metadata = compile_subschema("format.schema.json", "asset_metadata");

    let doc = read_json("../../samples/minimal/info.json");
    let errors: Vec<_> = info.iter_errors(&doc).collect();
    assert!(errors.is_empty(), "info.json: {errors:?}");
    let doc = read_json("../../samples/minimal/registry.json");
    let errors: Vec<_> = registry.iter_errors(&doc).collect();
    assert!(errors.is_empty(), "registry.json: {errors:?}");
    let doc = read_json("../../samples/minimal/metadata/images/0.png.json");
    let errors: Vec<_> = metadata.iter_errors(&doc).collect();
    assert!(errors.is_empty(), "metadata: {errors:?}");
}

#[test]
fn sample_model_matches_the_model_schema() {
    let validator = compile("model.schema.json");
    let doc = read_json("../../samples/minimal/spec/model.json");
    let errors: Vec<_> = validator.iter_errors(&doc).collect();
    assert!(errors.is_empty(), "model.json: {errors:?}");
}

#[test]
fn sample_animation_matches_the_animation_schema() {
    let validator = compile_subschema("spec.schema.json", "animation");
    let doc = read_json("../../samples/minimal/spec/animations/idle.anim.json");
    let errors: Vec<_> = validator.iter_errors(&doc).collect();
    assert!(errors.is_empty(), "idle.anim.json: {errors:?}");
}

#[test]
fn animation_schema_rejects_bad_kind_and_id() {
    // schema 不是装饰：坏数据必须被挡下
    let validator = compile_subschema("spec.schema.json", "animation");
    let mut doc = read_json("../../samples/minimal/spec/animations/idle.anim.json");
    doc["channels"][0]["kind"] = json!("teleport");
    let errors: Vec<_> = validator.iter_errors(&doc).collect();
    assert!(!errors.is_empty(), "未知通道种类应被拒绝");

    let mut doc = read_json("../../samples/minimal/spec/animations/idle.anim.json");
    doc["id"] = json!("Idle"); // 大写不允许
    let errors: Vec<_> = validator.iter_errors(&doc).collect();
    assert!(!errors.is_empty(), "大写动画 id 应被拒绝");
}

#[test]
fn easing_schema_matches_the_engine_representation() {
    // serde tag = "type"，形状是 {type: cubic_bezier, p1, p2}，
    // 不是 {cubic_bezier: {...}} —— 这条差异曾经真实存在过。
    let validator = compile_subschema("spec.schema.json", "easing");
    for good in [
        json!({ "type": "linear" }),
        json!({ "type": "step" }),
        json!({ "type": "ease_in_out" }),
        json!({ "type": "cubic_bezier", "p1": { "x": 0.42, "y": 0.0 }, "p2": { "x": 0.58, "y": 1.0 } }),
    ] {
        let errors: Vec<_> = validator.iter_errors(&good).collect();
        assert!(errors.is_empty(), "{good} 应被接受：{errors:?}");
    }
    // 旧形状必须被拒绝，避免 schema 与引擎再次漂移
    let stale = json!({ "cubic_bezier": { "p1": { "x": 0.1, "y": 0.1 }, "p2": { "x": 0.9, "y": 0.9 } } });
    let errors: Vec<_> = validator.iter_errors(&stale).collect();
    assert!(!errors.is_empty(), "旧形状不应被接受");

    let missing = json!({ "type": "cubic_bezier" });
    let errors: Vec<_> = validator.iter_errors(&missing).collect();
    assert!(!errors.is_empty(), "缺 p1/p2 应被拒绝");
}

#[test]
fn spec_schema_accepts_the_sample_spec() {
    let validator = compile("spec.schema.json");
    let spec = json!({
        "model": read_json("../../samples/minimal/spec/model.json"),
        "physics": {
            "enabled": true,
            "fps": 60.0,
            "gravity": { "x": 0.0, "y": -1.0 },
            "wind": { "x": 0.0, "y": 0.0 },
            "settings": []
        },
        "motions": [
            {
                "id": "idle",
                "name": "待机",
                "duration": 2.0,
                "looping": true,
                "fps": 30.0,
                "fade_in": 0.5,
                "fade_out": 0.5,
                "curves": [
                    {
                        "target": "AngleX",
                        "kind": "parameter",
                        "keys": [
                            { "time": 0.0, "value": -30.0 },
                            { "time": 1.0, "value": 30.0, "easing": { "type": "ease_in_out" } },
                            { "time": 2.0, "value": -30.0,
                              "easing": { "type": "cubic_bezier", "p1": [0.3, 0.0], "p2": [0.7, 1.0] } }
                        ]
                    }
                ]
            }
        ],
        "expressions": [
            {
                "id": "smile",
                "name": "微笑",
                "fade_in": 0.2,
                "fade_out": 0.2,
                "parameters": [ { "parameter": "AngleX", "value": 10.0, "blend": "additive", "weight": 1.0 } ]
            }
        ],
        "animations": [
            read_json("../../samples/minimal/spec/animations/idle.anim.json")
        ]
    });
    let errors: Vec<_> = validator.iter_errors(&spec).collect();
    assert!(errors.is_empty(), "spec: {errors:?}");
}

#[test]
fn commands_match_the_command_schema() {
    let validator = compile("command.schema.json");

    let batch = json!({
        "op": "batch",
        "label": "创建角色",
        "commands": [
            { "op": "node_create", "kind": "part", "name": "Root" },
            { "op": "node_create", "kind": "drawable", "name": "Body",
              "rect": { "min": [-64.0, -64.0], "max": [64.0, 64.0] } },
            { "op": "mesh_set", "node": "node-body",
              "vertices": [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
              "uvs": [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
              "indices": [0, 1, 2] },
            { "op": "drawable_set_blend", "node": "node-body", "blend": "additive" },
            { "op": "parameter_add", "id": "anglex", "name": "Angle X",
              "min": -30.0, "max": 30.0, "default": 0.0, "repeat": false },
            { "op": "node_set_rotation", "node": "node-body", "angle": 0.5 }
        ]
    });
    assert!(validator.is_valid(&batch), "batch 命令应通过校验");

    // 非法样例
    assert!(!validator.is_valid(&json!({ "op": "nope" })), "未知 op 必须被拒绝");
    assert!(!validator.is_valid(&json!({ "op": "node_delete" })), "缺 node 必须被拒绝");
    assert!(
        !validator.is_valid(&json!({ "op": "node_delete", "node": "Body X" })),
        "node id 不得包含空格等字符"
    );
    assert!(
        !validator.is_valid(&json!({ "op": "drawable_set_blend", "node": "x", "blend": "burn" })),
        "未知混合模式必须被拒绝"
    );
}

#[test]
fn envelopes_match_the_envelope_schema() {
    let validator = compile("envelope.schema.json");
    assert!(validator.is_valid(&json!({ "ok": true, "result": {} })));
    assert!(validator.is_valid(&json!({ "ok": true, "result": [1, 2, 3] })));
    assert!(validator.is_valid(&json!({
        "ok": false,
        "error": { "code": -32602, "message": "参数不存在" }
    })));

    assert!(!validator.is_valid(&json!({ "ok": false })), "失败必须有 error");
    assert!(!validator.is_valid(&json!({ "ok": true })), "成功必须有 result");
    assert!(
        !validator.is_valid(&json!({
            "ok": false,
            "error": { "code": -99999, "message": "?" }
        })),
        "未知错误码必须被拒绝"
    );
}
