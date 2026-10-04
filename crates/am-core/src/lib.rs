//! `am-core`：引擎门面。
//!
//! 这一层把各个内核 crate 组装成一个**可驱动的会话**，并提供与语言无关的
//! JSON 方法分发（`method` + `params` → `result`）。上层（FFI / wasm / CLI / 编辑器）
//! 只依赖这里的字符串协议，不直接依赖任何 Rust 类型。
//!
//! ```text
//! FFI / wasm / CLI
//!        │  dispatch_json("runtime.advance", "{\"dt\":0.016}")
//!        ▼
//!     Session ──► Document（模型 + 撤销） ──► am-eval（求值）
//!        │      ──► MotionPlayer（动作）
//!        │      ──► PhysicsEngine（物理）
//!        └──────► Renderer（离屏渲染，feature = "gpu"）
//! ```
//!
//! 会话是**唯一**的可变状态持有者；`evaluate` 永远返回全新结果，不做隐式缓存，
//! 保证「同样的输入必然得到同样的输出」。

use am_doc::{Command, Document, DocumentError};
use am_eval::{evaluate, Evaluated};
use am_model::Spec;
use am_motion::{MotionError, MotionPlayer, PlayOptions};
use am_physics::PhysicsEngine;
use serde_json::{json, Value};

#[cfg(feature = "gpu")]
use am_render::{decode_file, DecodedImage, Renderer, View};

pub mod animation;
pub mod animation_dispatch;
pub mod project;

pub use animation::{
    find as find_animation, AnimationError, Mode, ModeGuard, ModeState,
};
pub use project::{read_spec, write_spec};

/// 引擎版本（同时用于 `system.version` 与项目文件）。
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 支持的方法清单（编辑器用它做能力探测）。
pub const METHODS: &[&str] = &[
    "system.ping",
    "system.version",
    "system.capabilities",
    "doc.command",
    "doc.undo",
    "doc.redo",
    "doc.history",
    "doc.model",
    "doc.set_model",
    "doc.evaluate",
    "project.new",
    "project.load",
    "project.save",
    "project.validate",
    "project.spec",
    "project.set_spec",
    "runtime.set_param",
    "runtime.params",
    "runtime.reset_params",
    "runtime.advance",
    "runtime.set_time",
    "runtime.pause",
    "runtime.resume",
    "runtime.state",
    "runtime.scene",
    "motion.list",
    "motion.play",
    "motion.stop",
    "motion.pause",
    "motion.resume",
    "motion.seek",
    "motion.state",
    "expression.list",
    "expression.set",
    "physics.info",
    "physics.step",
    "physics.reset",
    "renderer.info",
    "renderer.init",
    "renderer.resize",
    "renderer.set_view",
    "renderer.set_texture",
    "renderer.clear_textures",
    "renderer.render",
    "renderer.frame",
    "renderer.save_png",
    "diagnostics.stats",
    // 预渲染动画（见 `docs/animation-mode.md`）
    "animation.mode",
    "animation.new",
    "animation.open",
    "animation.close",
    "animation.play",
    "animation.pause",
    "animation.resume",
    "animation.stop",
    "animation.seek",
    "animation.set_speed",
    "animation.set_loop",
    "animation.state",
    "animation.list",
    "animation.query",
    "animation.set_meta",
    "animation.delete",
    "animation.channel.add",
    "animation.channel.remove",
    "animation.key.set",
    "animation.key.remove",
    "animation.key.move",
    "animation.channel.set_easing",
    "animation.curve.set",
    "animation.overlay.set",
    "animation.set_param_ref",
    "animation.bake",
    "animation.baked",
    "animation.sample",
];

// ------------------------------------------------------------------ 错误

/// 分发错误（直接映射为 FFI 的错误对象）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ApiError {
    pub code: i32,
    pub message: String,
    /// 可执行的修复建议（方法名之类）。规范 §9 允许错误带 `hint`，
    /// 编辑器可以直接把它渲染成「点这里修好」。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// JSON-RPC 风格的错误码。
pub mod codes {
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL: i32 = -32603;
    /// 平台不支持（例如没有 GPU 却调用 renderer.*）。
    pub const UNSUPPORTED: i32 = -32000;
    /// 模式冲突：预渲染模式下不能实时推进（见 `docs/animation-mode.md` §9）。
    pub const MODE_CONFLICT: i32 = -32010;
    /// 动画尚未烘焙。
    pub const NOT_BAKED: i32 = -32011;
    /// 动画不存在。
    pub const ANIMATION_NOT_FOUND: i32 = -32012;
}

impl ApiError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), hint: None }
    }

    /// 带修复建议的错误。
    pub fn with_hint(code: i32, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self { code, message: message.into(), hint: Some(hint.into()) }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(codes::INVALID_PARAMS, message)
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new(codes::METHOD_NOT_FOUND, format!("未知方法：{method}"))
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(codes::UNSUPPORTED, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(codes::INTERNAL, message)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<AnimationError> for ApiError {
    fn from(err: AnimationError) -> Self {
        Self {
            code: err.code(),
            message: err.to_string(),
            hint: err.hint().map(|h| h.to_string()),
        }
    }
}

impl From<DocumentError> for ApiError {
    fn from(err: DocumentError) -> Self {
        Self::invalid_params(err.to_string())
    }
}

impl From<MotionError> for ApiError {
    fn from(err: MotionError) -> Self {
        Self::invalid_params(err.to_string())
    }
}

impl From<am_format::FormatError> for ApiError {
    fn from(err: am_format::FormatError) -> Self {
        Self::new(codes::INTERNAL, err.to_string())
    }
}

// ------------------------------------------------------------------ 会话

/// 一个可驱动的引擎会话。
pub struct Session {
    doc: Document,
    /// 除结构模型外的描述层（物理 / 动作 / 表情 / 姿势 / 设置）。
    ///
    /// `spec.model` 是结构模型的**镜像**，只在读写工程与序列化时使用；
    /// 结构编辑一律走 `doc`（撤销栈），调用 [`Session::spec`] 时会自动同步。
    spec: Spec,
    motions: MotionPlayer,
    physics: PhysicsEngine,
    time: f64,
    frame: u64,
    paused: bool,
    expression: Option<String>,
    /// 预渲染动画库（`spec/animations/`）。
    animations: am_anim::AnimationSet,
    /// 模式状态机（`live` / `prerender`）。
    mode: ModeState,
    #[cfg(feature = "gpu")]
    renderer: Option<Renderer>,
    #[cfg(feature = "gpu")]
    view: View,
    #[cfg(feature = "gpu")]
    pixels: Vec<u8>,
    #[cfg(feature = "gpu")]
    generation: u64,
}

impl Session {
    /// 由模型创建会话（动作与物理自动从模型装载）。
    pub fn new(model: am_model::Model) -> Self {
        Self::with_spec(Spec::new(model))
    }

    /// 由完整描述层创建会话。
    pub fn with_spec(spec: Spec) -> Self {
        let mut spec = spec;
        let model = std::mem::take(&mut spec.model);
        let mut session = Self {
            doc: Document::from_model(model),
            spec,
            motions: MotionPlayer::new(),
            physics: PhysicsEngine::new(&am_model::PhysicsSettings::default()),
            time: 0.0,
            frame: 0,
            paused: false,
            expression: None,
            animations: am_anim::AnimationSet::default(),
            mode: ModeState::live(),
            #[cfg(feature = "gpu")]
            renderer: None,
            #[cfg(feature = "gpu")]
            view: View::new(am_math::Vec2::ZERO, 1.0),
            #[cfg(feature = "gpu")]
            pixels: Vec::new(),
            #[cfg(feature = "gpu")]
            generation: 0,
        };
        session.reload_runtime();
        session
    }

    /// 空会话。
    pub fn empty() -> Self {
        Self::new(am_model::Model::new("untitled"))
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.doc
    }

    pub fn motions(&self) -> &MotionPlayer {
        &self.motions
    }

    pub fn physics(&self) -> &PhysicsEngine {
        &self.physics
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// 重新装载动作与物理（模型被整体替换或从磁盘载入后调用）。
    pub fn reload_runtime(&mut self) {
        self.spec.model = self.doc.model().clone();
        self.motions = MotionPlayer::with_motions(self.spec.motions.clone());
        self.physics = PhysicsEngine::new(&self.spec.physics);
        self.animations = self.decode_animations();
        self.mode = ModeState::live();
        let model = self.doc.model().clone();
        self.doc.params_mut().sync_with_model(&model);
    }

    /// 把 `spec.animations` 的 JSON 解码成强类型集合。
    ///
    /// 解码失败的条目**跳过并记日志**而不是整体失败：一份被手工改坏的动画
    /// 不应该让整个工程打不开（与 `read_spec` 对可选文件的宽容策略一致）。
    fn decode_animations(&self) -> am_anim::AnimationSet {
        let mut set = am_anim::AnimationSet::default();
        for value in &self.spec.animations {
            match serde_json::from_value::<am_anim::AmAnimation>(value.clone()) {
                Ok(animation) => set.upsert(animation),
                Err(err) => {
                    let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("<无 id>");
                    log::warn!("跳过无法解析的动画 {id}：{err}");
                }
            }
        }
        set
    }

    /// 把动画集合写回 `spec.animations`（保存 / 导出前调用）。
    fn encode_animations(&mut self) {
        self.spec.animations = self
            .animations
            .animations
            .iter()
            .filter_map(|a| serde_json::to_value(a).ok())
            .collect();
    }

    /// 动画库（只读）。
    pub fn animations(&self) -> &am_anim::AnimationSet {
        &self.animations
    }

    /// 当前模式。
    pub fn mode(&self) -> Mode {
        self.mode.mode
    }

    /// 当前描述层（会先把结构模型同步进镜像）。
    pub fn spec(&mut self) -> &Spec {
        self.sync_spec();
        &self.spec
    }

    /// 直接编辑描述层（**绕过撤销栈**，仅供导入 / 测试使用）。
    pub fn spec_mut(&mut self) -> &mut Spec {
        self.sync_spec();
        &mut self.spec
    }

    fn sync_spec(&mut self) {
        self.spec.model = self.doc.model().clone();
    }

    /// 整体替换描述层（清空撤销历史）。
    pub fn set_spec(&mut self, mut spec: Spec) {
        self.doc = Document::from_model(std::mem::take(&mut spec.model));
        self.spec = spec;
        self.time = 0.0;
        self.frame = 0;
        self.paused = false;
        self.expression = None;
        self.reload_runtime();
    }

    /// 替换模型（清空撤销历史）。
    pub fn set_model(&mut self, model: am_model::Model) {
        self.set_spec(Spec::new(model));
    }

    /// 推进一帧：动作 → 物理 → 时钟。
    pub fn advance(&mut self, dt: f32) {
        if self.paused || !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.motions.update(dt, self.doc.params_mut());
        self.physics.advance(dt, self.doc.params_mut());
        self.time += dt as f64;
        self.frame += 1;
    }

    /// 求值当前场景。
    pub fn evaluate(&self) -> Evaluated {
        evaluate(self.doc.model(), self.doc.params())
    }

    /// 最近一次渲染的像素。
    #[cfg(feature = "gpu")]
    pub fn frame_pixels(&self) -> &[u8] {
        &self.pixels
    }

    #[cfg(feature = "gpu")]
    pub fn renderer(&self) -> Option<&Renderer> {
        self.renderer.as_ref()
    }

    // -------------------------------------------------------------- 分发

    /// 以 JSON 字符串调用一个方法，返回完整信封（FFI 用的就是这一条路径）。
    pub fn dispatch_json(&mut self, method: &str, params_json: &str) -> String {
        let params: Value = if params_json.trim().is_empty() {
            Value::Null
        } else {
            match serde_json::from_str(params_json) {
                Ok(v) => v,
                Err(e) => {
                    let err =
                        ApiError::new(codes::INVALID_REQUEST, format!("参数不是合法 JSON：{e}"));
                    return envelope_err(&err);
                }
            }
        };
        match self.dispatch(method, &params) {
            Ok(result) => serde_json::to_string(&json!({ "ok": true, "result": result }))
                .unwrap_or_else(|e| {
                    format!(
                        r#"{{"ok":false,"error":{{"code":-32603,"message":"序列化失败：{e}"}}}}"#
                    )
                }),
            Err(err) => envelope_err(&err),
        }
    }

    /// 调用一个方法。
    pub fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, ApiError> {
        if let Some(result) = self.dispatch_animation(method, params)? {
            return Ok(result);
        }
        match method {
            // ---------------------------------------------------- system
            "system.ping" => Ok(json!({ "pong": true, "version": ENGINE_VERSION })),
            "system.version" => Ok(json!({
                "name": "anima-engine",
                "version": ENGINE_VERSION,
                "format": am_format::FORMAT_ID,
                "format_version": am_format::SDK_VERSION,
            })),
            "system.capabilities" => Ok(json!({
                "renderer": cfg!(feature = "gpu"),
                "physics": true,
                "motion": true,
                "expressions": true,
                "wasm": cfg!(target_arch = "wasm32"),
                "gpu_backend": self.gpu_backend(),
                "methods": METHODS,
            })),

            // ---------------------------------------------------- doc
            "doc.command" => {
                let command = params
                    .get("command")
                    .ok_or_else(|| ApiError::invalid_params("缺少 command"))?;
                let parsed: Command = serde_json::from_value(command.clone())
                    .map_err(|e| ApiError::invalid_params(format!("命令无法解析：{e}")))?;
                let effects = self.doc.apply(&parsed)?;
                Ok(json!({ "effects": effects, "revision": self.doc.revision() }))
            }
            "doc.undo" => {
                let label = self.doc.undo();
                Ok(json!({ "label": label, "revision": self.doc.revision() }))
            }
            "doc.redo" => {
                let label = self.doc.redo();
                Ok(json!({ "label": label, "revision": self.doc.revision() }))
            }
            "doc.history" => Ok(json!({
                "undo": self.doc.undo_labels(),
                "redo": self.doc.redo_labels(),
                "revision": self.doc.revision(),
                "dirty": self.doc.is_dirty(),
                "can_undo": self.doc.can_undo(),
                "can_redo": self.doc.can_redo(),
            })),
            "doc.model" => Ok(serde_json::to_value(self.doc.model())
                .map_err(|e| ApiError::internal(e.to_string()))?),
            "doc.set_model" => {
                let model = params
                    .get("model")
                    .ok_or_else(|| ApiError::invalid_params("缺少 model"))?;
                let model: am_model::Model = serde_json::from_value(model.clone())
                    .map_err(|e| ApiError::invalid_params(format!("模型无法解析：{e}")))?;
                self.set_model(model);
                Ok(json!({ "ok": true }))
            }
            "doc.evaluate" => {
                let evaluated = self.evaluate();
                Ok(json!({
                    "drawables": evaluated.scene.drawables.len(),
                    "nodes": evaluated.scene.nodes.len(),
                    "canvas": evaluated.scene.canvas,
                }))
            }

            // ---------------------------------------------------- project
            "project.new" => {
                let name = param_str(params, "name")?;
                let width = param_f32(params, "width").unwrap_or(1024.0);
                let height = param_f32(params, "height").unwrap_or(1024.0);
                let mut model = am_model::Model::new(name);
                model.canvas = am_model::Canvas::new(width, height);
                self.set_model(model);
                Ok(json!({ "name": name, "width": width, "height": height }))
            }
            "project.load" => {
                let path = param_str(params, "path")?;
                let project = am_format::Project::open(path)?;
                let spec = project::read_spec(&project)?;
                let (nodes, params_count, motions, expressions) = spec.counts();
                self.set_spec(spec);
                Ok(json!({
                    "path": path,
                    "nodes": nodes,
                    "parameters": params_count,
                    "motions": motions,
                    "expressions": expressions,
                }))
            }
            "project.save" => {
                let path = param_str(params, "path")?;
                self.sync_spec();
                let root = std::path::Path::new(path);
                let project = if root.is_dir() {
                    am_format::Project::open(root)?
                } else {
                    am_format::Project::create(
                        &self.spec.model.name,
                        root,
                        am_format::CreateOptions::default(),
                    )?
                };
                project::write_spec(&project, &self.spec)?;
                self.doc.mark_saved();
                Ok(json!({ "path": path }))
            }
            "project.validate" => {
                let report = self.doc.model().validate();
                Ok(json!({
                    "ok": report.ok(),
                    "issues": report.issues,
                }))
            }
            "project.spec" => {
                self.sync_spec();
                Ok(serde_json::to_value(&self.spec)
                    .map_err(|e| ApiError::internal(e.to_string()))?)
            }
            "project.set_spec" => {
                let value = params
                    .get("spec")
                    .ok_or_else(|| ApiError::invalid_params("缺少 spec"))?;
                let spec: Spec = serde_json::from_value(value.clone())
                    .map_err(|e| ApiError::invalid_params(format!("描述层无法解析：{e}")))?;
                self.set_spec(spec);
                Ok(json!({ "ok": true }))
            }

            // ---------------------------------------------------- runtime
            "runtime.set_param" => {
                let id = param_str(params, "id")?;
                let ok = if let Some(t) = param_f32(params, "normalized") {
                    let model = self.doc.model().clone();
                    self.doc.params_mut().set_normalized(&model, id, t)
                } else {
                    let value = param_f32(params, "value")
                        .ok_or_else(|| ApiError::invalid_params("需要 value 或 normalized"))?;
                    let model = self.doc.model().clone();
                    self.doc.params_mut().set_clamped(&model, id, value)
                };
                if !ok {
                    return Err(ApiError::invalid_params(format!("参数不存在：{id}")));
                }
                Ok(json!({ "id": id, "value": self.doc.params().get(id) }))
            }
            "runtime.params" => Ok(json!(self.doc.params().as_map())),
            "runtime.reset_params" => {
                let model = self.doc.model().clone();
                self.doc.params_mut().reset(&model);
                Ok(json!({ "ok": true }))
            }
            "runtime.advance" => {
                // §2.1：预渲染模式下状态由轨决定，实时推进必须被拒绝（-32010）。
                ModeGuard::require_live(&self.mode, "runtime.advance")?;
                let dt = param_f32(params, "dt").unwrap_or(1.0 / 60.0);
                self.advance(dt);
                Ok(json!({ "time": self.time, "frame": self.frame }))
            }
            "runtime.set_time" => {
                self.time = param_f64(params, "time").unwrap_or(self.time);
                Ok(json!({ "time": self.time }))
            }
            "runtime.pause" => {
                self.paused = true;
                Ok(json!({ "paused": true }))
            }
            "runtime.resume" => {
                self.paused = false;
                Ok(json!({ "paused": false }))
            }
            "runtime.state" => Ok(json!({
                "time": self.time,
                "frame": self.frame,
                "paused": self.paused,
                "params": self.doc.params().as_map(),
                "motion": self.motion_state(),
                "expression": self.expression,
            })),
            "runtime.scene" => {
                let evaluated = self.evaluate();
                Ok(serde_json::to_value(&evaluated.scene)
                    .map_err(|e| ApiError::internal(e.to_string()))?)
            }

            // ---------------------------------------------------- motion
            "motion.list" => Ok(json!(self
                .motions
                .motions()
                .iter()
                .map(|m| json!({
                    "id": m.id,
                    "name": m.name,
                    "duration": m.effective_duration(),
                    "looping": m.looping,
                }))
                .collect::<Vec<_>>())),
            "motion.play" => {
                let id = param_str(params, "id")?;
                let options = PlayOptions {
                    looping: params.get("looping").and_then(|v| v.as_bool()),
                    speed: param_f32(params, "speed"),
                    fade_in: param_f32(params, "fade_in"),
                    from_time: param_f32(params, "from_time"),
                };
                let snapshot = self.doc.params().clone();
                self.motions.play(id, &options, &snapshot)?;
                Ok(self.motion_state())
            }
            "motion.stop" => {
                self.motions.stop();
                Ok(json!({ "playing": false }))
            }
            "motion.pause" => {
                self.motions.pause();
                Ok(json!({ "paused": true }))
            }
            "motion.resume" => {
                self.motions.resume();
                Ok(json!({ "paused": false }))
            }
            "motion.seek" => {
                let time = param_f32(params, "time").unwrap_or(0.0);
                self.motions.seek(time);
                Ok(self.motion_state())
            }
            "motion.state" => Ok(self.motion_state()),

            // ---------------------------------------------------- expression
            "expression.list" => Ok(json!(self
                .spec
                .expressions
                .iter()
                .map(|e| json!({ "id": e.id, "name": e.name, "parameters": e.parameters.len() }))
                .collect::<Vec<_>>())),
            "expression.set" => {
                let id = param_str(params, "id")?;
                let weight = param_f32(params, "weight").unwrap_or(1.0);
                let model = self.doc.model().clone();
                let expression = self
                    .spec
                    .expressions
                    .iter()
                    .find(|e| e.id == id)
                    .cloned()
                    .ok_or_else(|| ApiError::invalid_params(format!("表情不存在：{id}")))?;
                self.doc.params_mut().apply_expression(&model, &expression, weight);
                self.expression = Some(id.to_string());
                Ok(json!({ "id": id, "weight": weight }))
            }

            // ---------------------------------------------------- physics
            "physics.info" => Ok(json!({
                "enabled": self.physics.is_enabled(),
                "settings": self.physics.settings().settings.len(),
                "fps": self.physics.settings().fps,
                "steps": self.physics.steps(),
                "settled": self.physics.is_settled(1e-3),
            })),
            "physics.step" => {
                let dt = param_f32(params, "dt").unwrap_or(1.0 / 60.0);
                let mut store = self.doc.params().clone();
                self.physics.step_fixed(dt, &mut store);
                for (id, value) in store.as_map() {
                    self.doc.params_mut().set(id.clone(), *value);
                }
                Ok(json!({ "steps": self.physics.steps() }))
            }
            "physics.reset" => {
                self.physics.reset();
                Ok(json!({ "ok": true }))
            }

            // ---------------------------------------------------- renderer
            "renderer.info" => Ok(self.renderer_info()),

            "renderer.init" => {
                #[cfg(feature = "gpu")]
                {
                    let width = param_u32(params, "width").unwrap_or(1024).max(1);
                    let height = param_u32(params, "height").unwrap_or(1024).max(1);
                    let renderer = Renderer::with_size(width, height)
                        .map_err(|e| ApiError::unsupported(format!("无法创建渲染器：{e}")))?;
                    self.view = View::new(am_math::Vec2::ZERO, 1.0);
                    self.renderer = Some(renderer);
                    Ok(json!({ "width": width, "height": height }))
                }
                #[cfg(not(feature = "gpu"))]
                {
                    let _ = params;
                    Err(ApiError::unsupported("本构建未启用渲染后端"))
                }
            }

            "renderer.resize" => {
                #[cfg(feature = "gpu")]
                {
                    let width = param_u32(params, "width").unwrap_or(0);
                    let height = param_u32(params, "height").unwrap_or(0);
                    let renderer = self
                        .renderer
                        .as_mut()
                        .ok_or_else(|| ApiError::unsupported("渲染器尚未初始化"))?;
                    renderer
                        .resize(width, height)
                        .map_err(|e| ApiError::invalid_params(format!("尺寸非法：{e}")))?;
                    Ok(json!({ "width": width, "height": height }))
                }
                #[cfg(not(feature = "gpu"))]
                {
                    let _ = params;
                    Err(ApiError::unsupported("本构建未启用渲染后端"))
                }
            }

            "renderer.set_view" => {
                #[cfg(feature = "gpu")]
                {
                    let zoom = param_f32(params, "zoom").unwrap_or(self.view.zoom);
                    let center = match params.get("center").and_then(|v| v.as_array()) {
                        Some(a) if a.len() == 2 => am_math::Vec2::new(
                            a[0].as_f64().unwrap_or(0.0) as f32,
                            a[1].as_f64().unwrap_or(0.0) as f32,
                        ),
                        _ => self.view.center,
                    };
                    self.view.center = center;
                    self.view.set_zoom(zoom);
                    Ok(json!({
                        "center": [self.view.center.x, self.view.center.y],
                        "zoom": self.view.zoom,
                    }))
                }
                #[cfg(not(feature = "gpu"))]
                {
                    let _ = params;
                    Err(ApiError::unsupported("本构建未启用渲染后端"))
                }
            }

            "renderer.set_texture" => {
                #[cfg(feature = "gpu")]
                {
                    let index = param_u32(params, "index").unwrap_or(0);
                    let path = param_str(params, "path")?;
                    let renderer = self
                        .renderer
                        .as_mut()
                        .ok_or_else(|| ApiError::unsupported("渲染器尚未初始化"))?;
                    renderer
                        .load_texture_file(index, path)
                        .map_err(|e| ApiError::invalid_params(format!("纹理载入失败：{e}")))?;
                    Ok(json!({ "index": index, "path": path }))
                }
                #[cfg(not(feature = "gpu"))]
                {
                    let _ = params;
                    Err(ApiError::unsupported("本构建未启用渲染后端"))
                }
            }

            "renderer.clear_textures" => {
                #[cfg(feature = "gpu")]
                {
                    if let Some(renderer) = self.renderer.as_mut() {
                        renderer.clear_textures();
                    }
                }
                Ok(json!({ "ok": true }))
            }

            "renderer.render" => {
                #[cfg(feature = "gpu")]
                {
                    self.upload_model_textures()?;
                    let evaluated = self.evaluate();
                    let renderer = self
                        .renderer
                        .as_mut()
                        .ok_or_else(|| ApiError::unsupported("渲染器尚未初始化"))?;
                    let (w, h) = renderer.size();
                    let view = self.view;
                    let pixels = renderer
                        .render_to_pixels(&evaluated.scene, &view)
                        .map_err(|e| ApiError::internal(format!("渲染失败：{e}")))?;
                    self.pixels = pixels;
                    self.generation += 1;
                    Ok(json!({ "width": w, "height": h, "generation": self.generation }))
                }
                #[cfg(not(feature = "gpu"))]
                {
                    let _ = params;
                    Err(ApiError::unsupported("本构建未启用渲染后端"))
                }
            }

            "renderer.frame" => {
                #[cfg(feature = "gpu")]
                {
                    let (width, height) =
                        self.renderer.as_ref().map(|r| r.size()).unwrap_or((0, 0));
                    Ok(json!({
                        "width": width,
                        "height": height,
                        "bytes": self.pixels.len(),
                        "stride": (width * 4) as usize,
                        "generation": self.generation,
                        "format": "rgba8_unorm",
                    }))
                }
                #[cfg(not(feature = "gpu"))]
                {
                    let _ = params;
                    Err(ApiError::unsupported("本构建未启用渲染后端"))
                }
            }

            "renderer.save_png" => {
                #[cfg(feature = "gpu")]
                {
                    let path = param_str(params, "path")?;
                    let (width, height) = self
                        .renderer
                        .as_ref()
                        .map(|r| r.size())
                        .ok_or_else(|| ApiError::unsupported("渲染器尚未初始化"))?;
                    if self.pixels.len() != (width * height * 4) as usize {
                        return Err(ApiError::invalid_params("尚未渲染任何帧"));
                    }
                    DecodedImage::new(width, height, self.pixels.clone())
                        .and_then(|image| image.save_png(path))
                        .map_err(|e| ApiError::internal(e.to_string()))?;
                    Ok(json!({ "path": path, "width": width, "height": height }))
                }
                #[cfg(not(feature = "gpu"))]
                {
                    let _ = params;
                    Err(ApiError::unsupported("本构建未启用渲染后端"))
                }
            }

            // ---------------------------------------------------- diagnostics
            "diagnostics.stats" => {
                let model = self.doc.model();
                Ok(json!({
                    "nodes": model.nodes.len(),
                    "parameters": model.parameters.len(),
                    "textures": model.textures.len(),
                    "motions": self.motions.motions().len(),
                    "expressions": self.spec.expressions.len(),
                    "physics": self.physics.settings().settings.len(),
                    "drawables": self.evaluate().scene.drawables.len(),
                    "revision": self.doc.revision(),
                    "dirty": self.doc.is_dirty(),
                    "frame": self.frame,
                }))
            }

            other => Err(ApiError::method_not_found(other)),
        }
    }

    // -------------------------------------------------------------- 内部

    fn motion_state(&self) -> Value {
        json!({
            "id": self.motions.current(),
            "time": self.motions.time(),
            "playing": self.motions.is_playing(),
            "paused": self.motions.is_paused(),
            "speed": self.motions.speed(),
            "fade": self.motions.fade_weight(),
        })
    }

    fn renderer_info(&self) -> Value {
        #[cfg(feature = "gpu")]
        {
            match &self.renderer {
                Some(r) => json!({
                    "initialized": true,
                    "width": r.size().0,
                    "height": r.size().1,
                    "format": format!("{:?}", r.format()),
                    "adapter": r.adapter_info(),
                    "textures": r.texture_count(),
                }),                None => json!({ "initialized": false, "supported": true }),
            }
        }
        #[cfg(not(feature = "gpu"))]
        {
            json!({ "initialized": false, "supported": false })
        }
    }

    #[cfg(feature = "gpu")]
    fn gpu_backend(&self) -> Value {
        match &self.renderer {
            Some(r) => Value::String(r.adapter_info().to_string()),
            None => Value::Null,
        }
    }

    #[cfg(not(feature = "gpu"))]
    fn gpu_backend(&self) -> Value {
        Value::Null
    }

    /// 把模型里登记但尚未上传的纹理补上（编辑器新增贴图后无需手动上传）。
    #[cfg(feature = "gpu")]
    fn upload_model_textures(&mut self) -> Result<(), ApiError> {
        let textures = self.doc.model().textures.clone();
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(());
        };
        for (index, texture) in textures.iter().enumerate() {
            if renderer.texture_size(index as u32).is_some() {
                continue;
            }
            if let Ok(image) = decode_file(&texture.asset) {
                let _ = renderer.set_texture(index as u32, &image);
            }
        }
        Ok(())
    }

    /// 直接注入一张纹理（测试与内存素材用）。
    #[cfg(feature = "gpu")]
    pub fn set_texture(&mut self, index: u32, image: &DecodedImage) -> Result<(), ApiError> {
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| ApiError::unsupported("渲染器尚未初始化"))?;
        renderer
            .set_texture(index, image)
            .map_err(|e| ApiError::internal(e.to_string()))
    }
}

fn envelope_err(err: &ApiError) -> String {
    let mut error = json!({ "code": err.code, "message": err.message });
    // 规范 §9：可执行的修复建议随错误一起回给宿主。
    if let Some(hint) = &err.hint {
        error["hint"] = json!(hint);
    }
    serde_json::to_string(&json!({ "ok": false, "error": error })).unwrap_or_else(|_| {
        r#"{"ok":false,"error":{"code":-32603,"message":"内部错误"}}"#.to_string()
    })
}

// ------------------------------------------------------------------ 参数取值助手

fn param_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, ApiError> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::invalid_params(format!("缺少字符串参数 {key}")))
}

fn param_f32(params: &Value, key: &str) -> Option<f32> {
    params.get(key).and_then(|v| v.as_f64()).map(|v| v as f32)
}

fn param_f64(params: &Value, key: &str) -> Option<f64> {
    params.get(key).and_then(|v| v.as_f64())
}

fn param_u32(params: &Value, key: &str) -> Option<u32> {
    params.get(key).and_then(|v| v.as_u64()).map(|v| v as u32)
}
