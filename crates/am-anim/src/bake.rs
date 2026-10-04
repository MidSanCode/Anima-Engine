//! 烘焙（§4.4）：把「曲线 + 叠加源 + 物理」压成一条逐帧轨。
//!
//! 这是本模式的核心，也是**唯一**会产出可交付数据的地方。因此这里的顺序是
//! 冻结契约，不能为了「看起来更合理」而调整：
//!
//! ```text
//! ① 本动画自身的参数通道（权重 1.0，直接写）
//! ② 叠加动作 overlay.motions      按 weight 混入当前值
//! ③ 结构通道 visibility / draw_order
//! ④ 叠加表情 overlay.expressions  区间内按 weight 淡入
//! ⑤ 自动效果 auto_effects         **确定性**，禁随机数与墙上时钟
//! ⑥ 物理 overlay.physics          固定步长 dt = 1/fps
//! ⑦ 记录本帧
//! ```
//!
//! 与 `Session::advance` 的「动作 → 物理 → 时钟」保持一致，并扩展了表情与
//! 自动效果 —— 所以「预渲染看起来就是刚才实时看到的那一段」。
//!
//! ## 参数集合一次性确定
//!
//! 采样哪些参数在烘焙**开始时**就固定（① 的 target ∪ ②④⑤ 会写到的参数 ∪
//! 物理输出参数），中途不因求值结果变化。否则不同帧的列数会不一致，
//! 行优先布局直接崩坏。

use crate::{
    AmAnimation, ChannelKind, GeometryMode, GeometryTrack, Track, TrackBlock,
};
use am_eval::{evaluate, ParamStore};
use am_model::{Id, Model, Motion, PhysicsSettings};
use am_physics::PhysicsEngine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// 烘焙选项（`animation.bake` 的入参）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BakeOptions {
    /// 覆盖帧率（缺省用动画自身的 `fps`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f32>,
    /// 覆盖区间（缺省用动画的 `effective_range`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<crate::Range>,
    /// 是否包含物理（缺省用 `overlay.physics`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_physics: Option<bool>,
    /// 是否包含自动效果（缺省用 `overlay.auto_effects`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_auto_effects: Option<bool>,
    /// 是否包含叠加源（缺省为真）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_overlays: Option<bool>,
    /// 几何模式（缺省用动画自身的 `geometry`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<GeometryMode>,
    /// 烘焙时间戳（Unix 秒）；`None` 时由调用方注入，保证测试可复现。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baked_at: Option<u64>,
}

/// 几何快照预算（§4.7）。
pub mod geometry_budget {
    /// 硬上限：超过则拒绝烘焙（`-32602`）。
    pub const HARD_LIMIT_BYTES: usize = 256 * 1024 * 1024;
    /// 预警线：超过则返回 `geometry_snapshot_large`（**不是错误**）。
    pub const WARN_BYTES: usize = 16 * 1024 * 1024;
}

/// 一次几何快照的规模估算（**先算再采**，避免真的把 256 MB 撑爆才发现）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeometryPlan {
    /// 参与快照的绘制对象数。
    pub mesh_count: usize,
    /// 每帧的 f32 个数（顶点 + 变形器控制点 + 不透明度）。
    pub floats_per_frame: usize,
    /// 预计字节数。
    pub estimated_bytes: usize,
}

impl GeometryPlan {
    pub fn exceeds_hard_limit(&self) -> bool {
        self.estimated_bytes > geometry_budget::HARD_LIMIT_BYTES
    }

    pub fn needs_warning(&self) -> bool {
        self.estimated_bytes > geometry_budget::WARN_BYTES
    }
}

/// 烘焙失败的原因。
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum BakeError {
    #[error("动画不存在：{0}")]
    AnimationNotFound(Id),
    #[error("帧率非法：{0}（应为 1..=240）")]
    BadFps(f32),
    #[error("区间非法：[{start}, {end}]")]
    BadRange { start: f32, end: f32 },
    #[error("几何快照体积 {estimated_bytes} 字节，超过上限 {limit_bytes} 字节（{frames} 帧）")]
    GeometryTooLarge { estimated_bytes: usize, limit_bytes: usize, frames: u32 },
}

/// 烘焙结果。
#[derive(Debug, Clone, PartialEq)]
pub struct BakeResult {
    pub track: Track,
    /// 体积预警码：`Some("geometry_snapshot_large")` 表示该提示用户。
    pub warning: Option<String>,
    pub estimated_bytes: usize,
    pub geometry: GeometryMode,
}

/// 计算几何快照规模（§4.7）。在**烘焙前**调用，用于预算判定。
pub fn plan_geometry(model: &Model, frames: u32) -> GeometryPlan {
    let mut floats_per_mesh = 0usize;
    let mut mesh_count = 0usize;
    for node in model.nodes.iter() {
        if let Some(drawable) = node.drawable.as_ref() {
            mesh_count += 1;
            floats_per_mesh += drawable.mesh.vertices.len() * 2;
        }
    }
    let mut floats_per_frame = floats_per_mesh + mesh_count; // 顶点 + opacity
    // 变形器控制点
    for node in model.nodes.iter() {
        if let Some(warp) = node.warp.as_ref() {
            floats_per_frame += warp.rows as usize * warp.cols as usize * 2;
        }
    }
    let estimated_bytes = floats_per_frame * frames as usize * std::mem::size_of::<f32>();
    GeometryPlan { mesh_count, floats_per_frame, estimated_bytes }
}

/// 确定性自动效果（§4.4 ⑤）。
///
/// **禁止随机数与墙上时钟**：否则同一份工程两次烘焙结果不同，逐位一致就没了。
/// 相位由「参数在列表中的索引」确定性派生 —— 它由工程数据决定，不由运行时状态决定。
pub fn apply_auto_effects(settings: &am_model::ModelSettings, time: f32, params: &mut ParamStore) {
    // 眨眼：固定周期的三角波，相位按参数索引错开。
    if settings.auto_blink.enabled {
        let effect = &settings.auto_blink;
        let interval = if effect.interval > 1e-6 { effect.interval } else { 4.0 };
        let duration = effect.duration.max(1e-6);
        for (index, id) in effect.parameters.iter().enumerate() {
            let phase = deterministic_phase(index) * interval;
            let cycle = ((time + phase) % interval + interval) % interval;
            let open = if cycle < duration {
                // 一次眨眼：闭→开，用平滑的三角波
                let t = (cycle / duration).clamp(0.0, 1.0);
                let blink = if t < 0.5 { t * 2.0 } else { (1.0 - t) * 2.0 };
                effect.max - (effect.max - effect.min) * blink
            } else {
                effect.max
            };
            params.set(id.clone(), open);
        }
    }

    // 呼吸：正弦，同样按索引错相。
    if settings.auto_breath.enabled {
        let effect = &settings.auto_breath;
        let interval = if effect.interval > 1e-6 { effect.interval } else { 4.0 };
        for (index, id) in effect.parameters.iter().enumerate() {
            let phase = deterministic_phase(index) * std::f32::consts::TAU;
            let wave = ((time / interval) * std::f32::consts::TAU + phase).sin();
            let value = effect.min + (effect.max - effect.min) * (wave * 0.5 + 0.5);
            params.set(id.clone(), value);
        }
    }
}

/// 由参数索引派生一个 `[0,1)` 的确定性相位（不用随机数）。
fn deterministic_phase(index: usize) -> f32 {
    // 黄金比例散列：分布均匀、完全由索引决定、跨平台一致。
    const GOLDEN: f32 = 0.618_034;
    let raw = (index as f32 + 1.0) * GOLDEN;
    raw - raw.floor()
}

/// 执行烘焙（§4.4）。
///
/// `model` 是结构模型，`motions` 是可用于 overlay 的动作库，
/// `expressions` 是表情库，`physics` 是工程物理设置。
pub fn bake(
    animation: &AmAnimation,
    model: &Model,
    settings: &am_model::ModelSettings,
    motions: &[Motion],
    expressions: &[am_model::Expression],
    physics_settings: &PhysicsSettings,
    options: &BakeOptions,
) -> Result<BakeResult, BakeError> {
    let fps = options.fps.unwrap_or(animation.fps);
    if !(fps.is_finite() && fps >= 1.0 && fps <= 240.0) {
        return Err(BakeError::BadFps(fps));
    }
    let range = options.range.unwrap_or_else(|| animation.effective_range());
    if !(range.start.is_finite() && range.end.is_finite()) || range.end < range.start {
        return Err(BakeError::BadRange { start: range.start, end: range.end });
    }

    let frames = range.frame_count(fps);
    let geometry_mode = options.geometry.unwrap_or(animation.geometry);
    let include_physics = options
        .include_physics
        .unwrap_or(animation.overlay.physics);
    let include_auto = options
        .include_auto_effects
        .unwrap_or(animation.overlay.auto_effects);
    let include_overlays = options.include_overlays.unwrap_or(true);

    // ---- 几何预算：先算再采（§4.7 硬上限）
    let geometry_plan = match geometry_mode {
        GeometryMode::Snapshot => Some(plan_geometry(model, frames)),
        GeometryMode::ModelRef => None,
    };
    if let Some(plan) = geometry_plan {
        if plan.exceeds_hard_limit() {
            return Err(BakeError::GeometryTooLarge {
                estimated_bytes: plan.estimated_bytes,
                limit_bytes: geometry_budget::HARD_LIMIT_BYTES,
                frames,
            });
        }
    }

    // ---- 参数集合一次性确定（见模块注释）
    let mut param_ids: BTreeSet<Id> = BTreeSet::new();
    for channel in &animation.channels {
        if channel.kind == ChannelKind::Parameter && channel.enabled {
            param_ids.insert(channel.target.clone());
        }
    }
    if include_overlays {
        for reference in &animation.overlay.motions {
            if let Some(motion) = motions.iter().find(|m| m.id == reference.id) {
                for curve in &motion.curves {
                    param_ids.insert(curve.target.clone());
                }
            }
        }
        for reference in &animation.overlay.expressions {
            if let Some(expression) = expressions.iter().find(|e| e.id == reference.id) {
                for param in &expression.parameters {
                    param_ids.insert(param.parameter.clone());
                }
            }
        }
    }
    if include_auto {
        for id in &settings.auto_blink.parameters {
            param_ids.insert(id.clone());
        }
        for id in &settings.auto_breath.parameters {
            param_ids.insert(id.clone());
        }
    }
    if include_physics && physics_settings.enabled && !physics_settings.settings.is_empty() {
        for setting in &physics_settings.settings {
            for output in &setting.outputs {
                param_ids.insert(output.parameter.clone());
            }
        }
    }

    // 结构通道：列顺序 = 稳定、去重、升序（BTreeSet 天然如此）
    let mut visibility_ids: BTreeSet<Id> = BTreeSet::new();
    let mut draw_order_ids: BTreeSet<Id> = BTreeSet::new();
    for channel in &animation.channels {
        if !channel.enabled {
            continue;
        }
        match channel.kind {
            ChannelKind::Visibility => {
                visibility_ids.insert(channel.target.clone());
            }
            ChannelKind::DrawOrder => {
                draw_order_ids.insert(channel.target.clone());
            }
            ChannelKind::Parameter => {}
        }
    }

    // ---- 采样缓冲（行优先）
    let param_ids: Vec<Id> = param_ids.into_iter().collect();
    let visibility_ids: Vec<Id> = visibility_ids.into_iter().collect();
    let draw_order_ids: Vec<Id> = draw_order_ids.into_iter().collect();

    let mut params_block = TrackBlock::new(param_ids.clone());
    let mut visibility_block = TrackBlock::new(visibility_ids.clone());
    let mut draw_order_block = TrackBlock::new(draw_order_ids.clone());
    let frame_count = frames as usize;
    params_block.normalize(frame_count);
    visibility_block.normalize(frame_count);
    draw_order_block.normalize(frame_count);

    // ---- 求值状态：物理是**有状态**的，必须逐帧推进，不能跳帧
    let mut physics = PhysicsEngine::new(physics_settings);
    let physics_active =
        include_physics && physics_settings.enabled && !physics_settings.settings.is_empty();

    let mut geometry = if geometry_mode == GeometryMode::Snapshot {
        Some(GeometryTrack::default())
    } else {
        None
    };

    let dt = 1.0 / fps;

    for frame in 0..frame_count {
        // 帧 i 的时间戳 = range.start + i / fps（§4.3 约定）
        let time = range.start + frame as f32 / fps;

        let mut params = ParamStore::from_model(model);

        // ① 本动画自身的参数通道（权重 1.0，直接写）
        for channel in &animation.channels {
            if channel.kind != ChannelKind::Parameter || !channel.enabled {
                continue;
            }
            if let Some(value) = channel.sample(time) {
                params.set(channel.target.clone(), value);
            }
        }

        // ② 叠加动作：按 weight 混入当前值
        if include_overlays {
            for reference in &animation.overlay.motions {
                let Some(motion) = motions.iter().find(|m| m.id == reference.id) else {
                    continue;
                };
                for (target, value) in motion.sample_looped(time) {
                    let current = params.get(&target);
                    params.set(target, current + (value - current) * reference.weight);
                }
            }
        }

        // ③ 结构通道：写可见性 / 绘制顺序（阶跃语义，取整后写）
        //
        // 注意写入的是**步进到本帧**的值：轨是逐帧采样，播放器不再插值，
        // 所以这里就要把 0/1 与整数定下来（与 §4.3 的阶跃读法对称）。
        for (column, id) in visibility_ids.iter().enumerate() {
            let value = animation
                .channels
                .iter()
                .find(|c| c.kind == ChannelKind::Visibility && c.target == *id)
                .and_then(|c| c.sample(time))
                .unwrap_or(0.0);
            visibility_block.write(frame, column, if value >= 0.5 { 1.0 } else { 0.0 });
        }
        for (column, id) in draw_order_ids.iter().enumerate() {
            let value = animation
                .channels
                .iter()
                .find(|c| c.kind == ChannelKind::DrawOrder && c.target == *id)
                .and_then(|c| c.sample(time))
                .unwrap_or(0.0);
            draw_order_block.write(frame, column, value.round());
        }

        // ④ 叠加表情
        if include_overlays {
            for reference in &animation.overlay.expressions {
                let Some(expression) = expressions.iter().find(|e| e.id == reference.id) else {
                    continue;
                };
                let width = reference.fade_weight(time);
                let weight = reference.weight * width;
                for param in &expression.parameters {
                    let current = params.get(&param.parameter);
                    params.set(
                        param.parameter.clone(),
                        current + (param.value - current) * weight,
                    );
                }
            }
        }

        // ⑤ 自动效果（确定性）
        if include_auto {
            apply_auto_effects(settings, time, &mut params);
        }

        // ⑥ 物理：固定步长推进
        if physics_active {
            physics.advance(dt, &mut params);
        }

        // ⑦ 记录本帧
        for (column, id) in param_ids.iter().enumerate() {
            params_block.write(frame, column, params.get(id));
        }

        // ⑦b 几何快照（§4.7）
        if let Some(geometry) = geometry.as_mut() {
            capture_geometry(model, &params, geometry, frame, time);
        }
    }

    // 几何轨的 ids / offsets 在采样后补齐（空场景也要自洽）
    if let Some(geometry) = geometry.as_mut() {
        finalize_geometry(model, geometry, frames);
    }

    let warning = geometry_plan
        .filter(|plan| plan.needs_warning())
        .map(|_| "geometry_snapshot_large".to_string());

    let estimated_bytes = geometry_plan.map(|p| p.estimated_bytes).unwrap_or(0);

    let track = Track {
        fps,
        frames,
        duration: range.span(),
        baked_at: options.baked_at.unwrap_or(0),
        params: params_block,
        visibility: visibility_block,
        draw_order: draw_order_block,
        geometry,
        range,
        looping: animation.looping,
    };

    Ok(BakeResult {
        track,
        warning,
        estimated_bytes,
        geometry: geometry_mode,
    })
}

/// 采集一帧几何（§4.7）：记录**该帧求值后的最终画布空间顶点**。
///
/// 播放时跳过关键形与变形器求值，直接用这些顶点 —— 所以这里必须求值到「画布空间」，
/// 不能存局部坐标。顺序必须与 `finalize_geometry` 的 `mesh_ids` 一致，
/// 否则播放端会把 A 的顶点画到 B 身上。
fn capture_geometry(
    model: &Model,
    params: &ParamStore,
    geometry: &mut GeometryTrack,
    _frame: usize,
    _time: f32,
) {
    let evaluated = evaluate(model, params);
    let drawable_order = drawable_ids(model);

    for id in &drawable_order {
        let Some(instance) = evaluated.drawable(id) else {
            continue;
        };
        for vertex in &instance.vertices {
            geometry.meshes.vertices.push(vertex.x);
            geometry.meshes.vertices.push(vertex.y);
        }
        geometry.meshes.opacity.push(instance.opacity);
    }

    // 变形器控制点（画布空间，已含变形器级联）
    for node in model.nodes.iter() {
        if node.warp.is_none() {
            continue;
        }
        if let Some(view) = evaluated.node_view(&node.id) {
            if let Some(points) = view.control_points_canvas.as_ref() {
                for point in points {
                    geometry.deformers.data.push(point.x);
                    geometry.deformers.data.push(point.y);
                }
            }
        }
    }
}

/// 模型里全部绘制对象 id，顺序固定（节点顺序 × 每节点的单个 drawable）。
fn drawable_ids(model: &Model) -> Vec<Id> {
    let mut ids = Vec::new();
    for node in model.nodes.iter() {
        if let Some(drawable) = node.drawable.as_ref() {
            ids.push(drawable_id(node, drawable));
        }
    }
    ids
}

/// 绘制对象的稳定 id：优先用节点 id（`DrawableData` 自身没有 id）。
fn drawable_id(node: &am_model::Node, _drawable: &am_model::DrawableData) -> Id {
    node.id.clone()
}

/// 补齐几何轨的 `mesh_ids` / `offsets` / 变形器行列数（让轨自洽可读）。
fn finalize_geometry(model: &Model, geometry: &mut GeometryTrack, _frames: u32) {
    let mut offsets = vec![0u32];
    let mut total: u32 = 0;
    for node in model.nodes.iter() {
        if let Some(drawable) = node.drawable.as_ref() {
            total += drawable.mesh.vertices.len() as u32;
            offsets.push(total);
        }
    }
    geometry.mesh_ids = drawable_ids(model);
    geometry.meshes.offsets = offsets;

    let mut deformer_ids = Vec::new();
    let mut rows = Vec::new();
    let mut cols = Vec::new();
    for node in model.nodes.iter() {
        if let Some(warp) = node.warp.as_ref() {
            deformer_ids.push(node.id.clone());
            rows.push(warp.rows);
            cols.push(warp.cols);
        }
    }
    geometry.deformers.ids = deformer_ids;
    geometry.deformers.rows = rows;
    geometry.deformers.cols = cols;
}
