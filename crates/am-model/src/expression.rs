//! 表情、姿势与模型设置（`spec/expressions/*.exp.json`、`spec/pose.json`、`spec/model.settings.json`）。

use crate::id::Id;
use crate::model::BlendType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn default_one() -> f32 {
    1.0
}

// ---------------------------------------------------------------- 表情

/// 表情中的一个参数预设。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpressionParam {
    pub parameter: Id,
    pub value: f32,
    #[serde(default)]
    pub blend: BlendType,
    #[serde(default = "default_one")]
    pub weight: f32,
}

impl ExpressionParam {
    pub fn new(parameter: impl Into<Id>, value: f32) -> Self {
        Self { parameter: parameter.into(), value, blend: BlendType::Normal, weight: 1.0 }
    }
}

/// 一个表情。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expression {
    pub id: Id,
    pub name: String,
    #[serde(default)]
    pub fade_in: f32,
    #[serde(default)]
    pub fade_out: f32,
    #[serde(default)]
    pub parameters: Vec<ExpressionParam>,
}

impl Expression {
    pub fn new(id: impl Into<Id>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            fade_in: 0.2,
            fade_out: 0.2,
            parameters: Vec::new(),
        }
    }

    pub fn set(&mut self, parameter: &str, value: f32) {
        match self.parameters.iter_mut().find(|p| p.parameter == parameter) {
            Some(p) => p.value = value,
            None => self.parameters.push(ExpressionParam::new(parameter, value)),
        }
    }

    pub fn get(&self, parameter: &str) -> Option<f32> {
        self.parameters.iter().find(|p| p.parameter == parameter).map(|p| p.value)
    }
}

// ---------------------------------------------------------------- 姿势

/// 姿势组内的一个部件可见性设置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PosePart {
    pub node: Id,
    #[serde(default = "default_true")]
    pub visible: bool,
}

/// 互斥的姿势组：同一组内只有一个条目可生效。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoseGroup {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub parts: Vec<PosePart>,
}

impl PoseGroup {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self { id: id.into(), name: name.into(), parts: Vec::new() }
    }

    /// 生成让 `active` 生效、其余条目隐藏的可见性映射。
    pub fn resolve(&self, active: usize) -> Vec<(Id, bool)> {
        self.parts
            .iter()
            .enumerate()
            .map(|(i, p)| (p.node.clone(), i == active && p.visible))
            .collect()
    }
}

/// `spec/pose.json` 的根。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    #[serde(default)]
    pub groups: Vec<PoseGroup>,
}

// ---------------------------------------------------------------- 自动效果

/// 自动效果（眨眼 / 呼吸）的通用配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoEffect {
    #[serde(default)]
    pub enabled: bool,
    /// 被驱动的参数 id 列表。
    #[serde(default)]
    pub parameters: Vec<Id>,
    /// 触发间隔（秒）。
    #[serde(default = "default_interval")]
    pub interval: f32,
    /// 单次动作时长（秒）。
    #[serde(default)]
    pub duration: f32,
    /// 取值下限 / 上限。
    #[serde(default)]
    pub min: f32,
    #[serde(default = "default_one")]
    pub max: f32,
    /// 随机抖动比例（0 = 完全规律）。
    #[serde(default)]
    pub jitter: f32,
}

fn default_interval() -> f32 {
    4.0
}

impl Default for AutoEffect {
    fn default() -> Self {
        Self {
            enabled: false,
            parameters: Vec::new(),
            interval: 4.0,
            duration: 0.15,
            min: 0.0,
            max: 1.0,
            jitter: 0.5,
        }
    }
}

/// 口型同步配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LipSync {
    #[serde(default)]
    pub enabled: bool,
    /// 张口量参数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_parameter: Option<Id>,
    /// 口形参数（元音变化）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_parameter: Option<Id>,
    /// 音量放大倍数。
    #[serde(default = "default_one")]
    pub amplify: f32,
    /// 平滑时间（秒）。
    #[serde(default)]
    pub smoothing: f32,
}

impl Default for LipSync {
    fn default() -> Self {
        Self { enabled: false, open_parameter: None, form_parameter: None, amplify: 1.0, smoothing: 0.05 }
    }
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------- 模型设置

/// `spec/model.settings.json`：模型对外暴露的默认行为。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// 默认动作（`spec/motions/` 下的文件名，不含扩展名）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_motion: Option<String>,
    /// 默认表情。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_expression: Option<String>,
    /// 是否启用物理。
    #[serde(default = "default_true")]
    pub physics: bool,
    #[serde(default)]
    pub auto_blink: AutoEffect,
    #[serde(default)]
    pub auto_breath: AutoEffect,
    #[serde(default)]
    pub lip_sync: LipSync,
    /// 参数默认值覆盖。
    #[serde(default)]
    pub parameter_defaults: BTreeMap<Id, f32>,
    /// 动作分组：分组名 → 动作名列表。
    #[serde(default)]
    pub motion_groups: BTreeMap<String, Vec<String>>,
    /// 表情分组：分组名 → 表情名列表。
    #[serde(default)]
    pub expression_groups: BTreeMap<String, Vec<String>>,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            display_name: None,
            default_motion: None,
            default_expression: None,
            physics: true,
            auto_blink: AutoEffect { enabled: false, ..Default::default() },
            auto_breath: AutoEffect {
                enabled: false,
                interval: 0.0,
                duration: 3.0,
                min: -1.0,
                max: 1.0,
                jitter: 0.0,
                ..Default::default()
            },
            lip_sync: LipSync::default(),
            parameter_defaults: BTreeMap::new(),
            motion_groups: BTreeMap::new(),
            expression_groups: BTreeMap::new(),
        }
    }
}

// ---------------------------------------------------------------- 工程配置

/// `spec/config.json`：工程级运行配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectConfig {
    #[serde(default)]
    pub project: ProjectInfoSection,
    #[serde(default)]
    pub engine: EngineSection,
    #[serde(default)]
    pub settings: SettingsSection,
    /// 未识别的字段原样保留。
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfoSection {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EngineSection {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsSection {
    #[serde(default = "default_quality")]
    pub quality: String,
    /// 画布宽度（像素）。
    #[serde(default)]
    pub canvas_width: f32,
    /// 画布高度（像素）。
    #[serde(default)]
    pub canvas_height: f32,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_one")]
    pub volume: f32,
}

fn default_quality() -> String {
    "high".to_string()
}

fn default_language() -> String {
    "zh-CN".to_string()
}

impl Default for SettingsSection {
    fn default() -> Self {
        Self {
            quality: default_quality(),
            canvas_width: 0.0,
            canvas_height: 0.0,
            language: default_language(),
            volume: 1.0,
        }
    }
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            project: ProjectInfoSection::default(),
            engine: EngineSection { name: "anima".into(), version: env!("CARGO_PKG_VERSION").into() },
            settings: SettingsSection::default(),
            extra: serde_json::Map::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expression_set_get() {
        let mut e = Expression::new("e1", "笑");
        e.set("ParamAngleX", 10.0);
        e.set("ParamAngleX", 20.0);
        assert_eq!(e.parameters.len(), 1);
        assert_eq!(e.get("ParamAngleX"), Some(20.0));
        assert_eq!(e.get("missing"), None);
    }

    #[test]
    fn pose_group_resolves_single_active_part() {
        let mut g = PoseGroup::new("g1", "手臂");
        g.parts = vec![
            PosePart { node: "arm_a".into(), visible: true },
            PosePart { node: "arm_b".into(), visible: true },
        ];
        let v = g.resolve(1);
        assert_eq!(v, vec![("arm_a".to_string(), false), ("arm_b".to_string(), true)]);
    }

    #[test]
    fn pose_round_trips_json() {
        let pose = Pose { groups: vec![PoseGroup::new("g", "n")] };
        let s = serde_json::to_string(&pose).unwrap();
        let back: Pose = serde_json::from_str(&s).unwrap();
        assert_eq!(back, pose);
    }

    #[test]
    fn model_settings_defaults_are_sane() {
        let s = ModelSettings::default();
        assert!(s.physics);
        assert!(!s.auto_blink.enabled);
        assert!(!s.lip_sync.enabled);
        // 空对象也能反序列化（字段全部有默认值）
        let parsed: ModelSettings = serde_json::from_str("{}").unwrap();
        assert!(parsed.physics);
    }

    #[test]
    fn project_config_keeps_unknown_sections() {
        let text = r#"{"project":{"name":"demo"},"engine":{"name":"anima","version":"0.1.0"},
            "settings":{"quality":"high","language":"zh-CN"},"flags":{"x":1}}"#;
        let cfg: ProjectConfig = serde_json::from_str(text).unwrap();
        assert_eq!(cfg.project.name, "demo");
        assert!(cfg.extra.contains_key("flags"));
    }

    #[test]
    fn auto_effect_min_max_defaults() {
        let e: AutoEffect = serde_json::from_str("{\"enabled\":true}").unwrap();
        assert!(e.enabled);
        assert_eq!((e.min, e.max), (0.0, 1.0));
        assert_eq!(e.interval, 4.0);
    }
}
