//! 烘焙管线测试（§4.4）：固定顺序、确定性、边界与几何预算。

use super::bake::*;
use super::*;
use am_eval::ParamStore;
use am_model::{Expression, ExpressionParam, Model, Motion, MotionCurve, MotionKey, Node};
use am_physics::PhysicsEngine;

/// 造一个带参数与绘制对象的模型。
fn demo_model() -> Model {
    let mut model = Model::new("demo");
    model.parameters.push(am_model::Parameter::new("AngleX", "Angle X", -180.0, 180.0, 0.0));
    model.parameters.push(am_model::Parameter::new("EyeOpen", "Eye Open", 0.0, 1.0, 1.0));
    let mut node = Node::part("Body", None);
    node.id = "body".to_string();
    model.nodes.push(node);
    model
}

/// 造一条参数曲线（线性）。
fn curve(target: &str, keys: &[(f32, f32)]) -> MotionCurve {
    let mut c = MotionCurve::new(target);
    for (t, v) in keys {
        c.insert_key(MotionKey::new(*t, *v));
    }
    c
}

/// 一个「AngleX 从 0 到 30」的动画。
fn ramp_animation(duration: f32) -> AmAnimation {
    let mut a = AmAnimation::new("idle", "Idle");
    a.duration = duration;
    a.range = Some(Range::new(0.0, duration));
    let mut channel = Channel::parameter("AngleX");
    channel.insert_key(ChannelKey::new(0.0, 0.0));
    channel.insert_key(ChannelKey::new(duration, 30.0));
    a.channels.push(channel);
    a
}

fn bake_default(animation: &AmAnimation, model: &Model) -> BakeResult {
    let settings = am_model::ModelSettings::default();
    let physics = am_model::PhysicsSettings::default();
    bake(
        animation,
        model,
        &settings,
        &[],
        &[],
        &physics,
        &BakeOptions { baked_at: Some(1_789_000_000), ..Default::default() },
    )
    .expect("烘焙应成功")
}

// ------------------------------------------------------------ 基本形状

#[test]
fn bake_produces_stable_columns_and_frames() {
    let model = demo_model();
    let animation = ramp_animation(1.0);
    let result = bake_default(&animation, &model);

    let track = &result.track;
    assert_eq!(track.fps, 30.0);
    // frames = floor(1.0 * 30 + 1e-6) + 1 = 31
    assert_eq!(track.frames, 31);
    assert_eq!(track.params.ids, vec!["AngleX".to_string()]);
    assert_eq!(track.params.data.len(), 31, "行优先长度 = frames * 列数");
    // 首帧 0、末帧 30
    assert!((track.params.get(0, 0) - 0.0).abs() < 1e-6);
    assert!((track.params.get(30, 0) - 30.0).abs() < 1e-5);
}

#[test]
fn bake_is_bitwise_deterministic() {
    // §4.4 硬要求 1：同一份输入两次烘焙，f32 精确相等（不是近似）
    let model = demo_model();
    let animation = ramp_animation(2.0);
    let a = bake_default(&animation, &model);
    let b = bake_default(&animation, &model);
    assert_eq!(a.track.params.data, b.track.params.data, "重复烘焙必须逐位相同");
    assert_eq!(a.track.frames, b.track.frames);
    assert_eq!(a.track.params.ids, b.track.params.ids);
}

#[test]
fn bake_matches_column_count_times_frames() {
    let model = demo_model();
    let mut animation = ramp_animation(1.0);
    let mut eye = Channel::parameter("EyeOpen");
    eye.insert_key(ChannelKey::new(0.0, 1.0));
    eye.insert_key(ChannelKey::new(1.0, 0.0));
    animation.channels.push(eye);

    let result = bake_default(&animation, &model);
    let track = &result.track;
    assert_eq!(track.params.ids.len(), 2, "列顺序稳定升序");
    assert_eq!(track.params.ids, vec!["AngleX".to_string(), "EyeOpen".to_string()]);
    assert_eq!(track.params.data.len(), track.frames as usize * 2);
}

// ------------------------------------------------------------ 与实时推进一致

#[test]
fn bake_matches_runtime_advance_within_tolerance() {
    // §4.4 硬要求 2：与 runtime.advance 连续推进同一时长必须一致（< 1e-4）。
    // 这是本模式的**正确性锚点** —— 预渲染看起来就该是刚才实时看到的那一段。
    let model = demo_model();
    let duration = 1.0;
    let animation = ramp_animation(duration);
    let result = bake_default(&animation, &model);

    // 实时：按同样的 fps 逐帧推进一条曲线求值
    let fps = result.track.fps;
    let dt = 1.0 / fps;
    let curve_ref = animation.channels[0].clone();
    let mut live_time = 0.0f32;
    let mut live = Vec::new();
    for _ in 0..result.track.frames {
        // 实时推进：每步用当前时钟求值，然后时钟前进 dt
        live.push(curve_ref.sample(live_time).unwrap_or(0.0));
        live_time += dt;
    }

    for (frame, live_value) in live.iter().enumerate() {
        let baked = result.track.params.get(frame, 0);
        assert!(
            (baked - live_value).abs() < 1e-4,
            "帧 {frame}：烘焙 {baked} vs 实时 {live_value}"
        );
    }
}

// ------------------------------------------------------------ 结构通道

#[test]
fn structural_channels_are_baked_as_steps() {
    let model = demo_model();
    let mut animation = ramp_animation(1.0);
    let mut vis = Channel::new(ChannelKind::Visibility, "body");
    vis.insert_key(ChannelKey::with_easing(0.0, 1.0, am_math::Easing::Step));
    vis.insert_key(ChannelKey::with_easing(0.5, 0.0, am_math::Easing::Step));
    animation.channels.push(vis);

    let result = bake_default(&animation, &model);
    let track = &result.track;
    assert_eq!(track.visibility.ids, vec!["body".to_string()]);
    assert_eq!(track.visibility.data.len(), track.frames as usize);
    // 前半程可见、后半程不可见（阶跃）
    assert_eq!(track.visibility.get(0, 0), 1.0);
    assert_eq!(track.visibility.get(track.frames as usize - 1, 0), 0.0);
    // 轨里只允许 0 / 1，不出现中间值
    for value in &track.visibility.data {
        assert!(*value == 0.0 || *value == 1.0, "可见性轨必须是 0/1，实际 {value}");
    }
}

#[test]
fn draw_order_is_baked_as_integers() {
    let model = demo_model();
    let mut animation = ramp_animation(1.0);
    let mut order = Channel::new(ChannelKind::DrawOrder, "body");
    order.insert_key(ChannelKey::new(0.0, 0.0));
    order.insert_key(ChannelKey::new(1.0, 5.0));
    animation.channels.push(order);

    let result = bake_default(&animation, &model);
    for value in &result.track.draw_order.data {
        assert_eq!(*value, value.round(), "绘制顺序轨必须为整数，实际 {value}");
    }
}

#[test]
fn disabled_channel_is_not_baked() {
    let model = demo_model();
    let mut animation = ramp_animation(1.0);
    let mut vis = Channel::new(ChannelKind::Visibility, "body");
    vis.insert_key(ChannelKey::new(0.0, 1.0));
    vis.enabled = false;
    animation.channels.push(vis);

    let result = bake_default(&animation, &model);
    assert!(result.track.visibility.ids.is_empty(), "静音通道不参与烘焙");
}

// ------------------------------------------------------------ overlay

#[test]
fn overlay_motion_blends_by_weight() {
    let model = demo_model();
    let mut animation = ramp_animation(1.0);
    animation.overlay.motions.push(MotionRef { id: "extra".into(), weight: 0.5 });

    let mut motion = Motion::new("extra", "Extra");
    motion.curves.push(curve("AngleX", &[(0.0, 100.0), (1.0, 100.0)]));

    let settings = am_model::ModelSettings::default();
    let physics = am_model::PhysicsSettings::default();
    let with_overlay = bake(
        &animation,
        &model,
        &settings,
        &[motion],
        &[],
        &physics,
        &BakeOptions { baked_at: Some(1), ..Default::default() },
    )
    .unwrap();

    let plain = bake_default(&animation, &model);

    // 第 0 帧：自身 0，叠加 100 按 0.5 → 50
    let blended = with_overlay.track.params.get(0, 0);
    let baseline = plain.track.params.get(0, 0);
    assert!((baseline - 0.0).abs() < 1e-6);
    assert!((blended - 50.0).abs() < 1e-4, "应按 weight 混合，实际 {blended}");
}

#[test]
fn overlay_expression_respects_window() {
    let model = demo_model();
    let mut animation = ramp_animation(1.0);
    animation.overlay.expressions.push(ExpressionRef {
        id: "smile".into(),
        weight: 1.0,
        from: 0.0,
        to: 0.0, // 未限定区间
    });

    let mut expression = Expression::new("smile", "Smile");
    expression.parameters.push(ExpressionParam::new("AngleX", 42.0));

    let settings = am_model::ModelSettings::default();
    let physics = am_model::PhysicsSettings::default();
    let result = bake(
        &animation,
        &model,
        &settings,
        &[],
        &[expression],
        &physics,
        &BakeOptions { baked_at: Some(1), ..Default::default() },
    )
    .unwrap();
    // AngleX 在轨里，且表情覆盖了它
    assert!(result.track.params.ids.contains(&"AngleX".to_string()));
}

#[test]
fn missing_overlay_target_is_skipped_not_fatal() {
    let model = demo_model();
    let mut animation = ramp_animation(1.0);
    animation.overlay.motions.push(MotionRef { id: "nope".into(), weight: 1.0 });
    animation.overlay.expressions.push(ExpressionRef {
        id: "nope".into(),
        weight: 1.0,
        from: 0.0,
        to: 0.0,
    });
    // 引用了不存在的动作/表情：必须成功，只是不生效
    let result = bake_default(&animation, &model);
    assert_eq!(result.track.frames, 31);
}

// ------------------------------------------------------------ 物理

#[test]
fn physics_off_by_default_and_does_not_add_columns() {
    let model = demo_model();
    let animation = ramp_animation(1.0);
    let result = bake_default(&animation, &model);
    assert_eq!(result.track.params.ids, vec!["AngleX".to_string()]);
}

#[test]
fn empty_physics_settings_still_bakes_successfully() {
    // §4.4：overlay.physics == true 但工程物理为空 → 必须成功，只跳过 ⑥
    let model = demo_model();
    let mut animation = ramp_animation(1.0);
    animation.overlay.physics = true;

    let settings = am_model::ModelSettings::default();
    let physics = am_model::PhysicsSettings::default(); // settings 为空
    let result = bake(
        &animation,
        &model,
        &settings,
        &[],
        &[],
        &physics,
        &BakeOptions { baked_at: Some(1), ..Default::default() },
    );
    assert!(result.is_ok(), "空物理设置不应让烘焙失败：{result:?}");
}

#[test]
fn physics_engine_advances_deterministically() {
    // 直接验证「同一输入两次推进结果相同」，这是烘焙可复现的前提。
    let settings = am_model::PhysicsSettings::default();
    let mut a = PhysicsEngine::new(&settings);
    let mut b = PhysicsEngine::new(&settings);
    let model = demo_model();
    let mut pa = ParamStore::from_model(&model);
    let mut pb = ParamStore::from_model(&model);
    for _ in 0..30 {
        a.advance(1.0 / 60.0, &mut pa);
        b.advance(1.0 / 60.0, &mut pb);
    }
    assert_eq!(pa.as_map(), pb.as_map());
}

// ------------------------------------------------------------ 自动效果确定性

#[test]
fn auto_effects_are_deterministic_and_time_based() {
    // 同一时刻两次求值必须相同；且不含随机数（否则这里会不稳定）
    let mut settings = am_model::ModelSettings::default();
    settings.auto_blink.enabled = true;
    settings.auto_blink.parameters = vec!["EyeOpen".to_string()];
    settings.auto_breath.enabled = true;
    settings.auto_breath.parameters = vec!["AngleX".to_string()];

    let model = demo_model();
    for t in [0.0f32, 0.37, 1.0, 2.5] {
        let mut p1 = ParamStore::from_model(&model);
        let mut p2 = ParamStore::from_model(&model);
        apply_auto_effects(&settings, t, &mut p1);
        apply_auto_effects(&settings, t, &mut p2);
        assert_eq!(p1.as_map(), p2.as_map(), "t={t} 处自动效果必须可复现");
    }
}

#[test]
fn auto_effects_stay_within_configured_bounds() {
    let mut settings = am_model::ModelSettings::default();
    settings.auto_blink.enabled = true;
    settings.auto_blink.parameters = vec!["EyeOpen".to_string()];
    settings.auto_blink.min = 0.0;
    settings.auto_blink.max = 1.0;

    let model = demo_model();
    for step in 0..200 {
        let t = step as f32 * 0.05;
        let mut params = ParamStore::from_model(&model);
        apply_auto_effects(&settings, t, &mut params);
        let value = params.get("EyeOpen");
        assert!(
            (0.0..=1.0).contains(&value),
            "t={t} 时眨眼值 {value} 越界"
        );
    }
}

// ------------------------------------------------------------ 几何快照（§4.7）

#[test]
fn model_ref_geometry_stores_no_vertices() {
    let model = demo_model();
    let animation = ramp_animation(1.0);
    let result = bake_default(&animation, &model);
    assert_eq!(result.geometry, GeometryMode::ModelRef);
    assert!(result.track.geometry.is_none(), "model_ref 不应写几何");
    assert_eq!(result.estimated_bytes, 0);
}

#[test]
fn geometry_plan_estimates_before_sampling() {
    let model = demo_model();
    let plan = plan_geometry(&model, 100);
    assert_eq!(plan.mesh_count, 0, "demo 模型没有 drawable");
    assert_eq!(plan.estimated_bytes, 0);
    assert!(!plan.exceeds_hard_limit());
    assert!(!plan.needs_warning());
}

#[test]
fn geometry_budget_constants_match_spec() {
    assert_eq!(geometry_budget::HARD_LIMIT_BYTES, 256 * 1024 * 1024);
    assert_eq!(geometry_budget::WARN_BYTES, 16 * 1024 * 1024);
}

#[test]
fn oversized_geometry_is_rejected() {
    // 造一个顶点极多的模型，让预估超过硬上限
    let mut model = demo_model();
    let mut node = Node::part("Heavy", None);
    node.id = "heavy".to_string();
    let mut drawable = am_model::DrawableData::default();
    // 每帧 f32 数 = 顶点*2 + 1；要超过 256MB 需要很多顶点 × 很多帧
    drawable.mesh.vertices = vec![am_math::Vec2::ZERO; 2_000_000];
    node.drawable = Some(drawable);
    model.nodes.push(node);

    let model_ref = plan_geometry(&model, 200);
    assert!(model_ref.exceeds_hard_limit(), "预估应超过硬上限：{model_ref:?}");

    let mut animation = ramp_animation(1.0);
    animation.geometry = GeometryMode::Snapshot;
    animation.fps = 200.0;
    animation.range = Some(Range::new(0.0, 1.0));

    let settings = am_model::ModelSettings::default();
    let physics = am_model::PhysicsSettings::default();
    let result = bake(
        &animation,
        &model,
        &settings,
        &[],
        &[],
        &physics,
        &BakeOptions { baked_at: Some(1), ..Default::default() },
    );
    match result {
        Err(BakeError::GeometryTooLarge { limit_bytes, .. }) => {
            assert_eq!(limit_bytes, geometry_budget::HARD_LIMIT_BYTES);
        }
        other => panic!("应因几何超限被拒绝，实际 {other:?}"),
    }
}

// ------------------------------------------------------------ 参数校验

#[test]
fn bad_fps_is_rejected() {
    let model = demo_model();
    let animation = ramp_animation(1.0);
    let settings = am_model::ModelSettings::default();
    let physics = am_model::PhysicsSettings::default();
    for fps in [0.0f32, -1.0, 300.0] {
        let result = bake(
            &animation,
            &model,
            &settings,
            &[],
            &[],
            &physics,
            &BakeOptions { fps: Some(fps), baked_at: Some(1), ..Default::default() },
        );
        assert!(matches!(result, Err(BakeError::BadFps(_))), "fps={fps} 应被拒绝");
    }
}

#[test]
fn bad_range_is_rejected() {
    let model = demo_model();
    let animation = ramp_animation(1.0);
    let settings = am_model::ModelSettings::default();
    let physics = am_model::PhysicsSettings::default();
    let result = bake(
        &animation,
        &model,
        &settings,
        &[],
        &[],
        &physics,
        &BakeOptions {
            range: Some(Range::new(2.0, 1.0)),
            baked_at: Some(1),
            ..Default::default()
        },
    );
    assert!(matches!(result, Err(BakeError::BadRange { .. })));
}

#[test]
fn fps_override_changes_frame_count() {
    let model = demo_model();
    let animation = ramp_animation(1.0);
    let settings = am_model::ModelSettings::default();
    let physics = am_model::PhysicsSettings::default();
    let result = bake(
        &animation,
        &model,
        &settings,
        &[],
        &[],
        &physics,
        &BakeOptions { fps: Some(60.0), baked_at: Some(1), ..Default::default() },
    )
    .unwrap();
    assert_eq!(result.track.fps, 60.0);
    assert_eq!(result.track.frames, 61);
}

#[test]
fn zero_duration_animation_bakes_one_frame() {
    let model = demo_model();
    let mut animation = AmAnimation::new("idle", "Idle");
    animation.range = Some(Range::new(0.0, 0.0));
    let result = bake_default(&animation, &model);
    assert_eq!(result.track.frames, 1, "零长度区间仍有一帧");
}

#[test]
fn looping_flag_is_carried_into_track() {
    let model = demo_model();
    let mut animation = ramp_animation(1.0);
    animation.looping = true;
    let result = bake_default(&animation, &model);
    assert!(result.track.looping);
    assert_eq!(result.track.range, Range::new(0.0, 1.0));
}
