//! 曲线插值：缓动类型与三次贝塞尔段。
//!
//! 动作（`spec/motions/*.motion.json`）与关键形插值共用这里的实现，
//! 保证编辑器预览、查看器播放与运行时求值给出完全相同的数值。

use crate::{saturate, Vec2};
use serde::{Deserialize, Serialize};

/// 三次贝塞尔段（4 个控制点）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BezierSegment {
    pub p0: Vec2,
    pub p1: Vec2,
    pub p2: Vec2,
    pub p3: Vec2,
}

impl BezierSegment {
    pub const fn new(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2) -> Self {
        Self { p0, p1, p2, p3 }
    }

    /// 单位域上的贝塞尔曲线（用于缓动）：`(0,0) → (1,1)`，控制点为 `p1`、`p2`。
    pub fn unit(p1: Vec2, p2: Vec2) -> Self {
        Self::new(Vec2::ZERO, p1, p2, Vec2::ONE)
    }

    /// 曲线上的点。
    pub fn eval(&self, t: f32) -> Vec2 {
        let t = saturate(t);
        let u = 1.0 - t;
        let (uu, tt) = (u * u, t * t);
        self.p0 * (uu * u)
            + self.p1 * (3.0 * uu * t)
            + self.p2 * (3.0 * u * tt)
            + self.p3 * (tt * t)
    }

    /// 一阶导数。
    pub fn derivative(&self, t: f32) -> Vec2 {
        let u = 1.0 - t;
        (self.p1 - self.p0) * (3.0 * u * u)
            + (self.p2 - self.p1) * (6.0 * u * t)
            + (self.p3 - self.p2) * (3.0 * t * t)
    }

    /// 求 `t` 使 `x(t) == x`。牛顿迭代 + 二分兜底，保证收敛。
    pub fn solve_t_for_x(&self, x: f32) -> f32 {
        let x = x.clamp(self.p0.x.min(self.p3.x), self.p0.x.max(self.p3.x));
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        let mut t = ((x - self.p0.x) / (self.p3.x - self.p0.x)).clamp(0.0, 1.0);
        if !t.is_finite() {
            t = x.clamp(0.0, 1.0);
        }
        for _ in 0..8 {
            let fx = self.eval(t).x - x;
            if fx.abs() < 1e-6 {
                return t;
            }
            if fx > 0.0 {
                hi = t;
            } else {
                lo = t;
            }
            let d = self.derivative(t).x;
            let next = if d.abs() > 1e-6 { t - fx / d } else { f32::NAN };
            if next.is_finite() && next > lo && next < hi {
                t = next;
            } else {
                t = (lo + hi) * 0.5;
            }
        }
        t
    }

    /// 给定 `x` 求 `y`（标准缓动曲线的用法）。
    pub fn eval_y_at_x(&self, x: f32) -> f32 {
        self.eval(self.solve_t_for_x(x)).y
    }
}

/// 关键帧插值方式。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Easing {
    #[default]
    Linear,
    /// 保持前一关键帧的值直到下一关键帧（阶梯）。
    Step,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// 任意三次贝塞尔：控制点相对单位域给出。
    CubicBezier { p1: Vec2, p2: Vec2 },
}

impl Easing {
    /// 把 `t ∈ [0,1]` 映射为缓动后的进度。
    pub fn eval(&self, t: f32) -> f32 {
        let t = saturate(t);
        match *self {
            Easing::Linear => t,
            Easing::Step => 0.0,
            Easing::EaseIn => t * t * t,
            Easing::EaseOut => {
                let u = 1.0 - t;
                1.0 - u * u * u
            }
            Easing::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    let u = -2.0 * t + 2.0;
                    1.0 - u * u * u / 2.0
                }
            }
            Easing::CubicBezier { p1, p2 } => BezierSegment::unit(p1, p2).eval_y_at_x(t),
        }
    }

    /// 是否为线性（可用于跳过求值）。
    pub fn is_linear(&self) -> bool {
        matches!(self, Easing::Linear)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_unit_endpoints_are_exact() {
        let b = BezierSegment::unit(Vec2::new(0.25, 0.1), Vec2::new(0.25, 1.0));
        assert!((b.eval_y_at_x(0.0) - 0.0).abs() < 1e-4);
        assert!((b.eval_y_at_x(1.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn cubic_bezier_ease_in_out_is_symmetric() {
        let e = Easing::CubicBezier { p1: Vec2::new(0.42, 0.0), p2: Vec2::new(0.58, 1.0) };
        let a = e.eval(0.25);
        let b = e.eval(0.75);
        assert!((a + b - 1.0).abs() < 1e-3, "a={a} b={b}");
    }

    #[test]
    fn step_holds_previous_value() {
        assert_eq!(Easing::Step.eval(0.99), 0.0);
        assert_eq!(Easing::Step.eval(1.0), 0.0);
    }

    #[test]
    fn linear_is_identity() {
        assert_eq!(Easing::Linear.eval(0.37), 0.37);
    }

    #[test]
    fn easing_is_clamped_outside_unit_range() {
        assert_eq!(Easing::Linear.eval(-1.0), 0.0);
        assert_eq!(Easing::Linear.eval(2.0), 1.0);
    }
}
