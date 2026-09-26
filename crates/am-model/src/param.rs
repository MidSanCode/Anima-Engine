//! 参数（Parameter）与参数分组。

use crate::id::Id;
use serde::{Deserialize, Serialize};

fn default_one() -> f32 {
    1.0
}

/// 参数分组（用于编辑器面板折叠显示）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterGroup {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub parameters: Vec<Id>,
}

impl ParameterGroup {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self { id: id.into(), name: name.into(), parameters: Vec::new() }
    }
}

/// 一个可被驱动的参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub id: Id,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// 关键形所在的关键点（升序、去重）。为空时按 `[min, default, max]` 处理。
    #[serde(default)]
    pub keys: Vec<f32>,
    /// 是否作为混合形状参数（不参与固定关键点约束）。
    #[serde(default)]
    pub is_blend_shape: bool,
    /// 参数是否循环（超出范围时回绕而非钳制）。
    #[serde(default)]
    pub repeat: bool,
    /// 由自动效果驱动（眨眼/呼吸/口型），录制动作时默认跳过。
    #[serde(default)]
    pub auto: bool,
    /// 默认权重（多参数叠加时的缩放）。
    #[serde(default = "default_one")]
    pub weight: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

impl Parameter {
    pub fn new(id: impl Into<Id>, name: impl Into<String>, min: f32, max: f32, default: f32) -> Self {
        let lo = min.min(max);
        let hi = min.max(max);
        let default = if default.is_finite() { default.clamp(lo, hi) } else { lo };
        Self {
            id: id.into(),
            name: name.into(),
            group: None,
            min: lo,
            max: hi,
            default,
            keys: vec![min, default, max],
            is_blend_shape: false,
            repeat: false,
            auto: false,
            weight: 1.0,
            comment: None,
        }
    }

    /// 把取值限制到参数范围内（`repeat` 时回绕）。
    pub fn clamp_value(&self, v: f32) -> f32 {
        if !v.is_finite() {
            return self.default;
        }
        if self.repeat {
            let span = self.max - self.min;
            if span <= f32::EPSILON {
                return self.min;
            }
            let t = (v - self.min) % span;
            self.min + if t < 0.0 { t + span } else { t }
        } else {
            v.clamp(self.min, self.max)
        }
    }

    /// 归一化到 `[0, 1]`。
    pub fn normalize(&self, v: f32) -> f32 {
        let span = self.max - self.min;
        if span.abs() <= f32::EPSILON {
            0.0
        } else {
            ((v - self.min) / span).clamp(0.0, 1.0)
        }
    }

    /// 关键点列表（保证至少包含 `min` / `default` / `max`，升序去重）。
    ///
    /// 默认值一定是关键点：这样"恢复默认"总能被关键形精确表示。
    pub fn effective_keys(&self) -> Vec<f32> {
        let mut keys: Vec<f32> = if self.keys.is_empty() {
            vec![self.min, self.default, self.max]
        } else {
            self.keys.clone()
        };
        keys.push(self.min);
        keys.push(self.max);
        keys.push(self.default);
        keys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        keys.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        keys
    }

    /// 把关键点序列化为 Live2D 风格的 `{min, default, max}` 三元组形式（供 UI 展示）。
    pub fn key_span(&self) -> (f32, f32, f32) {
        (self.min, self.default, self.max)
    }

    /// 参数是否被修改过（相对默认值）。
    pub fn is_default(&self, value: f32) -> bool {
        (value - self.default).abs() <= 1e-6
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_to_range_by_default() {
        let p = Parameter::new("p1", "ParamAngleX", -30.0, 30.0, 0.0);
        assert_eq!(p.clamp_value(100.0), 30.0);
        assert_eq!(p.clamp_value(-100.0), -30.0);
        assert_eq!(p.clamp_value(12.0), 12.0);
    }

    #[test]
    fn repeat_wraps_around_range() {
        let mut p = Parameter::new("p1", "Spin", 0.0, 360.0, 0.0);
        p.repeat = true;
        assert!((p.clamp_value(370.0) - 10.0).abs() < 1e-4);
        assert!((p.clamp_value(-10.0) - 350.0).abs() < 1e-4);
    }

    #[test]
    fn non_finite_falls_back_to_default() {
        let p = Parameter::new("p1", "X", -1.0, 1.0, 0.25);
        assert_eq!(p.clamp_value(f32::NAN), 0.25);
    }

    #[test]
    fn constructor_normalizes_inverted_bounds_and_default() {
        let p = Parameter::new("p1", "X", 10.0, -10.0, 100.0);
        assert_eq!((p.min, p.max, p.default), (-10.0, 10.0, 10.0));
    }

    #[test]
    fn effective_keys_always_include_min_and_max() {
        let mut p = Parameter::new("p1", "X", -1.0, 1.0, 0.0);
        p.keys = vec![0.5, 0.5];
        assert_eq!(p.effective_keys(), vec![-1.0, 0.0, 0.5, 1.0]);
    }

    #[test]
    fn normalize_maps_range_to_unit() {
        let p = Parameter::new("p1", "X", -10.0, 10.0, 0.0);
        assert_eq!(p.normalize(0.0), 0.5);
        assert_eq!(p.normalize(-100.0), 0.0);
        assert_eq!(p.normalize(100.0), 1.0);
    }

    #[test]
    fn zero_span_parameter_does_not_divide_by_zero() {
        let p = Parameter::new("p1", "X", 1.0, 1.0, 1.0);
        assert_eq!(p.normalize(5.0), 0.0);
        assert!(p.clamp_value(5.0).is_finite());
    }
}
