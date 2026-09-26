//! 视图变换：画布空间 ⇄ 屏幕像素 ⇄ NDC。
//!
//! 画布坐标 **Y 轴向上**（原点通常是画布中心）；宿主 UI 通常是 Y 轴向下。
//! 两套坐标的换算都在这里集中处理，避免各处手写正负号。

use am_math::{Mat3, Rect, Vec2};
use am_model::Canvas;
use serde::{Deserialize, Serialize};

/// 平移 + 缩放视图。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct View {
    /// 视图中心（画布坐标）。
    pub center: Vec2,
    /// 每个画布单位对应多少屏幕像素。
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self { center: Vec2::ZERO, zoom: 1.0 }
    }
}

impl View {
    pub fn new(center: Vec2, zoom: f32) -> Self {
        Self { center, zoom: sanitize_zoom(zoom) }
    }

    /// 让画布完整可见（留 5% 边距）。
    pub fn fit(canvas: &Canvas, width: u32, height: u32) -> Self {
        Self::fit_rect(canvas.rect(), width, height, 0.05)
    }

    /// 让某个矩形完整可见。
    pub fn fit_rect(rect: Rect, width: u32, height: u32, padding: f32) -> Self {
        let w = width.max(1) as f32;
        let h = height.max(1) as f32;
        let size = rect.size();
        if size.x <= 0.0 || size.y <= 0.0 {
            return Self { center: rect.center(), zoom: 1.0 };
        }
        let scale = (1.0 - 2.0 * padding.clamp(0.0, 0.45)).max(0.05);
        let zoom = (w / size.x).min(h / size.y) * scale;
        Self { center: rect.center(), zoom: sanitize_zoom(zoom) }
    }

    /// 当前可见的画布矩形。
    pub fn visible_rect(&self, width: u32, height: u32) -> Rect {
        let half = Vec2::new(width.max(1) as f32 / 2.0, height.max(1) as f32 / 2.0) / self.zoom;
        Rect::from_center_size(self.center, half * 2.0)
    }

    /// 画布 → NDC（供着色器使用，Y 轴向上）。
    pub fn to_matrix(&self, width: u32, height: u32) -> Mat3 {
        let rect = self.visible_rect(width, height);
        let sx = 2.0 / rect.width();
        let sy = 2.0 / rect.height();
        let c = rect.center();
        Mat3::new(sx, 0.0, 0.0, sy, -c.x * sx, -c.y * sy)
    }

    /// 画布 → 屏幕像素（Y 轴向下，符合 UI 习惯）。
    pub fn world_to_screen(&self, p: Vec2, width: u32, height: u32) -> Vec2 {
        Vec2::new(
            (p.x - self.center.x) * self.zoom + width as f32 / 2.0,
            height as f32 / 2.0 - (p.y - self.center.y) * self.zoom,
        )
    }

    /// 屏幕像素 → 画布。
    pub fn screen_to_world(&self, p: Vec2, width: u32, height: u32) -> Vec2 {
        Vec2::new(
            (p.x - width as f32 / 2.0) / self.zoom + self.center.x,
            (height as f32 / 2.0 - p.y) / self.zoom + self.center.y,
        )
    }

    /// 平移（按画布单位）。
    pub fn pan_world(&mut self, delta: Vec2) {
        self.center += delta;
    }

    /// 平移（按屏幕像素，自动换算）。
    pub fn pan_screen(&mut self, delta_px: Vec2) {
        self.center -= delta_px / self.zoom;
    }

    /// 以某个屏幕点为锚点缩放。
    pub fn zoom_at(&mut self, screen_point: Vec2, factor: f32, width: u32, height: u32) {
        let before = self.screen_to_world(screen_point, width, height);
        self.zoom = sanitize_zoom(self.zoom * factor);
        let after = self.screen_to_world(screen_point, width, height);
        self.center += before - after;
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = sanitize_zoom(zoom);
    }
}

fn sanitize_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() {
        zoom.clamp(1e-4, 1e4)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_centers_canvas() {
        let canvas = Canvas::new(100.0, 50.0);
        let view = View::fit(&canvas, 200, 200);
        assert_eq!(view.center, Vec2::ZERO);
        // 宽度是限制因素：100 * zoom * 0.9 <= 200
        assert!((view.zoom - 200.0 / 100.0 * 0.9).abs() < 1e-3);
    }

    #[test]
    fn matrix_maps_center_to_origin_of_ndc() {
        let view = View::new(Vec2::new(10.0, 20.0), 2.0);
        let m = view.to_matrix(400, 400);
        let ndc = m.transform_point(Vec2::new(10.0, 20.0));
        assert!((ndc - Vec2::ZERO).length() < 1e-6);
    }

    #[test]
    fn matrix_maps_visible_edge_to_ndc_edge() {
        let view = View::new(Vec2::ZERO, 1.0);
        let (w, h) = (200u32, 100u32);
        let m = view.to_matrix(w, h);
        let rect = view.visible_rect(w, h);
        let bottom_left = m.transform_point(rect.min);
        assert!((bottom_left - Vec2::new(-1.0, -1.0)).length() < 1e-5, "got {bottom_left:?}");
        let top_right = m.transform_point(rect.max);
        assert!((top_right - Vec2::new(1.0, 1.0)).length() < 1e-5, "got {top_right:?}");
    }

    #[test]
    fn screen_round_trip_is_y_flipped() {
        let view = View::new(Vec2::ZERO, 1.0);
        let (w, h) = (100u32, 80u32);
        // 画布 Y 向上 → 屏幕 Y 向下
        let top = view.world_to_screen(Vec2::new(0.0, 10.0), w, h);
        assert!((top - Vec2::new(50.0, 30.0)).length() < 1e-5, "got {top:?}");
        let back = view.screen_to_world(top, w, h);
        assert!((back - Vec2::new(0.0, 10.0)).length() < 1e-5);
    }

    #[test]
    fn zoom_at_keeps_anchor_fixed() {
        let mut view = View::new(Vec2::ZERO, 1.0);
        let (w, h) = (200u32, 200u32);
        let anchor = Vec2::new(150.0, 50.0);
        let before = view.screen_to_world(anchor, w, h);
        view.zoom_at(anchor, 2.5, w, h);
        let after = view.screen_to_world(anchor, w, h);
        assert!((before - after).length() < 1e-4, "{before:?} vs {after:?}");
        assert!((view.zoom - 2.5).abs() < 1e-6);
    }

    #[test]
    fn zoom_is_clamped_and_never_nan() {
        let mut view = View::default();
        view.set_zoom(f32::NAN);
        assert_eq!(view.zoom, 1.0);
        view.set_zoom(0.0);
        assert!(view.zoom >= 1e-4);
        view.set_zoom(1e9);
        assert!(view.zoom <= 1e4);
    }

    #[test]
    fn pan_screen_converts_units() {
        let mut view = View::new(Vec2::ZERO, 2.0);
        view.pan_screen(Vec2::new(20.0, -10.0));
        assert!((view.center - Vec2::new(-10.0, 5.0)).length() < 1e-6);
    }

    #[test]
    fn degenerate_rect_falls_back_to_center() {
        let view = View::fit_rect(Rect::ZERO, 100, 100, 0.05);
        assert_eq!(view.zoom, 1.0);
        assert_eq!(view.center, Vec2::ZERO);
    }
}
