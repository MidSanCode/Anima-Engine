//! `am-anim` 单元测试：数据模型、轨求值（§4.3 冻结算法）、区间与循环（§4.6）。

use super::*;

/// 造一条参数通道：`[(time, value)]` 全部线性。
fn linear_channel(target: &str, keys: &[(f32, f32)]) -> Channel {
    let mut c = Channel::parameter(target);
    for (t, v) in keys {
        c.insert_key(ChannelKey::new(*t, *v));
    }
    c
}

/// 造一个 4 帧、单参数通道、帧值 `[0,10,20,30]` 的轨。
fn four_frame_track() -> Track {
    let mut track = Track {
        fps: 30.0,
        frames: 4,
        range: Range::new(0.0, 3.0 / 30.0),
        ..Default::default()
    };
    track.params = TrackBlock::new(vec!["p".to_string()]);
    track.params.normalize(4);
    for (frame, value) in [0.0f32, 10.0, 20.0, 30.0].iter().enumerate() {
        track.params.write(frame, 0, *value);
    }
    track
}

// ------------------------------------------------------------ id

#[test]
fn animation_id_rules_match_spec() {
    assert!(is_valid_animation_id("idle"));
    assert!(is_valid_animation_id("walk-fast_2"));
    assert!(!is_valid_animation_id(""));
    assert!(!is_valid_animation_id("Idle")); // 大写不允许
    assert!(!is_valid_animation_id("has space"));
    assert!(!is_valid_animation_id("汉字"));
}

#[test]
fn normalize_id_strips_illegal_characters() {
    assert_eq!(normalize_animation_id("Idle Loop"), "idle-loop");
    assert_eq!(normalize_animation_id("  待机  "), "");
    assert_eq!(normalize_animation_id("a/b?c"), "abc");
}

// ------------------------------------------------------------ 通道

#[test]
fn channel_sampling_interpolates_and_clamps() {
    let c = linear_channel("AngleX", &[(0.0, 0.0), (1.0, 10.0)]);
    assert_eq!(c.sample(0.5), Some(5.0));
    assert_eq!(c.sample(-1.0), Some(0.0), "越界取首帧，不外推");
    assert_eq!(c.sample(99.0), Some(10.0), "越界取末帧，不外推");
}

#[test]
fn channel_step_easing_holds_previous_value() {
    let mut c = Channel::parameter("EyeOpen");
    c.insert_key(ChannelKey::with_easing(0.0, 0.0, Easing::Step));
    c.insert_key(ChannelKey::new(1.0, 1.0));
    assert_eq!(c.sample(0.99), Some(0.0));
    assert_eq!(c.sample(1.0), Some(1.0));
}

#[test]
fn disabled_channel_does_not_sample() {
    let mut c = linear_channel("p", &[(0.0, 0.0), (1.0, 1.0)]);
    c.enabled = false;
    assert_eq!(c.sample(0.5), None, "静音通道不参与求值");
}

#[test]
fn insert_key_keeps_order_and_overwrites_same_time() {
    let mut c = Channel::parameter("p");
    c.insert_key(ChannelKey::new(2.0, 20.0));
    c.insert_key(ChannelKey::new(1.0, 10.0));
    c.insert_key(ChannelKey::new(1.5, 15.0));
    c.insert_key(ChannelKey::new(1.5, 99.0));
    let times: Vec<f32> = c.keys.iter().map(|k| k.time).collect();
    assert_eq!(times, vec![1.0, 1.5, 2.0]);
    assert_eq!(c.keys[1].value, 99.0);
}

#[test]
fn duplicate_key_times_do_not_divide_by_zero() {
    let mut c = Channel::parameter("p");
    c.keys = vec![ChannelKey::new(1.0, 1.0), ChannelKey::new(1.0, 2.0)];
    assert_eq!(c.sample(1.0), Some(1.0));
}

// ------------------------------------------------------------ 区间与帧数

#[test]
fn frame_count_uses_frozen_formula() {
    // frames = floor((end-start)*fps + 1e-6) + 1
    assert_eq!(Range::new(0.0, 3.0).frame_count(30.0), 91);
    assert_eq!(Range::new(0.0, 1.0).frame_count(30.0), 31);
    assert_eq!(Range::new(0.0, 0.0).frame_count(30.0), 1);
    // 末帧时间戳恰好是 range.end
    let r = Range::new(0.0, 3.0);
    let frames = r.frame_count(30.0);
    let last_time = r.start + (frames - 1) as f32 / 30.0;
    assert!((last_time - r.end).abs() < 1e-5, "末帧时间戳应为 range.end");
}

#[test]
fn frame_count_survives_float_drift() {
    // 0.1 * 30 = 3.0000002 之类：1e-6 容差保证不会多出一帧
    let r = Range::new(0.0, 0.1);
    assert_eq!(r.frame_count(30.0), 4);
    let r2 = Range::new(0.0, 1.0 / 30.0);
    assert_eq!(r2.frame_count(30.0), 2);
}

#[test]
fn sample_coordinate_loops_in_time_domain() {
    let r = Range::new(0.0, 3.0);
    assert_eq!(r.sample_coordinate(1.5, 30.0, false), 45.0);
    assert_eq!(r.sample_coordinate(4.5, 30.0, true), 45.0, "循环回到 1.5s");
    assert_eq!(r.sample_coordinate(999.0, 30.0, false), 90.0, "非循环钳到区间末");
    // 起点偏移
    let r2 = Range::new(1.0, 3.0);
    assert_eq!(r2.sample_coordinate(1.0, 30.0, false), 0.0);
    assert_eq!(r2.sample_coordinate(2.0, 30.0, false), 30.0);
}

// ------------------------------------------------------------ 轨求值（§4.3）

#[test]
fn track_samples_integer_frames_exactly() {
    let t = four_frame_track();
    assert_eq!(t.sample(0.0).params["p"], 0.0);
    assert_eq!(t.sample(1.0 / 30.0).params["p"], 10.0);
    assert_eq!(t.sample(2.0 / 30.0).params["p"], 20.0);
    assert_eq!(t.sample(3.0 / 30.0).params["p"], 30.0);
}

#[test]
fn track_lerps_between_frames() {
    let t = four_frame_track();
    assert!((t.sample(0.5 / 30.0).params["p"] - 5.0).abs() < 1e-6);
    assert!((t.sample(1.25 / 30.0).params["p"] - 12.5).abs() < 1e-6);
}

#[test]
fn track_clamps_to_last_frame_without_extrapolating() {
    let t = four_frame_track();
    assert_eq!(t.sample(999.0).params["p"], 30.0, "末帧钳制，绝不外推");
    assert_eq!(t.sample(-5.0).params["p"], 0.0);
}

#[test]
fn track_out_of_range_data_does_not_panic() {
    let mut track = Track {
        fps: 30.0,
        frames: 4,
        range: Range::new(0.0, 3.0 / 30.0),
        ..Default::default()
    };
    track.params = TrackBlock::new(vec!["p".to_string()]);
    // 故意只给 2 个值（应为 4）
    track.params.data = vec![0.0, 10.0];
    assert_eq!(track.sample(999.0).params["p"], 0.0, "缺数据取 0，不 panic");
}

#[test]
fn empty_track_samples_to_empty() {
    let track = Track::default();
    let sample = track.sample(1.0);
    assert!(sample.params.is_empty());
    assert!(sample.visibility.is_empty());
}

#[test]
fn structural_channels_use_step_semantics() {
    let mut track = Track {
        fps: 30.0,
        frames: 4,
        range: Range::new(0.0, 3.0 / 30.0),
        ..Default::default()
    };
    track.visibility = TrackBlock::new(vec!["head".to_string()]);
    track.visibility.normalize(4);
    for (i, v) in [0.0f32, 1.0, 1.0, 0.0].iter().enumerate() {
        track.visibility.write(i, 0, *v);
    }
    track.draw_order = TrackBlock::new(vec!["arm".to_string()]);
    track.draw_order.normalize(4);
    for (i, v) in [3.0f32, 1.0, 0.0, 2.0].iter().enumerate() {
        track.draw_order.write(i, 0, *v);
    }

    assert!(!track.sample(0.0).visibility["head"]);
    assert!(track.sample(1.0 / 30.0).visibility["head"]);
    assert_eq!(track.sample(0.0).draw_order["arm"], 3);

    // 帧 0 与帧 1 之间：偏前仍是帧 0，偏后算帧 1（不插值）
    assert!(!track.sample(0.2 / 30.0).visibility["head"]);
    assert!(track.sample(0.8 / 30.0).visibility["head"]);
}

#[test]
fn track_bytes_counts_all_blocks() {
    let t = four_frame_track();
    assert_eq!(t.bytes(), 4 * std::mem::size_of::<f32>());
}

// ------------------------------------------------------------ 动画顶层

#[test]
fn effective_duration_falls_back_to_channels() {
    let mut a = AmAnimation::new("idle", "Idle");
    assert_eq!(a.effective_duration(), 0.0);
    a.channels.push(linear_channel("p", &[(0.0, 0.0), (2.5, 1.0)]));
    assert_eq!(a.effective_duration(), 2.5);

    // 显式 duration 优先
    a.duration = 4.0;
    assert_eq!(a.effective_duration(), 4.0);
}

#[test]
fn effective_range_defaults_to_duration() {
    let mut a = AmAnimation::new("idle", "Idle");
    a.duration = 3.0;
    let r = a.effective_range();
    assert_eq!(r.start, 0.0);
    assert_eq!(r.end, 3.0);

    a.range = Some(Range::new(1.0, 2.0));
    assert_eq!(a.effective_range(), Range::new(1.0, 2.0));
}

#[test]
fn is_baked_requires_frames() {
    let mut a = AmAnimation::new("idle", "Idle");
    assert!(!a.is_baked());
    a.track = Some(Track { frames: 0, ..Default::default() });
    assert!(!a.is_baked(), "frames=0 不算已烘焙");
    a.track = Some(four_frame_track());
    assert!(a.is_baked());
}

#[test]
fn upsert_channel_replaces_same_kind_and_target() {
    let mut a = AmAnimation::new("idle", "Idle");
    a.upsert_channel(linear_channel("AngleX", &[(0.0, 0.0), (1.0, 1.0)]));
    a.upsert_channel(linear_channel("AngleX", &[(0.0, 5.0), (1.0, 6.0)]));
    assert_eq!(a.channels.len(), 1, "同 kind+target 应覆盖而不是新增");
    assert_eq!(a.channels[0].keys[0].value, 5.0);

    // 不同 kind、同 target 是两条不同通道
    a.upsert_channel(Channel::new(ChannelKind::Visibility, "AngleX"));
    assert_eq!(a.channels.len(), 2);
}

#[test]
fn sample_channels_handles_structural_kinds() {
    let mut a = AmAnimation::new("idle", "Idle");
    a.duration = 1.0;
    a.upsert_channel(linear_channel("p", &[(0.0, 0.0), (1.0, 10.0)]));

    let mut vis = Channel::new(ChannelKind::Visibility, "head");
    vis.insert_key(ChannelKey::new(0.0, 1.0));
    vis.insert_key(ChannelKey::new(1.0, 0.0));
    a.upsert_channel(vis);

    let mut order = Channel::new(ChannelKind::DrawOrder, "arm");
    order.insert_key(ChannelKey::new(0.0, 0.0));
    order.insert_key(ChannelKey::new(1.0, 5.0));
    a.upsert_channel(order);

    let s = a.sample_channels(0.5);
    assert!((s.params["p"] - 5.0).abs() < 1e-6);
    assert!(s.visibility["head"], "0.5 处按 ≥0.5 判为可见");
    assert_eq!(s.draw_order["arm"], 3, "绘制顺序四舍五入");
}

#[test]
fn sample_channels_respects_looping() {
    let mut a = AmAnimation::new("idle", "Idle");
    a.duration = 1.0;
    a.looping = true;
    a.upsert_channel(linear_channel("p", &[(0.0, 0.0), (1.0, 10.0)]));
    // 循环一圈后回到起点
    let at_zero = a.sample_channels(0.0).params["p"];
    let at_one = a.sample_channels(1.0).params["p"];
    assert!((at_zero - at_one).abs() < 1e-5, "循环一圈应回到起点");
}

// ------------------------------------------------------------ 集合

#[test]
fn set_upsert_and_remove() {
    let mut set = AnimationSet::default();
    assert!(set.is_empty());
    set.upsert(AmAnimation::new("idle", "Idle"));
    set.upsert(AmAnimation::new("walk", "Walk"));
    assert_eq!(set.len(), 2);

    set.upsert(AmAnimation::new("idle", "Idle v2"));
    assert_eq!(set.len(), 2, "同 id 覆盖");
    assert_eq!(set.by_id("idle").unwrap().name, "Idle v2");

    assert!(set.remove("idle").is_some());
    assert!(!set.contains("idle"));
    assert!(set.remove("nope").is_none());
}

#[test]
fn suggest_id_avoids_collisions() {
    let mut set = AnimationSet::default();
    // 空基名退回 "anim"；此时集合为空，所以就是 anim 本身。
    assert_eq!(set.suggest_id(""), "anim");

    set.upsert(AmAnimation::new("anim", "A"));
    assert_eq!(set.suggest_id("anim"), "anim-2");
    set.upsert(AmAnimation::new("anim-2", "B"));
    assert_eq!(set.suggest_id("anim"), "anim-3");
    // 非法字符会被规范化后再查重
    assert_eq!(set.suggest_id("Anim Loop"), "anim-loop");
}

#[test]
fn animation_round_trips_through_json() {
    let mut a = AmAnimation::new("idle", "待机");
    a.duration = 3.0;
    a.looping = true;
    a.range = Some(Range::new(0.0, 3.0));
    a.upsert_channel(linear_channel("AngleX", &[(0.0, 0.0), (1.0, 30.0)]));
    a.overlay.motions.push(MotionRef { id: "breath".into(), weight: 1.0 });
    a.geometry = GeometryMode::Snapshot;

    let json = serde_json::to_string(&a).unwrap();
    let back: AmAnimation = serde_json::from_str(&json).unwrap();
    assert_eq!(a, back);
}

#[test]
fn partial_json_uses_defaults() {
    let a: AmAnimation =
        serde_json::from_str(r#"{"id":"idle","name":"Idle"}"#).unwrap();
    assert_eq!(a.version, ANIMATION_FORMAT_VERSION);
    assert_eq!(a.fps, 30.0);
    assert!(!a.looping);
    assert!(a.channels.is_empty());
    assert!(a.track.is_none());
    assert_eq!(a.geometry, GeometryMode::ModelRef);
}

#[test]
fn channel_kind_serializes_as_snake_case() {
    let json = serde_json::to_string(&ChannelKind::DrawOrder).unwrap();
    assert_eq!(json, r#""draw_order""#);
    let kind: ChannelKind = serde_json::from_str(r#""visibility""#).unwrap();
    assert_eq!(kind, ChannelKind::Visibility);
    assert!(ChannelKind::Visibility.is_structural());
    assert!(!ChannelKind::Parameter.is_structural());
}

#[test]
fn summary_reports_bake_state() {
    let mut a = AmAnimation::new("idle", "Idle");
    a.duration = 2.0;
    let s = a.summary();
    assert_eq!(s.id, "idle");
    assert_eq!(s.duration, 2.0);
    assert!(!s.baked);
    assert_eq!(s.baked_at, 0);
}

// ------------------------------------------------------------ 缓动映射

#[test]
fn tangent_mapping_matches_spec() {
    // §4.5：p1 = (0.42, in*0.42)、p2 = (0.58, 1 - out*0.42)
    let e = easing_from_tangents(1.0, 1.0);
    match e {
        Easing::CubicBezier { p1, p2 } => {
            assert!((p1.x - 0.42).abs() < 1e-6);
            assert!((p1.y - 0.42).abs() < 1e-6);
            assert!((p2.x - 0.58).abs() < 1e-6);
            assert!((p2.y - 0.58).abs() < 1e-6);
        }
        other => panic!("应为 CubicBezier，实际 {other:?}"),
    }
}

#[test]
fn easing_json_round_trips() {
    for e in [
        Easing::Linear,
        Easing::Step,
        Easing::EaseIn,
        Easing::CubicBezier {
            p1: Vec2::new(0.25, 0.1),
            p2: Vec2::new(0.25, 1.0),
        },
    ] {
        let value = easing_to_value(&e);
        assert_eq!(easing_from_value(&value), Some(e));
    }
}

#[test]
fn expression_ref_fade_weight_is_bounded() {
    let r = ExpressionRef { id: "smile".into(), weight: 1.0, from: 1.0, to: 3.0 };
    assert_eq!(r.fade_weight(0.5), 0.0, "区间外不生效");
    assert_eq!(r.fade_weight(5.0), 0.0);
    assert!((r.fade_weight(2.0) - 0.5).abs() < 1e-6);

    // 未限定区间 → 整段生效
    let open = ExpressionRef { id: "smile".into(), weight: 1.0, from: 0.0, to: 0.0 };
    assert!(!open.is_bounded());
    assert_eq!(open.fade_weight(99.0), 1.0);
}
