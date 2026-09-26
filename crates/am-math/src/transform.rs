//! TRS 变换（带轴心）。

use crate::{Mat3, Vec2};
use serde::{Deserialize, Serialize};

/// 平移 / 旋转 / 缩放 / 轴心 组成的 2D 变换。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform2D {
    pub translation: Vec2,
    /// 弧度。
    pub rotation: f32,
    pub scale: Vec2,
    /// 变换轴心（局部坐标）。
    pub origin: Vec2,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform2D {
    pub const IDENTITY: Transform2D = Transform2D {
        translation: Vec2::ZERO,
        rotation: 0.0,
        scale: Vec2::ONE,
        origin: Vec2::ZERO,
    };

    pub const fn new(translation: Vec2, rotation: f32, scale: Vec2, origin: Vec2) -> Self {
        Self { translation, rotation, scale, origin }
    }

    pub fn from_translation(t: Vec2) -> Self {
        Self { translation: t, ..Self::IDENTITY }
    }

    pub fn from_rotation(rad: f32) -> Self {
        Self { rotation: rad, ..Self::IDENTITY }
    }

    pub fn from_scale(s: Vec2) -> Self {
        Self { scale: s, ..Self::IDENTITY }
    }

    pub fn to_mat3(&self) -> Mat3 {
        Mat3::from_trs_around(self.translation, self.rotation, self.scale, self.origin)
    }

    /// 从矩阵反解（忽略剪切；用于编辑器把矩阵回写成可拖拽的 TRS）。
    pub fn from_mat3(m: &Mat3) -> Self {
        Self {
            translation: m.translation(),
            rotation: m.rotation(),
            scale: m.scale(),
            origin: Vec2::ZERO,
        }
    }

    pub fn transform_point(&self, p: Vec2) -> Vec2 {
        self.to_mat3().transform_point(p)
    }

    pub fn lerp(&self, o: &Transform2D, t: f32) -> Transform2D {
        Transform2D {
            translation: self.translation.lerp(o.translation, t),
            rotation: crate::lerp_angle(self.rotation, o.rotation, t),
            scale: self.scale.lerp(o.scale, t),
            origin: self.origin.lerp(o.origin, t),
        }
    }

    pub fn is_finite(&self) -> bool {
        self.translation.is_finite() && self.rotation.is_finite() && self.scale.is_finite()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_neutral() {
        assert_eq!(Transform2D::IDENTITY.transform_point(Vec2::new(3.0, 4.0)), Vec2::new(3.0, 4.0));
    }

    #[test]
    fn origin_behaves_as_pivot() {
        let t = Transform2D::new(Vec2::ZERO, std::f32::consts::PI, Vec2::ONE, Vec2::new(1.0, 0.0));
        let p = t.transform_point(Vec2::new(1.0, 0.0));
        assert!((p - Vec2::new(1.0, 0.0)).length() < 1e-5, "got {p:?}");
    }

    #[test]
    fn lerp_interpolates_translation() {
        let a = Transform2D::from_translation(Vec2::ZERO);
        let b = Transform2D::from_translation(Vec2::new(10.0, 0.0));
        assert_eq!(a.lerp(&b, 0.25).translation, Vec2::new(2.5, 0.0));
    }
}
