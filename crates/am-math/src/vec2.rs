//! 二维向量。

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// 二维向量（f32）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };
    pub const ONE: Vec2 = Vec2 { x: 1.0, y: 1.0 };
    pub const X: Vec2 = Vec2 { x: 1.0, y: 0.0 };
    pub const Y: Vec2 = Vec2 { x: 0.0, y: 1.0 };

    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[inline]
    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v }
    }

    /// 从角度（弧度）构造单位向量。
    #[inline]
    pub fn from_angle(rad: f32) -> Self {
        Self::new(rad.cos(), rad.sin())
    }

    #[inline]
    pub fn dot(self, o: Vec2) -> f32 {
        self.x * o.x + self.y * o.y
    }

    /// 二维叉积的标量结果。
    #[inline]
    pub fn cross(self, o: Vec2) -> f32 {
        self.x * o.y - self.y * o.x
    }

    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    #[inline]
    pub fn distance(self, o: Vec2) -> f32 {
        (self - o).length()
    }

    #[inline]
    pub fn distance_squared(self, o: Vec2) -> f32 {
        (self - o).length_squared()
    }

    /// 归一化；零向量返回零向量。
    #[inline]
    pub fn normalize_or_zero(self) -> Self {
        let len = self.length();
        if len <= super::EPSILON {
            Self::ZERO
        } else {
            self / len
        }
    }

    /// 逆时针旋转 90°。
    #[inline]
    pub fn perp(self) -> Self {
        Self::new(-self.y, self.x)
    }

    #[inline]
    pub fn angle(self) -> f32 {
        self.y.atan2(self.x)
    }

    #[inline]
    pub fn lerp(self, o: Vec2, t: f32) -> Self {
        Self::new(super::lerp(self.x, o.x, t), super::lerp(self.y, o.y, t))
    }

    #[inline]
    pub fn min(self, o: Vec2) -> Self {
        Self::new(self.x.min(o.x), self.y.min(o.y))
    }

    #[inline]
    pub fn max(self, o: Vec2) -> Self {
        Self::new(self.x.max(o.x), self.y.max(o.y))
    }

    #[inline]
    pub fn abs(self) -> Self {
        Self::new(self.x.abs(), self.y.abs())
    }

    /// 绕原点旋转。
    #[inline]
    pub fn rotated(self, rad: f32) -> Self {
        let (s, c) = rad.sin_cos();
        Self::new(self.x * c - self.y * s, self.x * s + self.y * c)
    }

    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    #[inline]
    pub fn to_array(self) -> [f32; 2] {
        [self.x, self.y]
    }
}

impl From<[f32; 2]> for Vec2 {
    fn from(v: [f32; 2]) -> Self {
        Self::new(v[0], v[1])
    }
}

impl From<Vec2> for [f32; 2] {
    fn from(v: Vec2) -> Self {
        v.to_array()
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    #[inline]
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    #[inline]
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn mul(self, s: f32) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}

impl Mul<Vec2> for f32 {
    type Output = Vec2;
    #[inline]
    fn mul(self, v: Vec2) -> Vec2 {
        v * self
    }
}

impl Mul<Vec2> for Vec2 {
    type Output = Vec2;
    /// 逐分量相乘。
    #[inline]
    fn mul(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x * o.x, self.y * o.y)
    }
}

impl Div<f32> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn div(self, s: f32) -> Vec2 {
        Vec2::new(self.x / s, self.y / s)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    #[inline]
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

impl AddAssign for Vec2 {
    #[inline]
    fn add_assign(&mut self, o: Vec2) {
        *self = *self + o;
    }
}

impl SubAssign for Vec2 {
    #[inline]
    fn sub_assign(&mut self, o: Vec2) {
        *self = *self - o;
    }
}

impl MulAssign<f32> for Vec2 {
    #[inline]
    fn mul_assign(&mut self, s: f32) {
        *self = *self * s;
    }
}

impl DivAssign<f32> for Vec2 {
    #[inline]
    fn div_assign(&mut self, s: f32) {
        *self = *self / s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_matches_angle_addition() {
        let v = Vec2::new(1.0, 0.0).rotated(std::f32::consts::FRAC_PI_2);
        assert!((v.x - 0.0).abs() < 1e-6);
        assert!((v.y - 1.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_zero_is_zero() {
        assert_eq!(Vec2::ZERO.normalize_or_zero(), Vec2::ZERO);
    }

    #[test]
    fn perp_is_counter_clockwise() {
        assert_eq!(Vec2::X.perp(), Vec2::Y);
    }
}
