//! `am-model`：Anima 的文档数据模型。
//!
//! 这里定义 `spec/` 描述层中**全部可序列化的结构**：
//!
//! | 文件 | 类型 |
//! | --- | --- |
//! | `spec/model.json` | [`Model`]（部件树 / 变形器 / 绘制对象 / 参数 / 关键形） |
//! | `spec/physics.json` | [`PhysicsSettings`] |
//! | `spec/pose.json` | [`Pose`] |
//! | `spec/model.settings.json` | [`ModelSettings`] |
//! | `spec/motions/*.motion.json` | [`Motion`] |
//! | `spec/expressions/*.exp.json` | [`Expression`] |
//! | `spec/config.json` | [`ProjectConfig`] |
//!
//! 所有类型都是纯数据（零副作用），求值与仿真逻辑在 `am-eval` / `am-physics` / `am-motion`。

pub mod expression;
pub mod id;
pub mod model;
pub mod motion;
pub mod param;
pub mod physics;
pub mod spec;
pub mod validate;

pub use expression::{
    AutoEffect, Expression, ExpressionParam, LipSync, ModelSettings, Pose, PoseGroup, PosePart,
    ProjectConfig, SettingsSection,
};
pub use id::{is_valid_id, new_id, Id};
pub use model::{
    BlendType, Canvas, DrawableData, Keyform, Mesh, Model, ModelStats, Node, NodeKind, RotationData,
    RotationValue, TextureRef, WarpData, MODEL_FORMAT_VERSION,
};
pub use motion::{Motion, MotionCurve, MotionKey, MotionTarget};
pub use param::{Parameter, ParameterGroup};
pub use physics::{
    NormalizationRange, PhysicsInput, PhysicsKind, PhysicsOutput, PhysicsParamType, PhysicsSetting,
    PhysicsSettings, PhysicsVertex,
};
pub use spec::Spec;
pub use validate::{ModelIssue, ModelReport};

/// 描述层文件名。
pub const SPEC_MODEL_FILE: &str = "model.json";
pub const SPEC_PHYSICS_FILE: &str = "physics.json";
pub const SPEC_POSE_FILE: &str = "pose.json";
pub const SPEC_SETTINGS_FILE: &str = "model.settings.json";
pub const SPEC_CONFIG_FILE: &str = "config.json";

/// 描述层子目录。
pub const MOTIONS_DIR: &str = "motions";
pub const EXPRESSIONS_DIR: &str = "expressions";
/// 预渲染动画目录（`spec/animations/<id>.anim.json`）。
pub const ANIMATIONS_DIR: &str = "animations";
/// 动画文件扩展名。
pub const ANIMATION_EXT: &str = ".anim.json";
