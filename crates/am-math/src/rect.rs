//! 轴对齐包围盒。

use crate::{Mat3, Vec2};
use serde::{Deserialize, Serialize};

/// 轴对齐矩形，用最小/最大角点表示。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub const ZERO: Rect = Rect { min: Vec2::ZERO, max: Vec2::ZERO };

    #[inline]
    pub const fn new(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }

    #[inline]
    pub fn from_min_max(min: Vec2, max: Vec2) -> Self {
        Self { min: min.min(max), max: min.max(max) }
    }

    #[inline]
    pub fn from_center_size(center: Vec2, size: Vec2) -> Self {
        let half = size.abs() * 0.5;
        Self { min: center - half, max: center + half }
    }

    #[inline]
    pub fn from_min_size(min: Vec2, size: Vec2) -> Self {
        Self { min, max: min + size }
    }

    /// 从一组点构造；空集合返回 `None`。
    pub fn from_points(points: impl IntoIterator<Item = Vec2>) -> Option<Rect> {
        let mut it = points.into_iter();
        let first = it.next()?;
        let mut r = Rect { min: first, max: first };
        for p in it {
            r.min = r.min.min(p);
            r.max = r.max.max(p);
        }
        Some(r)
    }

    #[inline]
    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    #[inline]
    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    #[inline]
    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }

    #[inline]
    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    #[inline]
    pub fn area(&self) -> f32 {
        self.width().max(0.0) * self.height().max(0.0)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.width() <= 0.0 || self.height() <= 0.0
    }

    #[inline]
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    #[inline]
    pub fn union(&self, o: Rect) -> Rect {
        Rect { min: self.min.min(o.min), max: self.max.max(o.max) }
    }

    /// 求交；不相交时返回空矩形。
    pub fn intersection(&self, o: Rect) -> Rect {
        let min = self.min.max(o.min);
        let max = self.max.min(o.max);
        if min.x > max.x || min.y > max.y {
            Rect { min, max: min }
        } else {
            Rect { min, max }
        }
    }

    #[inline]
    pub fn expand(&self, amount: f32) -> Rect {
        Rect { min: self.min - Vec2::splat(amount), max: self.max + Vec2::splat(amount) }
    }

    #[inline]
    pub fn translate(&self, d: Vec2) -> Rect {
        Rect { min: self.min + d, max: self.max + d }
    }

    /// 变换后重新求包围盒。
    pub fn transformed(&self, m: Mat3) -> Rect {
        let corners = [
            m.transform_point(Vec2::new(self.min.x, self.min.y)),
            m.transform_point(Vec2::new(self.max.x, self.min.y)),
            m.transform_point(Vec2::new(self.max.x, self.max.y)),
            m.transform_point(Vec2::new(self.min.x, self.max.y)),
        ];
        Rect::from_points(corners).unwrap_or(Rect::ZERO)
    }

    /// 四条边（顺序：下、右、上、左）。
    pub fn edges(&self) -> [(Vec2, Vec2); 4] {
        let (bl, br, tr, tl) =
            (self.min, Vec2::new(self.max.x, self.min.y), self.max, Vec2::new(self.min.x, self.max.y));
        [(bl, br), (br, tr), (tr, tl), (tl, bl)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_points_of_unsorted_input() {
        let r = Rect::from_points([Vec2::new(1.0, 5.0), Vec2::new(-2.0, 0.0)]).unwrap();
        assert_eq!(r.min, Vec2::new(-2.0, 0.0));
        assert_eq!(r.max, Vec2::new(1.0, 5.0));
    }

    #[test]
    fn disjoint_intersection_is_empty() {
        let a = Rect::from_min_max(Vec2::ZERO, Vec2::splat(1.0));
        let b = Rect::from_min_max(Vec2::splat(5.0), Vec2::splat(6.0));
        assert!(a.intersection(b).is_empty());
    }

    #[test]
    fn transformed_uses_four_corners() {
        let r = Rect::from_min_max(Vec2::ZERO, Vec2::splat(2.0));
        let t = r.transformed(Mat3::from_rotation(std::f32::consts::FRAC_PI_4));
        assert!((t.width() - 2.0 * 2f32.sqrt()).abs() < 1e-4, "width={}", t.width());
    }
}
