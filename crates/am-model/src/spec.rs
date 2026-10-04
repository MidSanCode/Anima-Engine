//! 完整描述层：`spec/` 目录在内存中的形态。
//!
//! 引擎运行只需要 [`Spec::model`] 与 [`Spec::physics`]，其余部分（动作、表情、姿势、
//! 设置）供编辑器与运行时按需使用。分开持有是为了让「结构编辑」和「素材/表演数据」
//! 各自演进，而序列化时仍然是一个整体。

use crate::expression::{Expression, ModelSettings, Pose, ProjectConfig};
use crate::model::Model;
use crate::motion::Motion;
use crate::physics::PhysicsSettings;
use serde::{Deserialize, Serialize};

/// `spec/` 描述层。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    /// `spec/model.json`
    #[serde(default)]
    pub model: Model,
    /// `spec/physics.json`
    #[serde(default)]
    pub physics: PhysicsSettings,
    /// `spec/pose.json`
    #[serde(default)]
    pub pose: Pose,
    /// `spec/model.settings.json`
    #[serde(default)]
    pub settings: ModelSettings,
    /// `spec/motions/*.motion.json`
    #[serde(default)]
    pub motions: Vec<Motion>,
    /// `spec/expressions/*.exp.json`
    #[serde(default)]
    pub expressions: Vec<Expression>,
    /// `spec/animations/<id>.anim.json`（预渲染动画，见 `docs/animation-mode.md`）。
    ///
    /// 注意：`am-model` 不认识动画的求值语义，只负责**搬运**这棵 JSON。
    /// 语义在 `am-anim`，这样描述层不必依赖求值链路。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub animations: Vec<serde_json::Value>,
    /// `spec/config.json`（可选）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<ProjectConfig>,
}

impl Spec {
    /// 由模型创建（其余为空）。
    pub fn new(model: Model) -> Self {
        Self { model, ..Default::default() }
    }

    pub fn motion(&self, id: &str) -> Option<&Motion> {
        self.motions.iter().find(|m| m.id == id)
    }

    pub fn motion_mut(&mut self, id: &str) -> Option<&mut Motion> {
        self.motions.iter_mut().find(|m| m.id == id)
    }

    pub fn expression(&self, id: &str) -> Option<&Expression> {
        self.expressions.iter().find(|e| e.id == id)
    }

    /// `(节点, 参数, 动作, 表情)` 数量统计。
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        (
            self.model.nodes.len(),
            self.model.parameters.len(),
            self.motions.len(),
            self.expressions.len(),
        )
    }

    /// 是否只有空模型。
    pub fn is_empty(&self) -> bool {
        self.model.nodes.is_empty()
            && self.model.parameters.is_empty()
            && self.motions.is_empty()
            && self.expressions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Node;

    #[test]
    fn new_spec_is_empty_except_model() {
        let spec = Spec::new(Model::new("demo"));
        assert_eq!(spec.model.name, "demo");
        assert!(spec.is_empty());
        assert_eq!(spec.counts(), (0, 0, 0, 0));
        assert!(spec.motion("nope").is_none());
    }

    #[test]
    fn spec_round_trips_through_json() {
        let mut spec = Spec::new(Model::new("demo"));
        spec.model.add_node(Node::part("Root", None));
        spec.motions.push(Motion::new("idle", "Idle"));
        spec.expressions.push(Expression::new("smile", "Smile"));
        let json = serde_json::to_string(&spec).unwrap();
        let back: Spec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);
    }

    #[test]
    fn partial_json_uses_defaults() {
        let spec: Spec = serde_json::from_str(r#"{"model":{"name":"only"}}"#).unwrap();
        assert_eq!(spec.model.name, "only");
        assert!(spec.motions.is_empty());
        assert!(spec.physics.settings.is_empty());
    }

    #[test]
    fn lookups_find_items() {
        let mut spec = Spec::new(Model::new("demo"));
        spec.motions.push(Motion::new("m1", "M1"));
        spec.expressions.push(Expression::new("e1", "E1"));
        assert!(spec.motion("m1").is_some());
        assert!(spec.motion_mut("m1").is_some());
        assert!(spec.expression("e1").is_some());
        assert!(spec.expression("m1").is_none());
    }
}
