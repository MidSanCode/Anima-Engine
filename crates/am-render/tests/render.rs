//! 离屏渲染的端到端测试（需要可用的图形适配器；没有则跳过）。
//!
//! GPU 设备创建很慢，因此全部用例共享一个渲染器（互斥串行执行）。

use am_eval::{evaluate, ParamStore};
use am_math::{Rect, Vec2};
use am_model::{BlendType, Canvas, Id, Mesh, Model, Node, TextureRef};
use am_render::{DecodedImage, Renderer, View};
use std::sync::{Mutex, MutexGuard, OnceLock};

const SIZE: u32 = 64;

fn gpu() -> Option<MutexGuard<'static, Renderer>> {
    static GPU: OnceLock<Option<Mutex<Renderer>>> = OnceLock::new();
    let cell = GPU.get_or_init(|| match Renderer::with_size(SIZE, SIZE) {
        Ok(r) => {
            eprintln!("[gpu] {}", r.adapter_info());
            Some(Mutex::new(r))
        }
        Err(e) => {
            eprintln!("[skip] 没有可用的图形适配器：{e}");
            None
        }
    });
    let guard = cell.as_ref()?.lock().unwrap_or_else(|e| e.into_inner());
    Some(guard)
}

/// 取得干净状态的渲染器（固定尺寸、无纹理）。
fn fresh() -> Option<MutexGuard<'static, Renderer>> {
    let mut renderer = gpu()?;
    renderer.resize(SIZE, SIZE).unwrap();
    renderer.clear_textures();
    Some(renderer)
}

fn pixel(px: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [px[i], px[i + 1], px[i + 2], px[i + 3]]
}

fn near(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    (0..4).all(|i| (a[i] as i32 - b[i] as i32).abs() <= tol)
}

fn quad_mesh(rect: Rect) -> Mesh {
    let (min, max) = (rect.min, rect.max);
    Mesh::new(
        vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)],
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

fn base_model() -> Model {
    let mut m = Model::new("render-test");
    m.canvas = Canvas::new(SIZE as f32, SIZE as f32);
    m.add_texture(TextureRef::new("assets/images/0.png"));
    m.add_texture(TextureRef::new("assets/images/1.png"));
    m
}

fn add_quad(m: &mut Model, name: &str, rect: Rect, texture: u32) -> Id {
    let id = m.add_node(Node::drawable(name, None, quad_mesh(rect)));
    m.node_mut(&id).unwrap().drawable.as_mut().unwrap().texture = Some(texture);
    id
}

fn center_rect(half: f32) -> Rect {
    Rect::from_min_max(Vec2::splat(-half), Vec2::splat(half))
}

fn scene_of(m: &Model) -> am_eval::Scene {
    evaluate(m, &ParamStore::from_model(m)).scene
}

fn view() -> View {
    View::new(Vec2::ZERO, 1.0)
}

// 画布 -16..16 对应屏幕 16..48；画布 x < 0 的遮罩覆盖屏幕 16..32
const INSIDE_MASK: (u32, u32) = (24, 32);
const OUTSIDE_MASK: (u32, u32) = (40, 32);

// ------------------------------------------------------------------ 用例

#[test]
fn solid_quad_covers_expected_pixels() {
    let Some(mut renderer) = fresh() else { return };
    let mut m = base_model();
    add_quad(&mut m, "Quad", center_rect(16.0), 0);
    renderer.set_texture(0, &DecodedImage::solid(4, 4, [255, 0, 0, 255])).unwrap();

    let px = renderer.render_to_pixels(&scene_of(&m), &view()).unwrap();
    assert_eq!(px.len(), (SIZE * SIZE * 4) as usize);
    assert_eq!(pixel(&px, 32, 32), [255, 0, 0, 255], "中心应为纯红");
    assert_eq!(pixel(&px, 4, 4), [0, 0, 0, 0], "外部应完全透明");
    assert_eq!(pixel(&px, 63, 63), [0, 0, 0, 0]);
}

#[test]
fn opacity_is_applied_to_pixels() {
    let Some(mut renderer) = fresh() else { return };
    let mut m = base_model();
    let id = add_quad(&mut m, "Quad", center_rect(16.0), 0);
    m.node_mut(&id).unwrap().drawable.as_mut().unwrap().opacity = 0.5;
    renderer.set_texture(0, &DecodedImage::solid(4, 4, [255, 0, 0, 255])).unwrap();

    let px = renderer.render_to_pixels(&scene_of(&m), &view()).unwrap();
    let c = pixel(&px, 32, 32);
    assert!(near(c, [128, 0, 0, 128], 4), "半透明应为预乘半值，实际 {c:?}");
}

#[test]
fn multiply_blend_darkens() {
    let Some(mut renderer) = fresh() else { return };
    let mut m = base_model();
    add_quad(&mut m, "Base", center_rect(16.0), 0);
    let top = add_quad(&mut m, "Top", center_rect(16.0), 1);
    m.node_mut(&top).unwrap().drawable.as_mut().unwrap().blend = BlendType::Multiply;
    renderer.set_texture(0, &DecodedImage::solid(4, 4, [255, 0, 0, 255])).unwrap();
    renderer.set_texture(1, &DecodedImage::solid(4, 4, [128, 128, 128, 255])).unwrap();

    let px = renderer.render_to_pixels(&scene_of(&m), &view()).unwrap();
    let c = pixel(&px, 32, 32);
    // src * dst = (0.5, 0.5, 0.5) * (1, 0, 0)
    assert!(near(c, [128, 0, 0, 255], 4), "multiply 结果应为暗红，实际 {c:?}");
}

#[test]
fn additive_blend_brightens() {
    let Some(mut renderer) = fresh() else { return };
    let mut m = base_model();
    add_quad(&mut m, "Base", center_rect(16.0), 0);
    let top = add_quad(&mut m, "Top", center_rect(16.0), 1);
    m.node_mut(&top).unwrap().drawable.as_mut().unwrap().blend = BlendType::Additive;
    renderer.set_texture(0, &DecodedImage::solid(4, 4, [51, 0, 0, 255])).unwrap();
    renderer.set_texture(1, &DecodedImage::solid(4, 4, [77, 0, 0, 255])).unwrap();

    let px = renderer.render_to_pixels(&scene_of(&m), &view()).unwrap();
    let c = pixel(&px, 32, 32);
    // 0.2 + 0.3 ≈ 0.5
    assert!(near(c, [128, 0, 0, 255], 4), "additive 结果应为 0.5 红，实际 {c:?}");
}

#[test]
fn screen_blend_lightens() {
    let Some(mut renderer) = fresh() else { return };
    let mut m = base_model();
    add_quad(&mut m, "Base", center_rect(16.0), 0);
    let top = add_quad(&mut m, "Top", center_rect(16.0), 1);
    m.node_mut(&top).unwrap().drawable.as_mut().unwrap().blend = BlendType::Screen;
    renderer.set_texture(0, &DecodedImage::solid(4, 4, [255, 0, 0, 255])).unwrap();
    renderer.set_texture(1, &DecodedImage::solid(4, 4, [128, 128, 128, 255])).unwrap();

    let px = renderer.render_to_pixels(&scene_of(&m), &view()).unwrap();
    let c = pixel(&px, 32, 32);
    // s + d(1-s) = (1.0, 0.5, 0.5)
    assert!(near(c, [255, 128, 128, 255], 4), "screen 结果应为亮红，实际 {c:?}");
}

#[test]
fn mask_clips_drawable_to_mask_shape() {
    let Some(mut renderer) = fresh() else { return };
    let mut m = base_model();
    let mask = add_quad(
        &mut m,
        "Mask",
        Rect::from_min_max(Vec2::new(-16.0, -16.0), Vec2::new(0.0, 16.0)),
        1,
    );
    let target = add_quad(&mut m, "Target", center_rect(16.0), 0);
    m.node_mut(&mask).unwrap().visible = false; // 遮罩来源本身不显示
    m.node_mut(&target).unwrap().drawable.as_mut().unwrap().masks = vec![mask.clone()];
    renderer.set_texture(0, &DecodedImage::solid(4, 4, [255, 0, 0, 255])).unwrap();
    renderer.set_texture(1, &DecodedImage::solid(4, 4, [255, 255, 255, 255])).unwrap();

    let px = renderer.render_to_pixels(&scene_of(&m), &view()).unwrap();
    assert_eq!(
        pixel(&px, INSIDE_MASK.0, INSIDE_MASK.1),
        [255, 0, 0, 255],
        "遮罩内应可见"
    );
    assert_eq!(
        pixel(&px, OUTSIDE_MASK.0, OUTSIDE_MASK.1),
        [0, 0, 0, 0],
        "遮罩外应被裁掉"
    );
}

#[test]
fn inverted_mask_keeps_the_opposite_side() {
    let Some(mut renderer) = fresh() else { return };
    let mut m = base_model();
    let mask = add_quad(
        &mut m,
        "Mask",
        Rect::from_min_max(Vec2::new(-16.0, -16.0), Vec2::new(0.0, 16.0)),
        1,
    );
    let target = add_quad(&mut m, "Target", center_rect(16.0), 0);
    m.node_mut(&mask).unwrap().visible = false;
    {
        let d = m.node_mut(&target).unwrap().drawable.as_mut().unwrap();
        d.masks = vec![mask.clone()];
        d.inverted_mask = true;
    }
    renderer.set_texture(0, &DecodedImage::solid(4, 4, [255, 0, 0, 255])).unwrap();
    renderer.set_texture(1, &DecodedImage::solid(4, 4, [255, 255, 255, 255])).unwrap();

    let px = renderer.render_to_pixels(&scene_of(&m), &view()).unwrap();
    assert_eq!(
        pixel(&px, INSIDE_MASK.0, INSIDE_MASK.1),
        [0, 0, 0, 0],
        "反向遮罩内应被裁掉"
    );
    assert_eq!(
        pixel(&px, OUTSIDE_MASK.0, OUTSIDE_MASK.1),
        [255, 0, 0, 255],
        "反向遮罩外应可见"
    );
}

#[test]
fn mask_can_come_from_a_part_with_children() {
    let Some(mut renderer) = fresh() else { return };
    let mut m = base_model();
    let part = m.add_node(Node::part("MaskPart", None));
    let child = add_quad(
        &mut m,
        "MaskChild",
        Rect::from_min_max(Vec2::new(-16.0, -16.0), Vec2::new(0.0, 16.0)),
        1,
    );
    m.node_mut(&child).unwrap().parent = Some(part.clone());
    let target = add_quad(&mut m, "Target", center_rect(16.0), 0);
    m.node_mut(&part).unwrap().visible = false;
    m.node_mut(&target).unwrap().drawable.as_mut().unwrap().masks = vec![part.clone()];
    renderer.set_texture(0, &DecodedImage::solid(4, 4, [255, 0, 0, 255])).unwrap();
    renderer.set_texture(1, &DecodedImage::solid(4, 4, [255, 255, 255, 255])).unwrap();

    let px = renderer.render_to_pixels(&scene_of(&m), &view()).unwrap();
    assert_eq!(
        pixel(&px, INSIDE_MASK.0, INSIDE_MASK.1),
        [255, 0, 0, 255],
        "部件下的子绘制对象应作为遮罩"
    );
    assert_eq!(pixel(&px, OUTSIDE_MASK.0, OUTSIDE_MASK.1), [0, 0, 0, 0]);
}

#[test]
fn empty_scene_is_transparent() {
    let Some(mut renderer) = fresh() else { return };
    let m = base_model();
    let px = renderer.render_to_pixels(&scene_of(&m), &view()).unwrap();
    assert!(px.iter().all(|b| *b == 0), "空场景应完全透明");
}

#[test]
fn view_zoom_and_pan_change_placement() {
    let Some(mut renderer) = fresh() else { return };
    let mut m = base_model();
    add_quad(&mut m, "Quad", center_rect(16.0), 0);
    renderer.set_texture(0, &DecodedImage::solid(4, 4, [0, 255, 0, 255])).unwrap();

    // 放大 2 倍：-16..16 覆盖整个视口
    let px = renderer.render_to_pixels(&scene_of(&m), &View::new(Vec2::ZERO, 2.0)).unwrap();
    assert_eq!(pixel(&px, 2, 2), [0, 255, 0, 255]);
    assert_eq!(pixel(&px, 61, 61), [0, 255, 0, 255]);

    // 平移出画面：全部透明
    let px = renderer
        .render_to_pixels(&scene_of(&m), &View::new(Vec2::new(200.0, 0.0), 1.0))
        .unwrap();
    assert!(px.iter().all(|b| *b == 0), "平移到画面外应为空");
}

#[test]
fn render_to_image_matches_target_size() {
    let Some(mut renderer) = fresh() else { return };
    let mut m = base_model();
    add_quad(&mut m, "Quad", center_rect(8.0), 0);
    renderer.set_texture(0, &DecodedImage::solid(2, 2, [10, 20, 30, 255])).unwrap();
    let image = renderer.render_to_image(&scene_of(&m), &view()).unwrap();
    assert_eq!((image.width, image.height), (SIZE, SIZE));
    assert_eq!(image.rgba.len(), (SIZE * SIZE * 4) as usize);
    assert_eq!(image.pixel(32, 32), [10, 20, 30, 255]);
}

#[test]
fn resize_changes_target_size() {
    let Some(mut renderer) = fresh() else { return };
    renderer.resize(32, 48).unwrap();
    assert_eq!(renderer.size(), (32, 48));
    let px = renderer.read_pixels().unwrap();
    assert_eq!(px.len(), 32 * 48 * 4);
    assert!(renderer.resize(0, 10).is_err());
    renderer.resize(SIZE, SIZE).unwrap();
}

#[test]
fn missing_texture_does_not_break_the_frame() {
    let Some(mut renderer) = fresh() else { return };
    let mut m = base_model();
    add_quad(&mut m, "Quad", center_rect(16.0), 0); // 纹理 0 未上传
    let px = renderer.render_to_pixels(&scene_of(&m), &view()).unwrap();
    assert!(px.iter().all(|b| *b == 0), "没有纹理时不应画出任何东西");
}

#[test]
fn invisible_drawable_is_not_rendered() {
    let Some(mut renderer) = fresh() else { return };
    let mut m = base_model();
    let id = add_quad(&mut m, "Quad", center_rect(16.0), 0);
    m.node_mut(&id).unwrap().visible = false;
    renderer.set_texture(0, &DecodedImage::solid(4, 4, [255, 0, 0, 255])).unwrap();
    let px = renderer.render_to_pixels(&scene_of(&m), &view()).unwrap();
    assert!(px.iter().all(|b| *b == 0), "隐藏的绘制对象不应出现在画面上");
}
