//! `am-physics`：确定性物理（摆锤 + 顶点链）。
//!
//! 设计原则：
//!
//! * **确定性**：固定步长积分，不使用随机数、不依赖墙钟；同样的输入序列必然得到同样的输出。
//!   这是录制回放、网络同步、黄金图测试的前提。
//! * **可重启**：状态完全保存在 [`PhysicsEngine`] 里，可序列化、可重置。
//! * **单位一致**：物理内部只产生归一化量 `[-1, 1]`（每个轴一个分量），
//!   每个输出再按自己的 [`NormalizationRange`] 映射回参数取值。
//!
//! 与参数系统的关系：物理**读取**输入参数、**写入**输出参数，是参数求值管线的一环，
//! 必须发生在关键形求值之前。

use am_eval::ParamStore;
use am_math::Vec2;
use am_model::{NormalizationRange, PhysicsKind, PhysicsSetting, PhysicsSettings};
use serde::{Deserialize, Serialize};

/// 单次 `advance` 最多推进的固定步数（防止卡顿后「追帧」雪崩）。
pub const MAX_STEPS_PER_ADVANCE: usize = 8;

/// 输入满量程对应的位移（画布单位）。顶点链用它把输入换算成位移。
pub const INPUT_GAIN: f32 = 40.0;

const EPS: f32 = 1e-6;
const HALF_PI: f32 = std::f32::consts::FRAC_PI_2;

/// 单个物理设定的运行状态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingState {
    pub id: String,
    /// 摆角（弧度）。
    pub angle: f32,
    pub angular_velocity: f32,
    /// 顶点链当前位置。
    pub points: Vec<Vec2>,
    pub velocities: Vec<Vec2>,
    /// 上一次写入的输出值（用于差分与调试）。
    pub outputs: Vec<f32>,
    input_x: f32,
    input_y: f32,
}

impl SettingState {
    fn new(setting: &PhysicsSetting) -> Self {
        let points: Vec<Vec2> = setting.vertices.iter().map(|v| v.position).collect();
        Self {
            id: setting.id.clone(),
            angle: 0.0,
            angular_velocity: 0.0,
            velocities: vec![Vec2::ZERO; points.len()],
            points,
            outputs: vec![0.0; setting.outputs.len()],
            input_x: 0.0,
            input_y: 0.0,
        }
    }

    /// 参数取值是否已经稳定（用于编辑器显示与性能优化）。
    pub fn is_settled(&self, eps: f32) -> bool {
        self.angular_velocity.abs() <= eps
            && self.velocities.iter().all(|v| v.length() <= eps)
    }
}

/// 物理引擎。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsEngine {
    settings: PhysicsSettings,
    states: Vec<SettingState>,
    accumulator: f32,
    steps: u64,
}

impl PhysicsEngine {
    /// 由物理设定创建。
    pub fn new(settings: &PhysicsSettings) -> Self {
        let states = settings.settings.iter().map(SettingState::new).collect();
        Self { settings: settings.clone(), states, accumulator: 0.0, steps: 0 }
    }

    /// 重新加载设定；能对上的状态会被保留（编辑器实时调参时不会「跳一下」）。
    pub fn reconfigure(&mut self, settings: &PhysicsSettings) {
        let mut next: Vec<SettingState> = Vec::with_capacity(settings.settings.len());
        for setting in &settings.settings {
            let fresh = SettingState::new(setting);
            match self.states.iter().find(|s| s.id == setting.id) {
                Some(old) if old.points.len() == fresh.points.len() => {
                    let mut kept = old.clone();
                    kept.outputs.resize(fresh.outputs.len(), 0.0);
                    next.push(kept);
                }
                _ => next.push(fresh),
            }
        }
        self.settings = settings.clone();
        self.states = next;
        self.accumulator = 0.0;
    }

    /// 回到静止状态。
    pub fn reset(&mut self) {
        self.states = self.settings.settings.iter().map(SettingState::new).collect();
        self.accumulator = 0.0;
        self.steps = 0;
    }

    pub fn settings(&self) -> &PhysicsSettings {
        &self.settings
    }

    pub fn states(&self) -> &[SettingState] {
        &self.states
    }

    /// 已推进的固定步数（可复现性测试用）。
    pub fn steps(&self) -> u64 {
        self.steps
    }

    pub fn is_enabled(&self) -> bool {
        self.settings.enabled && !self.settings.settings.is_empty()
    }

    /// 是否全部设定都已稳定。
    pub fn is_settled(&self, eps: f32) -> bool {
        self.states.iter().all(|s| s.is_settled(eps))
    }

    /// 推进物理；`dt` 为秒。
    pub fn advance(&mut self, dt: f32, params: &mut ParamStore) {
        if !self.is_enabled() || !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let step = self.settings.fixed_dt();
        self.accumulator += dt;
        let mut taken = 0;
        while self.accumulator + EPS >= step && taken < MAX_STEPS_PER_ADVANCE {
            self.step_fixed(step, params);
            self.accumulator -= step;
            taken += 1;
        }
        if taken == MAX_STEPS_PER_ADVANCE {
            // 追不上就丢弃积压，避免越拖越久
            self.accumulator = 0.0;
        }
    }

    /// 推进一个固定步长（测试与录制回放使用）。
    pub fn step_fixed(&mut self, dt: f32, params: &mut ParamStore) {
        if !self.is_enabled() || !dt.is_finite() || dt <= 0.0 {
            return;
        }
        for i in 0..self.settings.settings.len() {
            let setting = self.settings.settings[i].clone();
            if !setting.enabled {
                continue;
            }
            let state = &mut self.states[i];
            let (x, y) = read_inputs(&setting, params);
            let phys = match setting.kind {
                PhysicsKind::Pendulum => step_pendulum(&setting, state, x, y, dt),
                PhysicsKind::Vertex => step_vertices(&setting, state, x, y, dt, &self.settings),
            };
            state.input_x = x;
            state.input_y = y;
            write_outputs(&setting, state, phys, params);
        }
        self.steps += 1;
    }
}

/// 读取输入参数，返回 `(x, y)` 归一化分量（默认位置为 0）。
fn read_inputs(setting: &PhysicsSetting, params: &ParamStore) -> (f32, f32) {
    let mut x = 0.0;
    let mut y = 0.0;
    for (i, input) in setting.inputs.iter().enumerate() {
        let raw = params.get(&input.parameter);
        let mut value = input.normalization.normalize(raw);
        if input.inverted {
            value = -value;
        }
        value *= input.weight;
        if i == 1 {
            y += value;
        } else {
            x += value;
        }
    }
    (x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0))
}

/// 摆锤：阻尼弹簧跟随输入，`gravity` 提供重力偏置。
fn step_pendulum(
    setting: &PhysicsSetting,
    state: &mut SettingState,
    x: f32,
    y: f32,
    dt: f32,
) -> Vec2 {
    let pivot = setting.position_normalization.normalize(x).clamp(-1.0, 1.0);
    let target = (pivot * HALF_PI).clamp(-HALF_PI, HALF_PI);
    // 延迟越大，弹簧越软
    let stiffness = 8.0 / setting.delay.max(0.02);
    let damping = 2.0 * stiffness.sqrt() * 0.35;
    // 重力是**扭矩**（θ = 0 时为零），让摆永远想回到竖直
    let gravity_torque = -setting.gravity.clamp(0.0, 10.0) * state.angle.sin();
    let accel =
        -stiffness * (state.angle - target) - damping * state.angular_velocity + gravity_torque;
    state.angular_velocity += accel * dt;
    state.angle += state.angular_velocity * dt;
    if !state.angle.is_finite() || !state.angular_velocity.is_finite() {
        state.angle = target;
        state.angular_velocity = 0.0;
    }
    state.angle = state.angle.clamp(-HALF_PI, HALF_PI);
    state.angular_velocity = state.angular_velocity.clamp(-20.0, 20.0);
    let nx = (state.angle / HALF_PI).clamp(-1.0, 1.0);
    Vec2::new(nx, y.clamp(-1.0, 1.0))
}

/// 顶点链：每个顶点是带阻尼的弹簧点，输入把整条链拖走。
fn step_vertices(
    setting: &PhysicsSetting,
    state: &mut SettingState,
    x: f32,
    y: f32,
    dt: f32,
    global: &PhysicsSettings,
) -> Vec2 {
    if setting.vertices.is_empty() {
        return Vec2::new(x, y);
    }
    if state.points.len() != setting.vertices.len() {
        state.points = setting.vertices.iter().map(|v| v.position).collect();
        state.velocities = vec![Vec2::ZERO; state.points.len()];
    }

    let drive = Vec2::new(
        setting.position_normalization.normalize(x),
        setting.position_normalization.normalize(y),
    ) * INPUT_GAIN;
    let gravity = Vec2::new(global.gravity.x, global.gravity.y);
    let wind = global.wind;

    for i in 0..setting.vertices.len() {
        let v = &setting.vertices[i];
        let rest = v.position;
        let mobility = v.mobility.clamp(0.0, 1.0);
        let stiffness = 20.0 + v.acceleration.max(0.0) * 20.0;
        // 阻尼必须满足 c*dt < 2 才稳定；延迟越大阻尼越小（摆动越久）
        let damping = 2.0 / v.delay.max(0.05);
        let target = rest + drive * mobility;
        let accel = (target - state.points[i]) * stiffness * mobility
            + gravity * mobility
            + wind * mobility
            - state.velocities[i] * damping;
        state.velocities[i] += accel * dt;
        // 防御性钳制：极端参数下也不允许状态发散
        let limit = INPUT_GAIN * 4.0;
        state.velocities[i] = Vec2::new(
            state.velocities[i].x.clamp(-limit, limit),
            state.velocities[i].y.clamp(-limit, limit),
        );
        state.points[i] += state.velocities[i] * dt;
        state.points[i] = Vec2::new(
            state.points[i].x.clamp(rest.x - limit, rest.x + limit),
            state.points[i].y.clamp(rest.y - limit, rest.y + limit),
        );
        if !state.points[i].is_finite() {
            state.points[i] = rest;
            state.velocities[i] = Vec2::ZERO;
        }
    }

    // 输出取链尾相对静止位置的位移（归一化到 [-1, 1]）
    let last = setting.vertices.len() - 1;
    let disp = state.points[last] - setting.vertices[last].position;
    Vec2::new((disp.x / INPUT_GAIN).clamp(-1.0, 1.0), (disp.y / INPUT_GAIN).clamp(-1.0, 1.0))
}

/// 把归一化物理量写入输出参数。
fn write_outputs(
    setting: &PhysicsSetting,
    state: &mut SettingState,
    phys: Vec2,
    params: &mut ParamStore,
) {
    if state.outputs.len() != setting.outputs.len() {
        state.outputs.resize(setting.outputs.len(), 0.0);
    }
    for (i, out) in setting.outputs.iter().enumerate() {
        let mut component = if i == 0 { phys.x } else { phys.y };
        if out.reflect {
            component = component.abs();
        }
        component = (component * out.weight).clamp(-1.0, 1.0);
        let mut t = (component + 1.0) * 0.5;
        if out.inverted {
            t = 1.0 - t;
        }
        let range = out.normalization.sanitized();
        let value = denormalize(&range, t) * out.scale;
        let value = value.clamp(range.min.min(range.max), range.min.max(range.max));
        params.set(&out.parameter, value);
        state.outputs[i] = value;
    }
}

fn denormalize(range: &NormalizationRange, t: f32) -> f32 {
    range.min + (range.max - range.min) * t.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_model::{NormalizationRange, PhysicsInput, PhysicsOutput, PhysicsVertex};

    fn input(param: &str) -> PhysicsInput {
        PhysicsInput::new(param)
    }

    fn output(param: &str, min: f32, max: f32) -> PhysicsOutput {
        let mut o = PhysicsOutput::new(param);
        o.normalization = NormalizationRange { min, default: (min + max) * 0.5, max };
        o
    }

    fn pendulum() -> PhysicsSettings {
        let mut setting = PhysicsSetting::new("s1", "Hair");
        setting.kind = PhysicsKind::Pendulum;
        setting.inputs = vec![input("AngleX")];
        setting.outputs = vec![output("HairFront", -1.0, 1.0)];
        setting.delay = 0.5;
        let mut settings = PhysicsSettings::default();
        settings.settings = vec![setting];
        settings
    }

    fn store_with(params: &[(&str, f32)]) -> ParamStore {
        let mut store = ParamStore::new();
        for (k, v) in params {
            store.set(*k, *v);
        }
        store
    }

    #[test]
    fn disabled_engine_does_nothing() {
        let mut settings = pendulum();
        settings.enabled = false;
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 1.0), ("HairFront", 0.0)]);
        engine.advance(0.5, &mut params);
        assert_eq!(params.get("HairFront"), 0.0);
        assert_eq!(engine.steps(), 0);
    }

    #[test]
    fn empty_settings_are_harmless() {
        let settings = PhysicsSettings::default();
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 1.0)]);
        engine.advance(0.1, &mut params);
        assert!(engine.is_settled(1e-3));
    }

    #[test]
    fn rest_input_keeps_output_at_default() {
        let settings = pendulum();
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 0.0), ("HairFront", 99.0)]);
        for _ in 0..120 {
            engine.step_fixed(1.0 / 60.0, &mut params);
        }
        // 输入为默认值 → 归一化 0 → 目标摆角 0 → 输出取 normalization 中点
        assert!(params.get("HairFront").abs() < 1e-3, "got {}", params.get("HairFront"));
        assert!(engine.is_settled(1e-4));
    }

    #[test]
    fn pendulum_follows_input_and_converges() {
        let settings = pendulum();
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 1.0), ("HairFront", 0.0)]);
        for _ in 0..600 {
            engine.step_fixed(1.0 / 60.0, &mut params);
        }
        let value = params.get("HairFront");
        assert!(value > 0.9, "正向输入应把输出推向正向，got {value}");
        assert!(engine.is_settled(1e-3), "足够长时间后应稳定，状态 {:?}", engine.states()[0]);
    }

    #[test]
    fn inverted_input_flips_direction() {
        let mut settings = pendulum();
        settings.settings[0].inputs[0].inverted = true;
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 1.0), ("HairFront", 0.0)]);
        for _ in 0..240 {
            engine.step_fixed(1.0 / 60.0, &mut params);
        }
        assert!(params.get("HairFront") < -0.9, "got {}", params.get("HairFront"));
    }

    #[test]
    fn simulation_is_deterministic() {
        let settings = pendulum();
        let mut a = PhysicsEngine::new(&settings);
        let mut b = PhysicsEngine::new(&settings);
        let mut pa = store_with(&[("AngleX", 0.0), ("HairFront", 0.0)]);
        let mut pb = pa.clone();
        for i in 0..300 {
            let x = ((i as f32) * 0.05).sin();
            pa.set("AngleX", x);
            pb.set("AngleX", x);
            a.step_fixed(1.0 / 60.0, &mut pa);
            b.step_fixed(1.0 / 60.0, &mut pb);
        }
        assert_eq!(pa.as_map(), pb.as_map());
        assert_eq!(a.states(), b.states());
    }

    #[test]
    fn advance_uses_fixed_steps() {
        let settings = pendulum(); // fps = 60
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 1.0), ("HairFront", 0.0)]);
        engine.advance(1.0 / 30.0, &mut params);
        assert_eq!(engine.steps(), 2, "1/30 秒应推进两个 1/60 步");
    }

    #[test]
    fn huge_dt_is_capped() {
        let settings = pendulum();
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 1.0), ("HairFront", 0.0)]);
        engine.advance(10.0, &mut params);
        assert_eq!(engine.steps(), MAX_STEPS_PER_ADVANCE as u64);
    }

    #[test]
    fn non_positive_dt_is_ignored() {
        let settings = pendulum();
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 1.0)]);
        engine.advance(0.0, &mut params);
        engine.advance(-1.0, &mut params);
        engine.advance(f32::NAN, &mut params);
        assert_eq!(engine.steps(), 0);
    }

    #[test]
    fn reset_clears_state() {
        let settings = pendulum();
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 1.0), ("HairFront", 0.0)]);
        for _ in 0..60 {
            engine.step_fixed(1.0 / 60.0, &mut params);
        }
        assert!(!engine.is_settled(1e-3));
        engine.reset();
        assert!(engine.is_settled(1e-6));
        assert_eq!(engine.steps(), 0);
        assert_eq!(engine.states()[0].angle, 0.0);
    }

    #[test]
    fn reconfigure_keeps_matching_state() {
        let settings = pendulum();
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 1.0), ("HairFront", 0.0)]);
        for _ in 0..30 {
            engine.step_fixed(1.0 / 60.0, &mut params);
        }
        let angle = engine.states()[0].angle;
        assert!(angle.abs() > 1e-3);

        let mut tweaked = settings.clone();
        tweaked.settings[0].delay = 0.9;
        engine.reconfigure(&tweaked);
        assert!((engine.states()[0].angle - angle).abs() < 1e-6, "对得上的状态应保留");
        assert_eq!(engine.settings().settings[0].delay, 0.9);
    }

    #[test]
    fn reconfigure_resets_when_vertex_count_changes() {
        let mut setting = PhysicsSetting::new("s1", "Chain");
        setting.kind = PhysicsKind::Vertex;
        setting.vertices = vec![PhysicsVertex::new(Vec2::ZERO), PhysicsVertex::new(Vec2::new(0.0, -10.0))];
        setting.outputs = vec![output("HairFront", -1.0, 1.0)];
        let mut settings = PhysicsSettings::default();
        settings.settings = vec![setting.clone()];
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("HairFront", 0.0)]);
        for _ in 0..60 {
            engine.step_fixed(1.0 / 60.0, &mut params);
        }
        setting.vertices.push(PhysicsVertex::new(Vec2::new(0.0, -20.0)));
        let mut changed = PhysicsSettings::default();
        changed.settings = vec![setting];
        engine.reconfigure(&changed);
        assert_eq!(engine.states()[0].points.len(), 3);
        assert!(engine.states()[0].velocities.iter().all(|v| *v == Vec2::ZERO));
    }

    #[test]
    fn vertex_chain_reacts_to_input_and_returns_to_rest() {
        let mut setting = PhysicsSetting::new("s1", "Chain");
        setting.kind = PhysicsKind::Vertex;
        setting.inputs = vec![input("AngleX")];
        setting.outputs = vec![output("HairFront", -1.0, 1.0)];
        setting.vertices = vec![
            PhysicsVertex::new(Vec2::ZERO),
            PhysicsVertex::new(Vec2::new(0.0, -10.0)),
            PhysicsVertex::new(Vec2::new(0.0, -20.0)),
        ];
        let mut settings = PhysicsSettings::default();
        settings.settings = vec![setting];
        settings.gravity = Vec2::ZERO;

        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 1.0), ("HairFront", 0.0)]);
        for _ in 0..240 {
            engine.step_fixed(1.0 / 60.0, &mut params);
        }
        let moved = params.get("HairFront");
        assert!(moved > 0.1, "输入应把链尾拖走，got {moved}");

        params.set("AngleX", 0.0);
        for _ in 0..600 {
            engine.step_fixed(1.0 / 60.0, &mut params);
        }
        assert!(params.get("HairFront").abs() < 0.02, "回到默认输入应回到静止，got {}", params.get("HairFront"));
    }

    #[test]
    fn output_respects_normalization_range() {
        let mut settings = pendulum();
        settings.settings[0].outputs[0].normalization =
            NormalizationRange { min: -30.0, default: 0.0, max: 30.0 };
        settings.settings[0].outputs[0].scale = 10.0; // 故意超范围
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 1.0), ("HairFront", 0.0)]);
        for _ in 0..600 {
            engine.step_fixed(1.0 / 60.0, &mut params);
        }
        let v = params.get("HairFront");
        assert!((-30.0..=30.0).contains(&v), "输出必须落在归一化区间内，got {v}");
    }

    #[test]
    fn reflect_output_is_one_sided() {
        let mut settings = pendulum();
        settings.settings[0].outputs[0].reflect = true;
        settings.settings[0].inputs[0].inverted = true; // 让物理量取负
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 1.0), ("HairFront", 0.0)]);
        for _ in 0..300 {
            engine.step_fixed(1.0 / 60.0, &mut params);
        }
        assert!(params.get("HairFront") >= 0.0, "reflect 后输出不应为负");
    }

    #[test]
    fn vertex_setting_without_vertices_is_safe() {
        let mut setting = PhysicsSetting::new("s1", "Empty");
        setting.kind = PhysicsKind::Vertex;
        setting.outputs = vec![output("Out", -1.0, 1.0)];
        let mut settings = PhysicsSettings::default();
        settings.settings = vec![setting];
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("Out", 0.0)]);
        engine.step_fixed(1.0 / 60.0, &mut params);
        assert!(params.get("Out").is_finite());
    }

    #[test]
    fn state_round_trips_through_json() {
        let settings = pendulum();
        let mut engine = PhysicsEngine::new(&settings);
        let mut params = store_with(&[("AngleX", 1.0), ("HairFront", 0.0)]);
        for _ in 0..30 {
            engine.step_fixed(1.0 / 60.0, &mut params);
        }
        let json = serde_json::to_string(&engine).unwrap();
        let back: PhysicsEngine = serde_json::from_str(&json).unwrap();
        assert_eq!(engine, back);
    }
}
