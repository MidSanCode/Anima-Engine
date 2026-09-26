//! 二维仿射矩阵（3x3，最后一列为 `(0, 0, 1)`）。

use crate::Vec2;
use serde::{Deserialize, Serialize};

/// 二维仿射矩阵，按列存储：
///
/// ```text
/// | a  c  tx |
/// | b  d  ty |
/// | 0  0  1  |
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Mat3 {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Default for Mat3 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3 { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    #[inline]
    pub const fn new(a: f32, b: f32, c: f32, d: f32, tx: f32, ty: f32) -> Self {
        Self { a, b, c, d, tx, ty }
    }

    /// 平移矩阵。
    #[inline]
    pub fn from_translation(t: Vec2) -> Self {
        Self { tx: t.x, ty: t.y, ..Self::IDENTITY }
    }

    /// 旋转矩阵（弧度）。
    #[inline]
    pub fn from_rotation(rad: f32) -> Self {
        let (s, c) = rad.sin_cos();
        Self::new(c, s, -s, c, 0.0, 0.0)
    }

    /// 缩放矩阵。
    #[inline]
    pub fn from_scale(s: Vec2) -> Self {
        Self::new(s.x, 0.0, 0.0, s.y, 0.0, 0.0)
    }

    /// 组合变换：先缩放，再旋转，最后平移。
    #[inline]
    pub fn from_trs(translation: Vec2, rotation: f32, scale: Vec2) -> Self {
        Self::from_translation(translation) * Self::from_rotation(rotation) * Self::from_scale(scale)
    }

    /// 绕 `pivot` 做 TRS。
    #[inline]
    pub fn from_trs_around(translation: Vec2, rotation: f32, scale: Vec2, pivot: Vec2) -> Self {
        Self::from_translation(translation)
            * Self::from_translation(pivot)
            * Self::from_rotation(rotation)
            * Self::from_scale(scale)
            * Self::from_translation(-pivot)
    }

    #[inline]
    pub fn transform_point(&self, p: Vec2) -> Vec2 {
        Vec2::new(self.a * p.x + self.c * p.y + self.tx, self.b * p.x + self.d * p.y + self.ty)
    }

    /// 变换方向向量（忽略平移）。
    #[inline]
    pub fn transform_vector(&self, v: Vec2) -> Vec2 {
        Vec2::new(self.a * v.x + self.c * v.y, self.b * v.x + self.d * v.y)
    }

    #[inline]
    pub fn determinant(&self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    /// 求逆。奇异矩阵返回 `None`。
    pub fn inverse(&self) -> Option<Mat3> {
        let det = self.determinant();
        if det.abs() <= crate::EPSILON {
            return None;
        }
        let inv = 1.0 / det;
        Some(Mat3 {
            a: self.d * inv,
            b: -self.b * inv,
            c: -self.c * inv,
            d: self.a * inv,
            tx: (self.c * self.ty - self.d * self.tx) * inv,
            ty: (self.b * self.tx - self.a * self.ty) * inv,
        })
    }

    /// 提取平移分量。
    #[inline]
    pub fn translation(&self) -> Vec2 {
        Vec2::new(self.tx, self.ty)
    }

    /// 按上三角分解提取旋转角（弧度）。
    #[inline]
    pub fn rotation(&self) -> f32 {
        self.b.atan2(self.a)
    }

    /// 提取缩放（忽略剪切）。
    #[inline]
    pub fn scale(&self) -> Vec2 {
        Vec2::new(Vec2::new(self.a, self.b).length(), Vec2::new(self.c, self.d).length())
    }

    /// 转成列主序 3x3 数组（供着色器使用）。
    #[inline]
    pub fn to_cols_array_3x3(&self) -> [f32; 9] {
        [self.a, self.b, 0.0, self.c, self.d, 0.0, self.tx, self.ty, 1.0]
    }

    #[inline]
    pub fn is_finite(&self) -> bool {
        self.a.is_finite()
            && self.b.is_finite()
            && self.c.is_finite()
            && self.d.is_finite()
            && self.tx.is_finite()
            && self.ty.is_finite()
    }
}

impl std::ops::Mul for Mat3 {
    type Output = Mat3;
    /// `self * rhs`：先应用 `rhs`，再应用 `self`。
    #[inline]
    fn mul(self, r: Mat3) -> Mat3 {
        Mat3 {
            a: self.a * r.a + self.c * r.b,
            b: self.b * r.a + self.d * r.b,
            c: self.a * r.c + self.c * r.d,
            d: self.b * r.c + self.d * r.d,
            tx: self.a * r.tx + self.c * r.ty + self.tx,
            ty: self.b * r.tx + self.d * r.ty + self.ty,
        }
    }
}

impl std::ops::MulAssign for Mat3 {
    #[inline]
    fn mul_assign(&mut self, r: Mat3) {
        *self = *self * r;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn approx(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn trs_composes_in_scale_rotate_translate_order() {
        let m = Mat3::from_trs(Vec2::new(10.0, 0.0), FRAC_PI_2, Vec2::splat(2.0));
        let p = m.transform_point(Vec2::new(1.0, 0.0));
        assert!(approx(p, Vec2::new(10.0, 2.0)), "got {p:?}");
    }

    #[test]
    fn inverse_round_trips() {
        let m = Mat3::from_trs(Vec2::new(3.0, -4.0), 0.7, Vec2::new(1.5, 0.5));
        let inv = m.inverse().expect("invertible");
        let p = Vec2::new(2.0, 5.0);
        assert!(approx(inv.transform_point(m.transform_point(p)), p));
    }

    #[test]
    fn singular_matrix_has_no_inverse() {
        assert!(Mat3::from_scale(Vec2::new(0.0, 1.0)).inverse().is_none());
    }

    #[test]
    fn around_pivot_keeps_pivot_fixed() {
        let m = Mat3::from_trs_around(Vec2::ZERO, 1.3, Vec2::splat(3.0), Vec2::new(4.0, -2.0));
        let p = m.transform_point(Vec2::new(4.0, -2.0));
        assert!(approx(p, Vec2::new(4.0, -2.0)), "got {p:?}");
    }
}
