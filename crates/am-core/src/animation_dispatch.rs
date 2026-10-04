//! `animation.*` 方法面（规范 §3）。
//!
//! 这里只做三件事：**取参数 → 改状态 → 回 JSON**。求值在 `am-anim`，
//! 模式规则在 [`crate::animation`]。所有错误都走 [`AnimationError`]，
//! 它自带 §9 的错误码与 `hint`，避免每个分支手写错误形状。
//!
//! ## 与其他方法的关系
//!
//! `prerender` 模式下 `runtime.*` 的写路径被闸门挡住（`-32010`），但
//! `runtime.scene` / `runtime.params` **仍然可用**，它们反映的是轨在当前
//! 时刻的值（规范 §3）。这就是「查看器只用 `animation.seek` +
//! `runtime.scene` 就能播动画」的原因。

use crate::animation::{
    find as find_animation, find_mut as find_animation_mut, parse_channel_kind, validate_new_id,
    AnimationError, Mode, ModeGuard, ModeState,
};
use crate::{param_f32, param_f64, param_str, Session};
use am_anim::{
    AmAnimation, BakeOptions, Channel, ChannelKey, ChannelKind, ExpressionRef, GeometryMode,
    MotionRef, Overlay, Range,
};
use am_model::Id;
use serde_json::{json, Value};

/// `animation.*` 的入口。返回 `Ok(None)` 表示「不是我的方法」。
impl Session {
    pub(crate) fn dispatch_animation(
        &mut self,
        method: &str,
        params: &Value,
    ) -> Result<Option<Value>, crate::ApiError> {
        if !method.starts_with("animation.") {
            return Ok(None);
        }
        // 未知的 animation.* 应当是 METHOD_NOT_FOUND，而不是「参数非法」——
        // 宿主会拿这个码判断能力缺失。这里先查清单，避免误报。
        if !crate::METHODS.contains(&method) {
            return Err(crate::ApiError::method_not_found(method));
        }
        let result = self.animation_inner(method, params)?;
        Ok(Some(result))
    }

    fn animation_inner(&mut self, method: &str, params: &Value) -> Result<Value, AnimationError> {
        match method {
            "animation.mode" => Ok(self.animation_mode()),
            "animation.new" => self.animation_new(params),
            "animation.open" => self.animation_open(params),
            "animation.close" => self.animation_close(),
            "animation.play" => self.animation_play(params),
            "animation.pause" => self.animation_pause(true),
            "animation.resume" => self.animation_pause(false),
            "animation.stop" => self.animation_stop(),
            "animation.seek" => self.animation_seek(params),
            "animation.set_speed" => self.animation_set_speed(params),
            "animation.set_loop" => self.animation_set_loop(params),
            "animation.state" => Ok(self.animation_state()),
            "animation.list" => self.animation_list(),
            "animation.query" => self.animation_query(params),
            "animation.set_meta" => self.animation_set_meta(params),
            "animation.delete" => self.animation_delete(params),
            "animation.channel.add" => self.animation_channel_add(params),
            "animation.channel.remove" => self.animation_channel_remove(params),
            "animation.key.set" => self.animation_key_set(params),
            "animation.key.remove" => self.animation_key_remove(params),
            "animation.key.move" => self.animation_key_move(params),
            "animation.channel.set_easing" => self.animation_channel_set_easing(params),
            "animation.curve.set" => self.animation_curve_set(params),
            "animation.overlay.set" => self.animation_overlay_set(params),
            "animation.set_param_ref" => self.animation_set_param_ref(params),
            "animation.bake" => self.animation_bake(params),
            "animation.baked" => self.animation_baked(params),
            "animation.sample" => self.animation_sample(params),
            other => Err(AnimationError::InvalidParams(format!("未知方法：{other}"))),
        }
    }
    // ------------------------------------------------------------ 模式

    fn animation_mode(&self) -> Value {
        json!({
            "mode": self.mode.mode.as_str(),
            "open": self.mode.open,
            "time": self.mode.time,
            "playing": self.mode.playing,
            "speed": self.mode.speed,
            "looping": self.mode.looping,
        })
    }

    fn animation_new(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let raw = param_str(params, "id").ok().unwrap_or_default().to_string();
        let id = if raw.is_empty() {
            let base = param_str(params, "name").ok().unwrap_or("anim").to_string();
            self.animations.suggest_id(&base)
        } else {
            raw
        };
        validate_new_id(&self.animations, &id)?;

        let name = param_str(params, "name")
            .ok()
            .unwrap_or(id.as_str())
            .to_string();
        let mut animation = AmAnimation::new(id.clone(), name);
        if let Some(fps) = param_f32(params, "fps") {
            animation.fps = fps;
        }
        if let Some(duration) = param_f32(params, "duration") {
            animation.duration = duration;
        }
        if let Some(looping) = params.get("looping").and_then(|v| v.as_bool()) {
            animation.looping = looping;
        }
        self.animations.upsert(animation.clone());
        self.encode_animations();
        Ok(json!({ "id": id, "animation": animation }))
    }

    /// 打开一段动画并切到 `prerender` 模式。
    fn animation_open(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let id = self.resolve_id(params)?;
        // 先确认存在且已烘焙，避免「切了模式却没有轨」的半死状态。
        let animation = find_animation(&self.animations, &id)?.clone();
        ModeGuard::require_baked(&animation, &id)?;

        self.mode.mode = Mode::Prerender;
        self.mode.open = Some(id.clone());
        self.mode.speed = 1.0;
        self.mode.playing = false;
        self.mode.looping = None;
        self.mode.fade_in = 0.0;
        self.mode.invalidate();

        self.mode.time = param_f32(params, "time").unwrap_or(0.0);
        self.apply_prerender_frame()?;

        Ok(json!({
            "mode": self.mode.mode.as_str(),
            "id": id,
            "time": self.mode.time,
            "state": self.animation_state(),
        }))
    }

    fn animation_close(&mut self) -> Result<Value, AnimationError> {
        self.mode = ModeState::live();
        Ok(json!({ "mode": "live", "id": Value::Null }))
    }

    fn animation_play(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let id = self.resolve_id(params)?;
        let animation = find_animation(&self.animations, &id)?.clone();
        ModeGuard::require_baked(&animation, &id)?;

        if self.mode.open.as_deref() != Some(id.as_str()) {
            self.mode.open = Some(id.clone());
            self.mode.time = 0.0;
        }
        self.mode.mode = Mode::Prerender;
        self.mode.playing = true;
        if let Some(speed) = param_f32(params, "speed") {
            self.mode.speed = speed;
        }
        if let Some(looping) = params.get("looping").and_then(|v| v.as_bool()) {
            self.mode.looping = Some(looping);
        }
        if let Some(fade_in) = param_f32(params, "fade_in") {
            self.mode.fade_in = fade_in.max(0.0);
        }
        // 非循环播放到底后重播：从区间起点开始（与 motion.play 的习惯一致）
        let range = animation.effective_range();
        if !self.looping_of(&animation) && self.mode.time >= range.end - 1e-6 {
            self.mode.time = range.start;
        }
        self.apply_prerender_frame()?;
        Ok(self.animation_state())
    }

    fn animation_pause(&mut self, paused: bool) -> Result<Value, AnimationError> {
        if !self.mode.is_prerender() {
            return Err(AnimationError::ModeConflict("animation.pause".into()));
        }
        self.mode.playing = !paused;
        Ok(self.animation_state())
    }

    fn animation_stop(&mut self) -> Result<Value, AnimationError> {
        self.mode.playing = false;
        if let Some(id) = self.mode.open.clone() {
            let animation = find_animation(&self.animations, &id)?.clone();
            self.mode.time = animation.effective_range().start;
        } else {
            self.mode.time = 0.0;
        }
        self.apply_prerender_frame()?;
        Ok(self.animation_state())
    }

    /// 定位到指定时间。**这是查看器唯一需要的写入方法。**
    fn animation_seek(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let id = self.resolve_id(params)?;
        let animation = find_animation(&self.animations, &id)?.clone();
        ModeGuard::require_baked(&animation, &id)?;

        if self.mode.open.as_deref() != Some(id.as_str()) {
            self.mode.open = Some(id.clone());
        }
        self.mode.mode = Mode::Prerender;
        self.mode.time = param_f32(params, "time")
            .ok_or_else(|| AnimationError::InvalidParams("animation.seek 需要 time".into()))?;
        self.apply_prerender_frame()?;

        let track = animation.track.as_ref().expect("已确认烘焙");
        let coordinate =
            track
                .range
                .sample_coordinate(self.mode.time, track.fps, self.looping_of(&animation));
        Ok(json!({
            "time": self.mode.time,
            "frame": coordinate.floor() as i64,
            "state": self.animation_state(),
        }))
    }

    fn animation_set_speed(&mut self, params: &Value) -> Result<Value, AnimationError> {
        if !self.mode.is_prerender() {
            return Err(AnimationError::ModeConflict("animation.set_speed".into()));
        }
        let speed = param_f32(params, "speed").ok_or_else(|| {
            AnimationError::InvalidParams("animation.set_speed 需要 speed".into())
        })?;
        if !speed.is_finite() {
            return Err(AnimationError::InvalidParams("speed 必须是有限数".into()));
        }
        self.mode.speed = speed;
        Ok(self.animation_state())
    }

    fn animation_set_loop(&mut self, params: &Value) -> Result<Value, AnimationError> {
        if !self.mode.is_prerender() {
            return Err(AnimationError::ModeConflict("animation.set_loop".into()));
        }
        let looping = params
            .get("looping")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| {
                AnimationError::InvalidParams("animation.set_loop 需要 looping".into())
            })?;
        self.mode.looping = Some(looping);
        self.mode.invalidate();
        self.apply_prerender_frame()?;
        Ok(self.animation_state())
    }

    fn animation_state(&self) -> Value {
        let baked = self
            .mode
            .open
            .as_deref()
            .and_then(|id| self.animations.by_id(id))
            .map(|a| a.is_baked())
            .unwrap_or(false);
        json!({
            "mode": self.mode.mode.as_str(),
            "id": self.mode.open,
            "time": self.mode.time,
            "playing": self.mode.playing,
            "speed": self.mode.speed,
            "looping": self.mode.looping,
            "fade_in": self.mode.fade_in,
            "baked": baked,
        })
    }

    /// 解析目标动画 id：显式 `id` 优先，否则用当前打开的。
    fn resolve_id(&self, params: &Value) -> Result<Id, AnimationError> {
        match param_str(params, "id").ok() {
            Some(id) => Ok(id.to_string()),
            None => self.mode.open.clone().ok_or(AnimationError::MissingId),
        }
    }

    /// 该动画当前有效的循环设置（模式覆盖优先于动画自身）。
    fn looping_of(&self, animation: &AmAnimation) -> bool {
        self.mode.looping.unwrap_or(animation.looping)
    }

    /// 按当前 `mode.time` 求值轨并写入参数 —— 预渲染模式的**唯一**状态更新路径。
    fn apply_prerender_frame(&mut self) -> Result<(), AnimationError> {
        let Some(id) = self.mode.open.clone() else {
            return Ok(());
        };
        let animation = find_animation(&self.animations, &id)?.clone();
        let track = ModeGuard::require_baked(&animation, &id)?.clone();

        let time = self.mode.time;
        let sample = self.mode.sample(&track, time);

        let model = self.doc.model().clone();
        // 先把参数重置到模型默认，再叠加轨 —— 否则上一次 seek 的残留会累积。
        self.doc.params_mut().reset(&model);
        for (id, value) in &sample.params {
            self.doc.params_mut().set_clamped(&model, id, *value);
        }
        Ok(())
    }

    // ------------------------------------------------------------ 查询

    fn animation_list(&mut self) -> Result<Value, AnimationError> {
        Ok(json!(
            self.animations
                .animations
                .iter()
                .map(|a| a.summary())
                .collect::<Vec<_>>()
        ))
    }

    fn animation_query(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let id = match param_str(params, "id").ok() {
            Some(id) => Some(id.to_string()),
            None => self.mode.open.clone(),
        };
        let detail = params
            .get("detail")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        match id {
            Some(id) => {
                let animation = find_animation(&self.animations, &id)?;
                if detail {
                    Ok(serde_json::to_value(animation)
                        .map_err(|e| AnimationError::InvalidParams(e.to_string()))?)
                } else {
                    Ok(serde_json::to_value(animation.summary())
                        .map_err(|e| AnimationError::InvalidParams(e.to_string()))?)
                }
            }
            None => self.animation_list(),
        }
    }

    fn animation_set_meta(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let id = self.resolve_id(params)?;
        let animation = find_animation_mut(&mut self.animations, &id)?;
        if let Some(name) = param_str(params, "name").ok() {
            animation.name = name.to_string();
        }
        if let Some(fps) = param_f32(params, "fps") {
            if !(fps.is_finite() && fps >= 1.0 && fps <= 240.0) {
                return Err(AnimationError::InvalidParams(format!(
                    "fps 应在 1..=240，实际 {fps}"
                )));
            }
            animation.fps = fps;
        }
        if let Some(duration) = param_f32(params, "duration") {
            animation.duration = duration.max(0.0);
        }
        if let Some(looping) = params.get("looping").and_then(|v| v.as_bool()) {
            animation.looping = looping;
        }
        if let Some(comment) = params.get("comment") {
            animation.comment = comment.as_str().map(|s| s.to_string());
        }
        let updated = animation.clone();
        self.encode_animations();
        Ok(serde_json::to_value(&updated)
            .map_err(|e| AnimationError::InvalidParams(e.to_string()))?)
    }

    fn animation_delete(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let id = self.resolve_id(params)?;
        if self.mode.open.as_deref() == Some(id.as_str()) {
            self.mode = ModeState::live();
        }
        if self.animations.remove(&id).is_none() {
            return Err(AnimationError::NotFound(id));
        }
        self.encode_animations();
        Ok(json!({ "id": id, "deleted": true }))
    }

    // ------------------------------------------------------------ 通道

    /// 目标成员在编辑期的存在性检查。
    ///
    /// 规范 §4.2：通道 `target` 的存在性**不做**强校验（允许先建通道后建参数，
    /// 也允许导入外部动画），但 `animation.channel.add` 时若 id 不存在会返回
    /// `-32602` 以免手滑。两处语义不同，不要合并。
    fn check_target_exists(&self, kind: ChannelKind, target: &str) -> Result<(), AnimationError> {
        let model = self.doc.model();
        let exists = match kind {
            ChannelKind::Parameter => model.parameters.iter().any(|p| p.id == target),
            ChannelKind::Visibility | ChannelKind::DrawOrder => {
                model.nodes.iter().any(|n| n.id == target)
            }
        };
        if exists {
            Ok(())
        } else {
            Err(AnimationError::InvalidParams(format!(
                "{} 不存在：{target}",
                kind.as_str()
            )))
        }
    }

    fn animation_channel_add(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let id = self.resolve_id(params)?;
        let kind_raw = param_str(params, "kind")
            .ok()
            .unwrap_or("parameter")
            .to_string();
        let kind = parse_channel_kind(&kind_raw)?;
        let target = param_str(params, "target")
            .ok()
            .map(|s| s.to_string())
            .ok_or(AnimationError::MissingId)?;
        self.check_target_exists(kind, &target)?;

        let animation = find_animation_mut(&mut self.animations, &id)?;
        let mut channel = Channel::new(kind, target.clone());
        if let Some(color) = param_str(params, "color").ok() {
            channel.color = Some(color.to_string());
        }
        if let Some(enabled) = params.get("enabled").and_then(|v| v.as_bool()) {
            channel.enabled = enabled;
        }
        animation.upsert_channel(channel);
        let count = animation.channels.len();
        self.encode_animations();
        Ok(json!({ "id": id, "target": target, "kind": kind_raw, "channels": count }))
    }

    fn animation_channel_remove(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let id = self.resolve_id(params)?;
        let kind_raw = param_str(params, "kind")
            .ok()
            .unwrap_or("parameter")
            .to_string();
        let kind = parse_channel_kind(&kind_raw)?;
        let target = param_str(params, "target")
            .ok()
            .map(|s| s.to_string())
            .ok_or(AnimationError::MissingId)?;

        let animation = find_animation_mut(&mut self.animations, &id)?;
        if !animation.remove_channel(kind, &target) {
            return Err(AnimationError::InvalidParams(format!(
                "通道不存在：{}/{target}",
                kind.as_str()
            )));
        }
        self.encode_animations();
        Ok(json!({ "id": id, "target": target, "removed": true }))
    }

    /// 定位一条通道（供 `key.*` 系列复用）。
    fn locate_channel(&mut self, params: &Value) -> Result<(Id, ChannelKind, Id), AnimationError> {
        let id = self.resolve_id(params)?;
        let kind_raw = param_str(params, "kind")
            .ok()
            .unwrap_or("parameter")
            .to_string();
        let kind = parse_channel_kind(&kind_raw)?;
        let target = param_str(params, "target")
            .ok()
            .map(|s| s.to_string())
            .ok_or(AnimationError::MissingId)?;
        Ok((id, kind, target))
    }

    fn animation_key_set(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let (id, kind, target) = self.locate_channel(params)?;
        let time = param_f32(params, "time")
            .ok_or_else(|| AnimationError::InvalidParams("animation.key.set 需要 time".into()))?;
        let value = param_f32(params, "value")
            .ok_or_else(|| AnimationError::InvalidParams("animation.key.set 需要 value".into()))?;
        let easing = match params.get("easing") {
            Some(v) => am_anim::easing_from_value(v).ok_or_else(|| {
                AnimationError::InvalidParams("easing 无法解析（见 §4.5）".into())
            })?,
            None => am_math::Easing::Linear,
        };

        let animation = find_animation_mut(&mut self.animations, &id)?;
        let channel = animation
            .channel_mut(kind, &target)
            .ok_or_else(|| AnimationError::InvalidParams(format!("通道不存在：{target}")))?;
        channel.insert_key(ChannelKey::with_easing(time, value, easing));
        let keys = channel.keys.len();
        self.encode_animations();
        self.mode.invalidate();
        Ok(json!({ "id": id, "target": target, "time": time, "value": value, "keys": keys }))
    }

    fn animation_key_remove(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let (id, kind, target) = self.locate_channel(params)?;
        let time = param_f32(params, "time").ok_or_else(|| {
            AnimationError::InvalidParams("animation.key.remove 需要 time".into())
        })?;
        let tolerance = param_f32(params, "tolerance").unwrap_or(1e-4);

        let animation = find_animation_mut(&mut self.animations, &id)?;
        let channel = animation
            .channel_mut(kind, &target)
            .ok_or_else(|| AnimationError::InvalidParams(format!("通道不存在：{target}")))?;
        if !channel.remove_key_at(time, tolerance) {
            return Err(AnimationError::InvalidParams(format!(
                "该时间点没有关键帧：{time}"
            )));
        }
        self.encode_animations();
        self.mode.invalidate();
        Ok(json!({ "id": id, "target": target, "removed": true }))
    }

    /// 移动关键帧：`from` → `to`（可同时改 `value`）。
    fn animation_key_move(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let (id, kind, target) = self.locate_channel(params)?;
        let from = param_f32(params, "from")
            .ok_or_else(|| AnimationError::InvalidParams("animation.key.move 需要 from".into()))?;
        let to = param_f32(params, "to")
            .ok_or_else(|| AnimationError::InvalidParams("animation.key.move 需要 to".into()))?;
        let tolerance = param_f32(params, "tolerance").unwrap_or(1e-4);

        let animation = find_animation_mut(&mut self.animations, &id)?;
        let channel = animation
            .channel_mut(kind, &target)
            .ok_or_else(|| AnimationError::InvalidParams(format!("通道不存在：{target}")))?;
        let index = channel
            .keys
            .iter()
            .position(|k| (k.time - from).abs() <= tolerance)
            .ok_or_else(|| AnimationError::InvalidParams(format!("该时间点没有关键帧：{from}")))?;

        let mut key = channel.keys.remove(index);
        key.time = to;
        if let Some(value) = param_f32(params, "value") {
            key.value = value;
        }
        channel.insert_key(key);
        self.encode_animations();
        self.mode.invalidate();
        Ok(json!({ "id": id, "target": target, "from": from, "to": to }))
    }

    fn animation_channel_set_easing(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let (id, kind, target) = self.locate_channel(params)?;
        let easing = match params.get("easing") {
            Some(v) => am_anim::easing_from_value(v).ok_or_else(|| {
                AnimationError::InvalidParams("easing 无法解析（见 §4.5）".into())
            })?,
            None => return Err(AnimationError::InvalidParams("需要 easing".into())),
        };
        let time = param_f32(params, "time");
        let tolerance = param_f32(params, "tolerance").unwrap_or(1e-4);

        let animation = find_animation_mut(&mut self.animations, &id)?;
        let channel = animation
            .channel_mut(kind, &target)
            .ok_or_else(|| AnimationError::InvalidParams(format!("通道不存在：{target}")))?;

        let mut updated = 0usize;
        match time {
            Some(time) => {
                if let Some(key) = channel
                    .keys
                    .iter_mut()
                    .find(|k| (k.time - time).abs() <= tolerance)
                {
                    key.easing = easing;
                    updated = 1;
                } else {
                    return Err(AnimationError::InvalidParams(format!(
                        "该时间点没有关键帧：{time}"
                    )));
                }
            }
            None => {
                for key in channel.keys.iter_mut() {
                    key.easing = easing.clone();
                    updated += 1;
                }
            }
        }
        self.encode_animations();
        self.mode.invalidate();
        Ok(json!({ "id": id, "target": target, "updated": updated }))
    }

    /// 整段替换一条通道的关键帧（编辑器曲线面板拖拽后一次性提交）。
    fn animation_curve_set(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let (id, kind, target) = self.locate_channel(params)?;
        let keys = params.get("keys").ok_or_else(|| {
            AnimationError::InvalidParams("animation.curve.set 需要 keys".into())
        })?;
        let keys: Vec<ChannelKey> = serde_json::from_value(keys.clone())
            .map_err(|e| AnimationError::InvalidParams(format!("keys 无法解析：{e}")))?;

        let animation = find_animation_mut(&mut self.animations, &id)?;
        let channel = animation
            .channel_mut(kind, &target)
            .ok_or_else(|| AnimationError::InvalidParams(format!("通道不存在：{target}")))?;
        channel.keys = keys;
        channel
            .keys
            .sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
        let count = channel.keys.len();
        self.encode_animations();
        self.mode.invalidate();
        Ok(json!({ "id": id, "target": target, "keys": count }))
    }

    // ------------------------------------------------------------ overlay

    fn animation_overlay_set(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let id = self.resolve_id(params)?;
        let animation = find_animation_mut(&mut self.animations, &id)?;

        match params.get("overlay") {
            Some(value) => {
                let overlay: Overlay = serde_json::from_value(value.clone()).map_err(|e| {
                    AnimationError::InvalidParams(format!("overlay 无法解析：{e}"))
                })?;
                animation.overlay = overlay;
            }
            None => {
                // 逐字段更新（编辑器只改一项时不必回传整棵）
                if let Some(motions) = params.get("motions") {
                    animation.overlay.motions =
                        serde_json::from_value(motions.clone()).map_err(|e| {
                            AnimationError::InvalidParams(format!("motions 无法解析：{e}"))
                        })?;
                }
                if let Some(expressions) = params.get("expressions") {
                    animation.overlay.expressions =
                        serde_json::from_value(expressions.clone()).map_err(|e| {
                            AnimationError::InvalidParams(format!("expressions 无法解析：{e}"))
                        })?;
                }
                if let Some(physics) = params.get("physics").and_then(|v| v.as_bool()) {
                    animation.overlay.physics = physics;
                }
                if let Some(auto) = params.get("auto_effects").and_then(|v| v.as_bool()) {
                    animation.overlay.auto_effects = auto;
                }
            }
        }
        let overlay = animation.overlay.clone();
        self.encode_animations();
        Ok(json!({ "id": id, "overlay": overlay }))
    }

    /// 增删 overlay 里的动作/表情引用（避免宿主整棵回传）。
    fn animation_set_param_ref(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let id = self.resolve_id(params)?;
        let animation = find_animation_mut(&mut self.animations, &id)?;

        let mut touched = 0usize;
        if let Some(motion_id) = param_str(params, "motion").ok() {
            let weight = param_f32(params, "weight").unwrap_or(1.0);
            let remove = params
                .get("remove")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            animation.overlay.motions.retain(|m| m.id != motion_id);
            if !remove {
                animation.overlay.motions.push(MotionRef {
                    id: motion_id.to_string(),
                    weight,
                });
            }
            touched += 1;
        }
        if let Some(expression_id) = param_str(params, "expression").ok() {
            let weight = param_f32(params, "weight").unwrap_or(1.0);
            let from = param_f32(params, "from").unwrap_or(0.0);
            let to = param_f32(params, "to").unwrap_or(0.0);
            let remove = params
                .get("remove")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            animation
                .overlay
                .expressions
                .retain(|e| e.id != expression_id);
            if !remove {
                animation.overlay.expressions.push(ExpressionRef {
                    id: expression_id.to_string(),
                    weight,
                    from,
                    to,
                });
            }
            touched += 1;
        }
        if touched == 0 {
            return Err(AnimationError::InvalidParams(
                "需要 motion 或 expression".into(),
            ));
        }
        let overlay = animation.overlay.clone();
        self.encode_animations();
        Ok(json!({ "id": id, "overlay": overlay }))
    }

    // ------------------------------------------------------------ 烘焙与求值

    /// 烘焙（§4.4）。烘焙**不**需要先打开动画。
    fn animation_bake(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let id = self.resolve_id(params)?;
        let animation = find_animation(&self.animations, &id)?.clone();

        let mut options = bake_options_from(params)?;
        // 没有显式指定就用「现在」；测试会显式传入以保证可复现。
        if options.baked_at.is_none() {
            options.baked_at = Some(now_unix_secs());
        }

        let model = self.doc.model().clone();
        let result = am_anim::bake(
            &animation,
            &model,
            &self.spec.settings,
            &self.spec.motions,
            &self.spec.expressions,
            &self.spec.physics,
            &options,
        )
        .map_err(AnimationError::from)?;

        // 写回轨与几何模式
        let animation = find_animation_mut(&mut self.animations, &id)?;
        animation.geometry = result.geometry;
        animation.track = Some(result.track.clone());
        self.encode_animations();
        self.mode.invalidate();

        // 打开的就是它 → 立刻按新轨刷新
        if self.mode.open.as_deref() == Some(id.as_str()) {
            self.apply_prerender_frame()?;
        }

        let track = &result.track;
        let mut value = json!({
            "id": id,
            "fps": track.fps,
            "frames": track.frames,
            "duration": track.duration,
            "channels": track.params.ids.len(),
            "points": track.params.data.len(),
            "bytes": track.bytes(),
            "geometry": result.geometry.as_str(),
            "baked_at": track.baked_at,
        });
        if let Some(warning) = &result.warning {
            value["warning"] = json!(warning);
            value["estimated_bytes"] = json!(result.estimated_bytes);
        }
        Ok(value)
    }

    /// 查询烘焙状态（不返回整条轨）。
    fn animation_baked(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let id = self.resolve_id(params)?;
        let animation = find_animation(&self.animations, &id)?;
        Ok(match animation.track.as_ref() {
            Some(track) => json!({
                "id": id,
                "baked": animation.is_baked(),
                "fps": track.fps,
                "frames": track.frames,
                "duration": track.duration,
                "baked_at": track.baked_at,
                "geometry": if track.geometry.is_some() { "snapshot" } else { "model_ref" },
                "bytes": track.bytes(),
            }),
            None => json!({
                "id": id,
                "baked": false,
                "fps": animation.fps,
                "frames": 0,
                "duration": animation.effective_duration(),
                "baked_at": 0,
                "geometry": animation.geometry.as_str(),
                "bytes": 0,
            }),
        })
    }

    /// 按时间求值（不改变会话状态）。
    ///
    /// 这是查看器与编辑器预览的**核心只读接口**：给定时间拿一帧结果。
    /// 与 `animation.seek` 的区别是它不写参数、不动时钟，适合批量采样与离屏渲染。
    fn animation_sample(&mut self, params: &Value) -> Result<Value, AnimationError> {
        let id = self.resolve_id(params)?;
        let animation = find_animation(&self.animations, &id)?.clone();
        let track = ModeGuard::require_baked(&animation, &id)?;

        let time = param_f32(params, "time").unwrap_or(self.mode.time);
        let sample = track.sample(time);
        let coordinate =
            track
                .range
                .sample_coordinate(time, track.fps, self.looping_of(&animation));

        let mut value = json!({
            "id": id,
            "time": time,
            "frame": coordinate.floor() as i64,
            "params": sample.params,
            "structural": {
                "visibility": sample.visibility,
                "draw_order": sample.draw_order,
            },
        });
        // 几何快照：只在请求且存在时返回（可能很大）
        let want_geometry = params
            .get("geometry")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if want_geometry {
            if let Some(geometry) = track.geometry.as_ref() {
                value["geometry"] = json!({
                    "mesh_ids": geometry.mesh_ids,
                    "offsets": geometry.meshes.offsets,
                });
            } else {
                value["geometry"] = Value::Null;
            }
        }
        Ok(value)
    }
}

/// 当前 Unix 秒。烘焙时间戳只是信息字段，不参与求值。
fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 供测试与宿主复用：把字符串解析成 `GeometryMode`。
pub fn parse_geometry_mode(raw: &str) -> Option<GeometryMode> {
    match raw {
        "model_ref" => Some(GeometryMode::ModelRef),
        "snapshot" => Some(GeometryMode::Snapshot),
        _ => None,
    }
}

/// `animation.bake` 的选项解析（独立出来便于单测）。
pub fn bake_options_from(params: &Value) -> Result<BakeOptions, AnimationError> {
    let mut options = BakeOptions {
        fps: param_f32(params, "fps"),
        baked_at: param_f64(params, "baked_at").map(|v| v as u64),
        ..Default::default()
    };
    if let Some(range) = params.get("range") {
        let start = range.get("start").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let end = range.get("end").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        options.range = Some(Range::new(start, end));
    }
    if let Some(include) = params.get("include") {
        if let Some(v) = include.get("physics").and_then(|v| v.as_bool()) {
            options.include_physics = Some(v);
        }
        if let Some(v) = include.get("auto_effects").and_then(|v| v.as_bool()) {
            options.include_auto_effects = Some(v);
        }
        if let Some(v) = include.get("overlays").and_then(|v| v.as_bool()) {
            options.include_overlays = Some(v);
        }
    }
    if let Some(geometry) = param_str(params, "geometry").ok() {
        options.geometry = Some(parse_geometry_mode(geometry).ok_or_else(|| {
            AnimationError::InvalidParams(format!(
                "geometry 只能是 model_ref 或 snapshot，实际 {geometry}"
            ))
        })?);
    }
    Ok(options)
}
