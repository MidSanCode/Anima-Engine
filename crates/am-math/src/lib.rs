//! Anima 引擎基础数学库。
//!
//! 只提供 2D 渲染与参数求值所需的最小集合：向量、仿射矩阵、矩形、TRS 变换、曲线插值。
//! 坐标系约定（与 `tasks.md` §3.3 一致）：原点位于画布中心，Y 轴向上，单位 = 画布像素。

pub mod curve;
pub mod mat3;
pub mod rect;
pub mod transform;
pub mod vec2;

pub use curve::{BezierSegment, Easing};
pub use mat3::Mat3;
pub use rect::Rect;
pub use transform::Transform2D;
pub use vec2::Vec2;

/// 浮点比较容差。
pub const EPSILON: f32 = 1e-6;

/// 把 `v` 限制在 `[min, max]` 内。若 `min > max`，两者会被交换。
pub fn clamp(v: f32, min: f32, max: f32) -> f32 {
    if min <= max {
        v.max(min).min(max)
    } else {
        v.max(max).min(min)
    }
}

/// 限制到 `[0, 1]`。
pub fn saturate(v: f32) -> f32 {
    clamp(v, 0.0, 1.0)
}

/// 线性插值。
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// 反插值：`v` 在 `[a, b]` 中的归一化位置。`a == b` 时返回 0。
pub fn inverse_lerp(a: f32, b: f32, v: f32) -> f32 {
    if (b - a).abs() <= EPSILON {
        0.0
    } else {
        (v - a) / (b - a)
    }
}

/// 平滑阶跃。
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = saturate(inverse_lerp(edge0, edge1, x));
    t * t * (3.0 - 2.0 * t)
}

/// 把角度规范化到 `(-π, π]`。
pub fn wrap_angle(rad: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let mut a = rad % TAU;
    if a <= -PI {
        a += TAU;
    } else if a > PI {
        a -= TAU;
    }
    a
}

/// 角度插值（走最短弧）。
pub fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    a + wrap_angle(b - a) * t
}

/// 浮点近似相等。
pub fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_handles_reversed_bounds() {
        assert_eq!(clamp(5.0, 10.0, 0.0), 5.0);
        assert_eq!(clamp(-1.0, 10.0, 0.0), 0.0);
    }

    #[test]
    fn inverse_lerp_is_safe_on_degenerate_range() {
        assert_eq!(inverse_lerp(2.0, 2.0, 2.0), 0.0);
    }

    #[test]
    fn wrap_angle_canonicalizes() {
        use std::f32::consts::{PI, TAU};
        // 结果必须落在 (-π, π] 内（容差内）
        for k in [-7.0f32, -3.0, -1.5, -0.5, 0.0, 0.5, 1.5, 3.0, 7.0] {
            let w = wrap_angle(k * PI);
            assert!(w.abs() <= PI + 1e-4, "wrap_angle({k}π) = {w}");
        }
        // 整圈归零
        assert!(wrap_angle(TAU).abs() < 1e-4);
        assert!(wrap_angle(-TAU).abs() < 1e-4);
        // 小角度保持不变
        assert!(approx_eq(wrap_angle(0.25), 0.25));
    }

    #[test]
    fn lerp_angle_takes_short_way_around() {
        use std::f32::consts::PI;
        let r = lerp_angle(PI - 0.1, -PI + 0.1, 0.5);
        assert!(r.abs() > PI - 0.2, "got {r}");
    }
}
