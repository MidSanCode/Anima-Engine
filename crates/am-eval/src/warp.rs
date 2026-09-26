//! 自由变形的网格映射（双线性）。
//!
//! `WarpMap` 把**静止网格**（`source` 矩形上的均匀格点）映射到**当前格点**（`target`）。
//! 静止姿态下两者重合，映射即恒等；因此没有关键形的模型完全不受影响。

use am_math::{Rect, Vec2};
use am_model::WarpData;
use serde::{Deserialize, Serialize};

/// 一次自由变形的映射。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WarpMap {
    pub rows: u32,
    pub cols: u32,
    /// 静止网格范围（节点局部空间）。
    pub source: Rect,
    /// 当前格点（父空间），长度恒为 `(rows+1)*(cols+1)`。
    pub target: Vec<Vec2>,
}

impl WarpMap {
    /// 构造映射。控制点数量不足时用静止格点补齐（容错，结构问题由 `am-model` 校验报告）。
    pub fn new(rows: u32, cols: u32, source: Rect, target: Vec<Vec2>) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let expected = ((rows + 1) * (cols + 1)) as usize;
        let mut pts = Vec::with_capacity(expected);
        for i in 0..expected {
            match target.get(i) {
                Some(p) => pts.push(*p),
                None => {
                    let r = i as u32 / (cols + 1);
                    let c = i as u32 % (cols + 1);
                    pts.push(Self::grid_point(source, rows, cols, r, c));
                }
            }
        }
        pts.truncate(expected);
        Self { rows, cols, source, target: pts }
    }

    /// 从 `WarpData` 构造；`rest_rect` 缺省时以控制点包围盒兜底。
    pub fn from_warp_data(warp: &WarpData) -> Self {
        let source = warp
            .rest_rect
            .or_else(|| Rect::from_points(warp.control_points.iter().copied()))
            .unwrap_or(Rect::ZERO);
        Self::new(warp.rows, warp.cols, source, warp.control_points.clone())
    }

    /// 静止网格上第 `(row, col)` 个格点。
    pub fn grid_point(source: Rect, rows: u32, cols: u32, row: u32, col: u32) -> Vec2 {
        let u = col as f32 / cols as f32;
        let v = row as f32 / rows as f32;
        Vec2::new(source.min.x + source.width() * u, source.min.y + source.height() * v)
    }

    /// 是否为恒等映射（容差内）。
    pub fn is_identity(&self, eps: f32) -> bool {
        self.target.iter().enumerate().all(|(i, p)| {
            let r = i as u32 / (self.cols + 1);
            let c = i as u32 % (self.cols + 1);
            (*p - Self::grid_point(self.source, self.rows, self.cols, r, c)).length() <= eps
        })
    }

    /// 映射一个点。网格外的点按最近边界单元格钳制（不会外插到失控位置）。
    pub fn map(&self, p: Vec2) -> Vec2 {
        let cols = self.cols.max(1);
        let rows = self.rows.max(1);
        let size = self.source.size();

        let fx = if size.x.abs() <= f32::EPSILON {
            0.0
        } else {
            (p.x - self.source.min.x) / size.x * cols as f32
        };
        let fy = if size.y.abs() <= f32::EPSILON {
            0.0
        } else {
            (p.y - self.source.min.y) / size.y * rows as f32
        };

        let cx = fx.floor().clamp(0.0, (cols - 1) as f32);
        let cy = fy.floor().clamp(0.0, (rows - 1) as f32);
        let tx = (fx - cx).clamp(0.0, 1.0);
        let ty = (fy - cy).clamp(0.0, 1.0);
        let (cu, cv) = (cx as u32, cy as u32);
        let stride = cols + 1;

        let at = |r: u32, c: u32| -> Vec2 {
            self.target
                .get((r * stride + c) as usize)
                .copied()
                .unwrap_or_else(|| Self::grid_point(self.source, rows, cols, r, c))
        };

        let bottom = at(cv, cu).lerp(at(cv, cu + 1), tx);
        let top = at(cv + 1, cu).lerp(at(cv + 1, cu + 1), tx);
        bottom.lerp(top, ty)
    }

    /// 把子节点的顶点批量映射。
    pub fn map_points(&self, points: &[Vec2]) -> Vec<Vec2> {
        points.iter().map(|p| self.map(*p)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Rect {
        Rect::from_min_max(Vec2::ZERO, Vec2::splat(10.0))
    }

    #[test]
    fn identity_when_target_equals_source_grid() {
        let w = WarpData::from_rect(2, 2, square());
        let map = WarpMap::from_warp_data(&w);
        assert!(map.is_identity(1e-6));
        for p in [Vec2::ZERO, Vec2::new(5.0, 5.0), Vec2::new(10.0, 10.0), Vec2::new(2.5, 7.5)] {
            assert!((map.map(p) - p).length() < 1e-5, "{p:?} → {:?}", map.map(p));
        }
    }

    #[test]
    fn translated_grid_translates_points() {
        let mut w = WarpData::from_rect(1, 1, square());
        for p in &mut w.control_points {
            *p += Vec2::new(3.0, -2.0);
        }
        let map = WarpMap::from_warp_data(&w);
        assert!(!map.is_identity(1e-3));
        let out = map.map(Vec2::new(5.0, 5.0));
        assert!((out - Vec2::new(8.0, 3.0)).length() < 1e-4, "got {out:?}");
    }

    #[test]
    fn non_uniform_grid_maps_bilinearly() {
        // 2x1 网格：右列整体上抬
        let mut w = WarpData::from_rect(1, 2, square());
        let stride = 3;
        for row in 0..2u32 {
            w.control_points[(row * stride + 2) as usize].y += 4.0;
        }
        let map = WarpMap::from_warp_data(&w);
        // 右边界中心点上抬 4
        let right = map.map(Vec2::new(10.0, 5.0));
        assert!((right.y - 9.0).abs() < 1e-4, "got {right:?}");
        // 左边界不动
        let left = map.map(Vec2::new(0.0, 5.0));
        assert!((left.y - 5.0).abs() < 1e-4, "got {left:?}");
        // 中间线性过渡：x 在 5..10 之间时上抬量按比例（7.5 → 2）
        let mid = map.map(Vec2::new(7.5, 5.0));
        assert!((mid.y - 7.0).abs() < 1e-4, "got {mid:?}");
        // 中间列线（x=5）不受右边界抬起影响
        let column = map.map(Vec2::new(5.0, 5.0));
        assert!((column.y - 5.0).abs() < 1e-4, "got {column:?}");
    }

    #[test]
    fn points_outside_are_clamped_not_extrapolated() {
        let mut w = WarpData::from_rect(1, 1, square());
        for p in &mut w.control_points {
            *p += Vec2::new(1.0, 1.0);
        }
        let map = WarpMap::from_warp_data(&w);
        let far = map.map(Vec2::new(1e6, 1e6));
        let corner = map.map(Vec2::new(10.0, 10.0));
        assert!((far - corner).length() < 1e-3, "外部点应钳制到边界: {far:?} vs {corner:?}");
    }

    #[test]
    fn missing_control_points_fall_back_to_grid() {
        let map = WarpMap::new(1, 1, square(), vec![Vec2::new(99.0, 99.0)]);
        assert_eq!(map.target.len(), 4);
        assert_eq!(map.target[0], Vec2::new(99.0, 99.0));
        assert_eq!(map.target[1], Vec2::new(10.0, 0.0));
        assert_eq!(map.target[3], Vec2::new(10.0, 10.0));
    }

    #[test]
    fn degenerate_source_does_not_produce_nan() {
        let map = WarpMap::new(1, 1, Rect::ZERO, vec![Vec2::ZERO; 4]);
        let out = map.map(Vec2::new(5.0, 5.0));
        assert!(out.is_finite(), "got {out:?}");
    }

    #[test]
    fn map_points_matches_single_map() {
        let mut w = WarpData::from_rect(2, 2, square());
        for p in &mut w.control_points {
            *p += Vec2::new(0.5, 0.25);
        }
        let map = WarpMap::from_warp_data(&w);
        let pts = vec![Vec2::new(1.0, 1.0), Vec2::new(9.0, 3.0)];
        let out = map.map_points(&pts);
        assert_eq!(out.len(), 2);
        for (a, b) in out.iter().zip(pts.iter()) {
            assert!((*a - map.map(*b)).length() < 1e-6);
        }
    }
}
