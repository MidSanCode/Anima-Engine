//! 动作数据（`spec/motions/<name>.motion.json`）与曲线求值。
//!
//! 这里只做「数据 + 纯函数采样」；播放状态机、淡入淡出、循环与混合在 `am-motion`。

use crate::id::Id;
use am_math::{Easing, Vec2};
use serde::{Deserialize, Serialize};

fn default_fps() -> f32 {
    60.0
}

/// 曲线的驱动目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionTarget {
    /// 模型参数。
    #[default]
    Parameter,
    /// 某个部件的不透明度。
    PartOpacity,
}

/// 单个关键帧。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MotionKey {
    /// 时间（秒）。
    pub time: f32,
    pub value: f32,
    /// 从本关键帧到下一关键帧的插值方式。
    #[serde(default)]
    pub easing: Easing,
}

impl MotionKey {
    pub fn new(time: f32, value: f32) -> Self {
        Self { time, value, easing: Easing::Linear }
    }

    pub fn with_easing(time: f32, value: f32, easing: Easing) -> Self {
        Self { time, value, easing }
    }
}

/// 一条参数曲线。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotionCurve {
    /// 目标参数 id（`target == PartOpacity` 时是节点 id）。
    pub target: Id,
    #[serde(default)]
    pub kind: MotionTarget,
    /// 关键帧，按 `time` 升序。
    #[serde(default)]
    pub keys: Vec<MotionKey>,
}

impl MotionCurve {
    pub fn new(target: impl Into<Id>) -> Self {
        Self { target: target.into(), kind: MotionTarget::Parameter, keys: Vec::new() }
    }

    /// 插入关键帧并保持时间升序（同时间点覆盖）。
    pub fn insert_key(&mut self, key: MotionKey) {
        match self.keys.binary_search_by(|k| {
            k.time.partial_cmp(&key.time).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Ok(idx) => self.keys[idx] = key,
            Err(idx) => self.keys.insert(idx, key),
        }
    }

    pub fn remove_key_at(&mut self, time: f32, tolerance: f32) -> bool {
        if let Some(idx) = self.keys.iter().position(|k| (k.time - time).abs() <= tolerance) {
            self.keys.remove(idx);
            true
        } else {
            false
        }
    }

    pub fn duration(&self) -> f32 {
        self.keys.last().map(|k| k.time).unwrap_or(0.0)
    }

    /// 采样：区间内按关键帧的缓动插值；越界时取端点值（`Step` 保持前值）。
    pub fn sample(&self, time: f32) -> Option<f32> {
        if self.keys.is_empty() {
            return None;
        }
        if time <= self.keys[0].time {
            return Some(self.keys[0].value);
        }
        let last = self.keys.last()?;
        if time >= last.time {
            return Some(last.value);
        }
        // 找到 `time` 所在区间的左端点
        let idx = match self
            .keys
            .binary_search_by(|k| k.time.partial_cmp(&time).unwrap_or(std::cmp::Ordering::Equal))
        {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let a = &self.keys[idx];
        let b = &self.keys[idx + 1];
        let span = b.time - a.time;
        if span.abs() <= f32::EPSILON {
            return Some(b.value);
        }
        let t = ((time - a.time) / span).clamp(0.0, 1.0);
        let eased = a.easing.eval(t);
        Some(a.value + (b.value - a.value) * eased)
    }
}

/// 一个动作。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    pub id: Id,
    pub name: String,
    /// 时长（秒）。为 0 时按最长曲线推断。
    #[serde(default)]
    pub duration: f32,
    #[serde(default, rename = "loop")]
    pub looping: bool,
    #[serde(default = "default_fps")]
    pub fps: f32,
    #[serde(default)]
    pub fade_in: f32,
    #[serde(default)]
    pub fade_out: f32,
    #[serde(default)]
    pub curves: Vec<MotionCurve>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// 曲线编辑器中每条曲线的显示颜色（`#RRGGBB`），仅编辑器使用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<std::collections::BTreeMap<Id, String>>,
}

impl Motion {
    pub fn new(id: impl Into<Id>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            duration: 0.0,
            looping: false,
            fps: 60.0,
            fade_in: 0.0,
            fade_out: 0.0,
            curves: Vec::new(),
            comment: None,
            colors: None,
        }
    }

    /// 实际时长（`duration` 为 0 时取曲线最大时间）。
    pub fn effective_duration(&self) -> f32 {
        if self.duration > 0.0 {
            self.duration
        } else {
            self.curves.iter().map(MotionCurve::duration).fold(0.0, f32::max)
        }
    }

    pub fn curve(&self, target: &str) -> Option<&MotionCurve> {
        self.curves.iter().find(|c| c.target == target)
    }

    pub fn curve_mut(&mut self, target: &str) -> Option<&mut MotionCurve> {
        self.curves.iter_mut().find(|c| c.target == target)
    }

    /// 在 `time` 采样全部曲线；返回 `(参数 id, 值)` 列表。
    pub fn sample(&self, time: f32) -> Vec<(Id, f32)> {
        self.curves
            .iter()
            .filter_map(|c| c.sample(time).map(|v| (c.target.clone(), v)))
            .collect()
    }

    /// 归一化时间（按循环/结束钳制）。
    pub fn local_time(&self, time: f32) -> f32 {
        let dur = self.effective_duration();
        if dur <= f32::EPSILON {
            return 0.0;
        }
        if self.looping {
            let t = time % dur;
            if t < 0.0 {
                t + dur
            } else {
                t
            }
        } else {
            time.clamp(0.0, dur)
        }
    }

    /// 采样并把循环时间归一化。
    pub fn sample_looped(&self, time: f32) -> Vec<(Id, f32)> {
        self.sample(self.local_time(time))
    }

    pub fn is_finished(&self, time: f32) -> bool {
        !self.looping && time >= self.effective_duration()
    }

    /// 供 UI 展示的曲线包围盒（时间/值范围）。
    pub fn bounds(&self) -> Option<(Vec2, Vec2)> {
        let mut min = Vec2::new(f32::MAX, f32::MAX);
        let mut max = Vec2::new(f32::MIN, f32::MIN);
        let mut any = false;
        for c in &self.curves {
            for k in &c.keys {
                min = min.min(Vec2::new(k.time, k.value));
                max = max.max(Vec2::new(k.time, k.value));
                any = true;
            }
        }
        any.then_some((min, max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve_with(keys: &[(f32, f32, Easing)]) -> MotionCurve {
        let mut c = MotionCurve::new("p1");
        for (t, v, e) in keys {
            c.insert_key(MotionKey::with_easing(*t, *v, *e));
        }
        c
    }

    #[test]
    fn linear_sampling_interpolates() {
        let c = curve_with(&[(0.0, 0.0, Easing::Linear), (1.0, 10.0, Easing::Linear)]);
        assert_eq!(c.sample(0.5), Some(5.0));
        assert_eq!(c.sample(-1.0), Some(0.0));
        assert_eq!(c.sample(99.0), Some(10.0));
    }

    #[test]
    fn step_holds_previous_value_until_next_key() {
        let c = curve_with(&[(0.0, 0.0, Easing::Step), (1.0, 10.0, Easing::Linear)]);
        assert_eq!(c.sample(0.99), Some(0.0));
        assert_eq!(c.sample(1.0), Some(10.0));
    }

    #[test]
    fn insert_key_keeps_time_order_and_overwrites_duplicates() {
        let mut c = MotionCurve::new("p1");
        c.insert_key(MotionKey::new(2.0, 20.0));
        c.insert_key(MotionKey::new(1.0, 10.0));
        c.insert_key(MotionKey::new(1.5, 15.0));
        c.insert_key(MotionKey::new(1.5, 99.0));
        let times: Vec<f32> = c.keys.iter().map(|k| k.time).collect();
        assert_eq!(times, vec![1.0, 1.5, 2.0]);
        assert_eq!(c.keys[1].value, 99.0);
    }

    #[test]
    fn empty_curve_samples_to_none() {
        let c = MotionCurve::new("p1");
        assert_eq!(c.sample(0.5), None);
        assert_eq!(c.duration(), 0.0);
    }

    #[test]
    fn duplicate_times_do_not_divide_by_zero() {
        let mut c = MotionCurve::new("p1");
        c.keys = vec![MotionKey::new(1.0, 1.0), MotionKey::new(1.0, 2.0)];
        assert_eq!(c.sample(1.0), Some(1.0));
    }

    #[test]
    fn loop_and_finish_semantics() {
        let mut m = Motion::new("m1", "walk");
        m.curves = vec![curve_with(&[(0.0, 0.0, Easing::Linear), (2.0, 1.0, Easing::Linear)])];
        assert_eq!(m.effective_duration(), 2.0);
        assert!(!m.looping);
        assert!(m.is_finished(2.0));
        assert_eq!(m.local_time(5.0), 2.0);

        m.looping = true;
        assert!(!m.is_finished(5.0));
        assert!((m.local_time(2.5) - 0.5).abs() < 1e-5);
        assert!((m.local_time(-0.5) - 1.5).abs() < 1e-5);
    }

    #[test]
    fn sample_looped_returns_all_curves() {
        let mut m = Motion::new("m1", "idle");
        m.looping = true;
        m.curves = vec![
            curve_with(&[(0.0, 0.0, Easing::Linear), (1.0, 1.0, Easing::Linear)]),
            curve_with(&[(0.0, 5.0, Easing::Linear), (1.0, 6.0, Easing::Linear)]),
        ];
        // 第二条曲线目标 id 与第一条相同，这里只验证数量与采样长度
        let out = m.sample_looped(1.5);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].1, 0.5);
    }

    #[test]
    fn bounds_cover_all_keys() {
        let mut m = Motion::new("m1", "x");
        m.curves = vec![curve_with(&[(0.0, -3.0, Easing::Linear), (2.0, 7.0, Easing::Linear)])];
        let (min, max) = m.bounds().unwrap();
        assert_eq!(min, Vec2::new(0.0, -3.0));
        assert_eq!(max, Vec2::new(2.0, 7.0));
        assert!(Motion::new("m", "empty").bounds().is_none());
    }
}
