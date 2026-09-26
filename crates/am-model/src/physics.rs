//! 物理设置（`spec/physics.json`）。
//!
//! 数据结构对齐同类软件的物理设定语义：一个物理设定由「输入参数 → 顶点链 → 输出参数」组成，
//! 编辑器只负责编辑这里的数值，仿真逻辑在 `am-physics`。

use crate::id::Id;
use am_math::Vec2;
use serde::{Deserialize, Serialize};

/// 物理设定类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicsKind {
    /// 摆锤式（头发、饰品等整体跟随）。
    #[default]
    Pendulum,
    /// 顶点链式（逐段延迟，形态更柔软）。
    Vertex,
}

/// 参数在物理中扮演的角色类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicsParamType {
    /// 角度（弧度）。
    Angle,
    /// 位置（像素）。
    #[default]
    Position,
}

/// 归一化区间（用于把参数值映射到物理输入）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NormalizationRange {
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

impl Default for NormalizationRange {
    fn default() -> Self {
        Self { min: -1.0, default: 0.0, max: 1.0 }
    }
}

impl NormalizationRange {
    pub fn sanitized(self) -> Self {
        Self {
            min: self.min.min(self.max),
            max: self.min.max(self.max),
            default: self.default,
        }
    }

    /// 把参数值映射到 `[-1, 1]`（默认值处为 0）。
    pub fn normalize(&self, v: f32) -> f32 {
        let r = self.sanitized();
        if v >= r.default {
            let span = r.max - r.default;
            if span.abs() <= f32::EPSILON {
                0.0
            } else {
                ((v - r.default) / span).clamp(0.0, 1.0)
            }
        } else {
            let span = r.default - r.min;
            if span.abs() <= f32::EPSILON {
                0.0
            } else {
                ((v - r.default) / span).clamp(-1.0, 0.0)
            }
        }
    }
}

/// 物理输入：某个参数驱动物理。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsInput {
    pub parameter: Id,
    #[serde(default = "one")]
    pub weight: f32,
    #[serde(default)]
    pub inverted: bool,
    #[serde(default)]
    pub param_type: PhysicsParamType,
    #[serde(default)]
    pub normalization: NormalizationRange,
}

impl PhysicsInput {
    pub fn new(parameter: impl Into<Id>) -> Self {
        Self {
            parameter: parameter.into(),
            weight: 1.0,
            inverted: false,
            param_type: PhysicsParamType::default(),
            normalization: NormalizationRange::default(),
        }
    }
}

/// 物理输出：物理结果写回某个参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsOutput {
    pub parameter: Id,
    #[serde(default = "one")]
    pub weight: f32,
    #[serde(default)]
    pub inverted: bool,
    #[serde(default)]
    pub param_type: PhysicsParamType,
    /// 输出缩放（角度输出常用）。
    #[serde(default = "one")]
    pub scale: f32,
    /// 小于最小值后是否反向（防止穿模的常用手段）。
    #[serde(default)]
    pub reflect: bool,
    #[serde(default)]
    pub normalization: NormalizationRange,
}

impl PhysicsOutput {
    pub fn new(parameter: impl Into<Id>) -> Self {
        Self {
            parameter: parameter.into(),
            weight: 1.0,
            inverted: false,
            param_type: PhysicsParamType::default(),
            scale: 1.0,
            reflect: false,
            normalization: NormalizationRange::default(),
        }
    }
}

/// 顶点物理的一段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsVertex {
    /// 相对上一个顶点的位置（第一段相对输入原点）。
    pub position: Vec2,
    /// 跟随输入的强度（0 = 完全不跟随）。
    #[serde(default = "one")]
    pub mobility: f32,
    /// 延迟（秒）。
    #[serde(default)]
    pub delay: f32,
    /// 加速度（越大越迟钝）。
    #[serde(default = "one")]
    pub acceleration: f32,
    /// 影响半径。
    #[serde(default)]
    pub radius: f32,
    #[serde(default = "one")]
    pub weight: f32,
}

impl Default for PhysicsVertex {
    fn default() -> Self {
        Self {
            position: Vec2::new(0.0, -10.0),
            mobility: 1.0,
            delay: 0.0,
            acceleration: 1.0,
            radius: 0.0,
            weight: 1.0,
        }
    }
}

/// 一组物理设定。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsSetting {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: PhysicsKind,
    #[serde(default)]
    pub inputs: Vec<PhysicsInput>,
    #[serde(default)]
    pub outputs: Vec<PhysicsOutput>,
    /// `kind == Vertex` 时的顶点链。
    #[serde(default)]
    pub vertices: Vec<PhysicsVertex>,
    /// 位置归一化（对应顶点链的输入映射）。
    #[serde(default)]
    pub position_normalization: NormalizationRange,
    /// 角度归一化。
    #[serde(default)]
    pub angle_normalization: NormalizationRange,
    /// 每段整体延迟（秒）。
    #[serde(default)]
    pub delay: f32,
    /// 输出是否随重力方向摆动。
    #[serde(default = "one")]
    pub gravity: f32,
    #[serde(default)]
    pub enabled: bool,
}

impl Default for PhysicsSetting {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: PhysicsKind::Pendulum,
            inputs: Vec::new(),
            outputs: Vec::new(),
            vertices: Vec::new(),
            position_normalization: NormalizationRange::default(),
            angle_normalization: NormalizationRange { min: -0.9, default: 0.0, max: 0.9 },
            delay: 0.0,
            gravity: 1.0,
            enabled: true,
        }
    }
}

impl PhysicsSetting {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self { id: id.into(), name: name.into(), ..Default::default() }
    }
}

/// `spec/physics.json` 的根。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 物理步进频率（Hz）。固定步长保证编辑器/查看器/浏览器结果一致。
    #[serde(default = "default_fps")]
    pub fps: f32,
    /// 重力（像素/秒²），用于顶点链静止形态。
    #[serde(default)]
    pub gravity: Vec2,
    /// 风速（像素/秒²）。
    #[serde(default)]
    pub wind: Vec2,
    #[serde(default)]
    pub settings: Vec<PhysicsSetting>,
}

fn one() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_fps() -> f32 {
    60.0
}

impl Default for PhysicsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            fps: 60.0,
            gravity: Vec2::new(0.0, -9.8),
            wind: Vec2::ZERO,
            settings: Vec::new(),
        }
    }
}

impl PhysicsSettings {
    /// 固定步长（秒）。非法 `fps` 回退到 60。
    pub fn fixed_dt(&self) -> f32 {
        let fps = if self.fps.is_finite() && self.fps > 1.0 { self.fps } else { 60.0 };
        1.0 / fps
    }

    pub fn setting(&self, id: &str) -> Option<&PhysicsSetting> {
        self.settings.iter().find(|s| s.id == id)
    }

    pub fn setting_mut(&mut self, id: &str) -> Option<&mut PhysicsSetting> {
        self.settings.iter_mut().find(|s| s.id == id)
    }

    /// 所有被物理写出的参数 id（去重）。
    pub fn output_parameters(&self) -> Vec<Id> {
        let mut out: Vec<Id> = Vec::new();
        for s in &self.settings {
            for o in &s.outputs {
                if !out.contains(&o.parameter) {
                    out.push(o.parameter.clone());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_maps_default_to_zero() {
        let r = NormalizationRange { min: -30.0, default: 0.0, max: 30.0 };
        assert_eq!(r.normalize(0.0), 0.0);
        assert_eq!(r.normalize(30.0), 1.0);
        assert_eq!(r.normalize(-30.0), -1.0);
        assert_eq!(r.normalize(300.0), 1.0);
    }

    #[test]
    fn normalization_handles_degenerate_spans() {
        let r = NormalizationRange { min: 5.0, default: 5.0, max: 5.0 };
        assert_eq!(r.normalize(5.0), 0.0);
        assert!(r.normalize(9.0).is_finite());
    }

    #[test]
    fn fixed_dt_falls_back_for_invalid_fps() {
        let mut p = PhysicsSettings::default();
        p.fps = 0.0;
        assert!((p.fixed_dt() - 1.0 / 60.0).abs() < 1e-6);
        p.fps = 120.0;
        assert!((p.fixed_dt() - 1.0 / 120.0).abs() < 1e-6);
    }

    #[test]
    fn output_parameters_are_unique() {
        let mut p = PhysicsSettings::default();
        let mut s1 = PhysicsSetting::new("s1", "hair");
        s1.outputs = vec![PhysicsOutput::new("p1"), PhysicsOutput::new("p2")];
        let mut s2 = PhysicsSetting::new("s2", "skirt");
        s2.outputs = vec![PhysicsOutput::new("p2")];
        p.settings = vec![s1, s2];
        assert_eq!(p.output_parameters(), vec!["p1", "p2"]);
    }
}
