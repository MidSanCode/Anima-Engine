//! `am-motion`：动作播放状态机。
//!
//! 播放器只做三件事：**推进时间**、**采样曲线**、**按淡入淡出权重写入参数**。
//! 它不关心渲染，也不持有模型 —— 因此可以在引擎里跑，也可以在编辑器里跑，
//! 两边看到的时间轴完全一致。
//!
//! 淡入淡出的定义（自包含，不依赖模型默认值）：
//!
//! ```text
//! 播放开始：记录目标参数的当前取值快照 from
//! w = clamp(elapsed / fade_in, 0, 1)                       // 淡入
//! 非循环且剩余时间 < fade_out：w *= remaining / fade_out    // 淡出
//! 参数值 = from + (采样值 - from) * w
//! ```
//!
//! 因此「淡出」是回到**播放前的取值**，这也正是编辑器中拖动时间轴时的直觉。

use am_eval::ParamStore;
use am_model::{Id, Motion};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 播放参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlayOptions {
    /// 是否循环（缺省沿用动作自身的 `looping`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub looping: Option<bool>,
    /// 播放速度倍率。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    /// 覆盖动作自带的淡入时长（秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in: Option<f32>,
    /// 从指定时间开始（秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_time: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Active {
    id: Id,
    time: f32,
    speed: f32,
    looping: bool,
    finished: bool,
    /// 播放开始时的参数快照（淡入起点，也是淡出的终点）。
    from: BTreeMap<Id, f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Fade {
    duration: f32,
    elapsed: f32,
}

#[derive(Debug, thiserror::Error)]
pub enum MotionError {
    #[error("动作不存在：{0}")]
    NotFound(Id),
    #[error("动作没有有效时长：{0}")]
    EmptyMotion(Id),
}

/// 动作播放器。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MotionPlayer {
    motions: Vec<Motion>,
    active: Option<Active>,
    fade: Option<Fade>,
    paused: bool,
    /// 上一次写入的参数取值（无淡入时作为基准）。
    last_values: BTreeMap<Id, f32>,
}

impl MotionPlayer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_motions(motions: Vec<Motion>) -> Self {
        Self { motions, ..Default::default() }
    }

    // ------------------------------------------------------------ 动作库

    pub fn set_motions(&mut self, motions: Vec<Motion>) {
        self.motions = motions;
        if let Some(active) = &self.active {
            if self.motion(&active.id).is_none() {
                self.active = None;
            }
        }
    }

    pub fn motions(&self) -> &[Motion] {
        &self.motions
    }

    pub fn motion(&self, id: &str) -> Option<&Motion> {
        self.motions.iter().find(|m| m.id == id)
    }

    pub fn motion_mut(&mut self, id: &str) -> Option<&mut Motion> {
        self.motions.iter_mut().find(|m| m.id == id)
    }

    /// 加入动作（同 id 覆盖）。
    pub fn add_motion(&mut self, motion: Motion) {
        match self.motions.iter_mut().find(|m| m.id == motion.id) {
            Some(slot) => *slot = motion,
            None => self.motions.push(motion),
        }
    }

    pub fn remove_motion(&mut self, id: &str) -> Option<Motion> {
        let index = self.motions.iter().position(|m| m.id == id)?;
        let removed = self.motions.remove(index);
        if self.active.as_ref().map(|a| a.id.as_str()) == Some(id) {
            self.active = None;
            self.fade = None;
        }
        Some(removed)
    }

    // ------------------------------------------------------------ 播放控制

    /// 开始播放一个动作。
    pub fn play(
        &mut self,
        id: &str,
        options: &PlayOptions,
        params: &ParamStore,
    ) -> Result<(), MotionError> {
        let motion = self.motion(id).ok_or_else(|| MotionError::NotFound(id.to_string()))?;
        let duration = motion.effective_duration();
        if duration <= 0.0 {
            return Err(MotionError::EmptyMotion(id.to_string()));
        }
        let fade_in = options.fade_in.unwrap_or(motion.fade_in).max(0.0);
        let speed = options.speed.unwrap_or(1.0);
        let speed = if speed.is_finite() && speed > 0.0 { speed } else { 1.0 };
        let looping = options.looping.unwrap_or(motion.looping);

        // 记录淡入起点：本次动作会写到的全部参数
        let mut from = BTreeMap::new();
        for curve in &motion.curves {
            from.insert(curve.target.clone(), params.get(&curve.target));
        }
        let time = options.from_time.unwrap_or(0.0).clamp(0.0, duration);
        self.fade = if fade_in > 0.0 { Some(Fade { duration: fade_in, elapsed: 0.0 }) } else { None };
        self.active =
            Some(Active { id: id.to_string(), time, speed, looping, finished: false, from });
        self.paused = false;
        Ok(())
    }

    /// 停止播放（参数保持在当前取值）。
    pub fn stop(&mut self) {
        self.active = None;
        self.fade = None;
        self.paused = false;
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    pub fn resume(&mut self) {
        self.paused = false;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn is_playing(&self) -> bool {
        self.active.as_ref().map(|a| !a.finished).unwrap_or(false)
    }

    /// 当前动作是否已经播完（一次性动作到达结尾）。
    pub fn is_finished(&self) -> bool {
        self.active.as_ref().map(|a| a.finished).unwrap_or(true)
    }

    pub fn current(&self) -> Option<&str> {
        self.active.as_ref().map(|a| a.id.as_str())
    }

    /// 当前动作时间（秒）。
    pub fn time(&self) -> f32 {
        self.active.as_ref().map(|a| a.time).unwrap_or(0.0)
    }

    /// 跳转到指定时间。
    pub fn seek(&mut self, time: f32) {
        let duration = self
            .active
            .as_ref()
            .and_then(|a| self.motion(&a.id).map(|m| m.effective_duration()))
            .unwrap_or(0.0);
        if let Some(active) = &mut self.active {
            active.time = time.clamp(0.0, duration.max(0.0));
            active.finished = false;
        }
        if let Some(fade) = &mut self.fade {
            fade.elapsed = fade.duration; // 跳转时立即结束淡入
        }
    }

    pub fn speed(&self) -> f32 {
        self.active.as_ref().map(|a| a.speed).unwrap_or(1.0)
    }

    pub fn set_speed(&mut self, speed: f32) {
        if let Some(active) = &mut self.active {
            if speed.is_finite() && speed > 0.0 {
                active.speed = speed;
            }
        }
    }

    pub fn set_looping(&mut self, looping: bool) {
        if let Some(active) = &mut self.active {
            active.looping = looping;
        }
    }

    /// 淡入进度（0 = 完全在起点，1 = 完全由动作控制）。
    pub fn fade_weight(&self) -> f32 {
        match &self.fade {
            Some(fade) if fade.duration > 0.0 => (fade.elapsed / fade.duration).clamp(0.0, 1.0),
            Some(_) => 1.0,
            None => 1.0,
        }
    }

    // ------------------------------------------------------------ 推进

    /// 推进一帧；返回是否仍在播放。
    pub fn update(&mut self, dt: f32, params: &mut ParamStore) -> bool {
        if !dt.is_finite() || dt <= 0.0 || self.paused {
            return self.is_playing();
        }
        let Some(active) = self.active.clone() else {
            return false;
        };
        if active.finished {
            return false;
        }
        let Some(motion) = self.motion(&active.id).cloned() else {
            self.active = None;
            return false;
        };
        let duration = motion.effective_duration().max(0.0);

        let mut time = active.time + dt * active.speed;
        let mut finished = false;
        if duration <= 0.0 {
            time = 0.0;
            finished = true;
        } else if active.looping {
            time = time.rem_euclid(duration);
        } else if time >= duration {
            time = duration;
            finished = true;
        }

        let samples =
            if active.looping { motion.sample_looped(time) } else { motion.sample(time) };

        // 淡入权重
        let mut weight = match &mut self.fade {
            Some(fade) => {
                fade.elapsed += dt;
                if fade.elapsed >= fade.duration {
                    self.fade = None;
                    1.0
                } else {
                    (fade.elapsed / fade.duration).clamp(0.0, 1.0)
                }
            }
            None => 1.0,
        };
        // 淡出权重（非循环动作的收尾）
        if !active.looping && motion.fade_out > 0.0 && duration > 0.0 {
            let remaining = duration - time;
            if remaining < motion.fade_out {
                weight *= (remaining / motion.fade_out).clamp(0.0, 1.0);
            }
        }

        let from = active.from.clone();
        for (target, value) in samples {
            let base = from
                .get(&target)
                .copied()
                .unwrap_or_else(|| self.last_values.get(&target).copied().unwrap_or(value));
            let blended = base + (value - base) * weight;
            params.set(target.clone(), blended);
            self.last_values.insert(target, blended);
        }

        self.active = Some(Active { time, finished, ..active });
        !finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_model::{MotionCurve, MotionKey};

    fn curve(target: &str, keys: &[(f32, f32)]) -> MotionCurve {
        let mut c = MotionCurve::new(target);
        for (t, v) in keys {
            c.insert_key(MotionKey::new(*t, *v));
        }
        c
    }

    fn motion(id: &str, duration: f32, looping: bool, keys: &[(f32, f32)]) -> Motion {
        let mut m = Motion::new(id, id);
        m.duration = duration;
        m.looping = looping;
        m.curves = vec![curve("p", keys)];
        m
    }

    fn player() -> (MotionPlayer, ParamStore) {
        let mut m = motion("m1", 1.0, false, &[(0.0, 0.0), (1.0, 10.0)]);
        m.fade_in = 0.0;
        m.fade_out = 0.0;
        let mut looping = motion("loop", 1.0, true, &[(0.0, 0.0), (1.0, 10.0)]);
        looping.fade_in = 0.0;
        let player = MotionPlayer::with_motions(vec![m, looping]);
        let mut params = ParamStore::new();
        params.set("p", 0.0);
        (player, params)
    }

    #[test]
    fn play_and_update_writes_parameters() {
        let (mut player, mut params) = player();
        player.play("m1", &PlayOptions::default(), &params).unwrap();
        assert!(player.is_playing());
        player.update(0.5, &mut params);
        assert!((params.get("p") - 5.0).abs() < 1e-4, "got {}", params.get("p"));
        player.update(0.5, &mut params);
        assert!((params.get("p") - 10.0).abs() < 1e-4);
        assert!(!player.is_playing());
        assert!(player.is_finished());
    }

    #[test]
    fn looping_wraps_time() {
        let (mut player, mut params) = player();
        player.play("loop", &PlayOptions::default(), &params).unwrap();
        player.update(1.25, &mut params);
        assert!((player.time() - 0.25).abs() < 1e-4, "got {}", player.time());
        assert!(player.is_playing(), "循环动作不应结束");
        assert!((params.get("p") - 2.5).abs() < 1e-3, "got {}", params.get("p"));
    }

    #[test]
    fn speed_scales_time() {
        let (mut player, mut params) = player();
        player
            .play("loop", &PlayOptions { speed: Some(2.0), ..Default::default() }, &params)
            .unwrap();
        player.update(0.25, &mut params);
        assert!((player.time() - 0.5).abs() < 1e-4);
        player.set_speed(0.5);
        player.update(0.25, &mut params);
        assert!((player.time() - 0.625).abs() < 1e-4);
    }

    #[test]
    fn pause_freezes_time() {
        let (mut player, mut params) = player();
        player.play("loop", &PlayOptions::default(), &params).unwrap();
        player.update(0.3, &mut params);
        let t = player.time();
        player.pause();
        player.update(0.3, &mut params);
        assert_eq!(player.time(), t);
        player.resume();
        player.update(0.1, &mut params);
        assert!(player.time() > t);
    }

    #[test]
    fn fade_in_blends_from_current_values() {
        let mut m = motion("m1", 1.0, false, &[(0.0, 10.0), (1.0, 10.0)]);
        m.fade_in = 1.0;
        let mut player = MotionPlayer::with_motions(vec![m]);
        let mut params = ParamStore::new();
        params.set("p", 0.0);

        player.play("m1", &PlayOptions::default(), &params).unwrap();
        player.update(0.25, &mut params);
        assert!((params.get("p") - 2.5).abs() < 1e-3, "淡入 25%，got {}", params.get("p"));
        player.update(0.75, &mut params);
        assert!((params.get("p") - 10.0).abs() < 1e-3, "淡入结束应完全由动作控制");
        assert_eq!(player.fade_weight(), 1.0);
    }

    #[test]
    fn fade_out_returns_to_start_values() {
        let mut m = motion("m1", 1.0, false, &[(0.0, 0.0), (1.0, 10.0)]);
        m.fade_out = 0.5;
        m.fade_in = 0.0;
        let mut player = MotionPlayer::with_motions(vec![m]);
        let mut params = ParamStore::new();
        params.set("p", 3.0);

        player.play("m1", &PlayOptions::default(), &params).unwrap();
        player.update(0.5, &mut params);
        assert!((params.get("p") - 5.0).abs() < 1e-3, "got {}", params.get("p"));
        player.update(0.25, &mut params); // 剩余 0.25 → 权重 0.5
        let v = params.get("p");
        assert!(v < 7.5 && v > 3.0, "淡出应把取值拉回起点，got {v}");
        player.update(0.25, &mut params); // 结束 → 回到 3.0
        assert!((params.get("p") - 3.0).abs() < 1e-3, "got {}", params.get("p"));
    }

    #[test]
    fn seek_sets_time_and_skips_fade() {
        let mut m = motion("m1", 1.0, false, &[(0.0, 0.0), (1.0, 10.0)]);
        m.fade_in = 1.0;
        let mut player = MotionPlayer::with_motions(vec![m]);
        let mut params = ParamStore::new();
        player.play("m1", &PlayOptions::default(), &params).unwrap();
        player.seek(0.5);
        assert!((player.time() - 0.5).abs() < 1e-5);
        player.update(1.0 / 60.0, &mut params);
        assert!((params.get("p") - 5.0).abs() < 0.2, "跳转后不应再淡入，got {}", params.get("p"));
    }

    #[test]
    fn unknown_motion_is_reported() {
        let (mut player, params) = player();
        let err = player.play("nope", &PlayOptions::default(), &params).unwrap_err();
        assert!(matches!(err, MotionError::NotFound(_)));
    }

    #[test]
    fn empty_motion_cannot_play() {
        let mut m = Motion::new("empty", "empty");
        m.duration = 0.0;
        let mut player = MotionPlayer::with_motions(vec![m]);
        let params = ParamStore::new();
        assert!(matches!(
            player.play("empty", &PlayOptions::default(), &params),
            Err(MotionError::EmptyMotion(_))
        ));
    }

    #[test]
    fn update_without_active_motion_is_noop() {
        let (mut player, mut params) = player();
        params.set("p", 42.0);
        assert!(!player.update(0.1, &mut params));
        assert_eq!(params.get("p"), 42.0);
    }

    #[test]
    fn stop_keeps_last_values() {
        let (mut player, mut params) = player();
        player.play("m1", &PlayOptions::default(), &params).unwrap();
        player.update(0.5, &mut params);
        player.stop();
        assert!(!player.is_playing());
        assert!(player.current().is_none());
        let v = params.get("p");
        player.update(0.5, &mut params);
        assert_eq!(params.get("p"), v, "停止后不应再写入");
    }

    #[test]
    fn removing_active_motion_stops_playback() {
        let (mut player, params) = player();
        player.play("loop", &PlayOptions::default(), &params).unwrap();
        assert!(player.remove_motion("loop").is_some());
        assert!(!player.is_playing());
    }

    #[test]
    fn playback_is_deterministic() {
        let (mut a, mut pa) = player();
        let (mut b, mut pb) = player();
        a.play("m1", &PlayOptions::default(), &pa).unwrap();
        b.play("m1", &PlayOptions::default(), &pb).unwrap();
        for _ in 0..120 {
            a.update(1.0 / 60.0, &mut pa);
            b.update(1.0 / 60.0, &mut pb);
        }
        assert_eq!(pa.as_map(), pb.as_map());
    }

    #[test]
    fn state_round_trips_through_json() {
        let (mut player, params) = player();
        player.play("loop", &PlayOptions::default(), &params).unwrap();
        let json = serde_json::to_string(&player).unwrap();
        let back: MotionPlayer = serde_json::from_str(&json).unwrap();
        assert_eq!(player, back);
    }

    #[test]
    fn add_motion_replaces_same_id() {
        let (mut player, _) = player();
        let mut replacement = motion("m1", 2.0, true, &[(0.0, 1.0), (2.0, 2.0)]);
        replacement.name = "new".into();
        player.add_motion(replacement);
        assert_eq!(player.motions().len(), 2);
        assert_eq!(player.motion("m1").unwrap().name, "new");
        assert_eq!(player.motion("m1").unwrap().duration, 2.0);
    }

    #[test]
    fn non_finite_dt_is_ignored() {
        let (mut player, mut params) = player();
        player.play("loop", &PlayOptions::default(), &params).unwrap();
        player.update(f32::NAN, &mut params);
        assert_eq!(player.time(), 0.0);
        player.update(-1.0, &mut params);
        assert_eq!(player.time(), 0.0);
    }
}
