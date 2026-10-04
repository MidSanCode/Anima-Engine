//! 模式状态机（规范 §2、§3）。
//!
//! 引擎有两条**互斥**的运行路径：
//!
//! | 模式 | 时钟 | 参数来源 | `runtime.advance` |
//! | --- | --- | --- | --- |
//! | `live` | 由宿主 `dt` 推进 | 动作 / 物理 / 宿主写入 | 允许 |
//! | `prerender` | 由轨 `seek` 决定 | 烘焙轨（只读） | `-32010` |
//!
//! 互斥是刻意的：预渲染模式下参数由轨决定，如果还允许实时推进，
//! 「预渲染看起来就是刚才实时看到的那一段」这条正确性锚点就无从谈起。
//!
//! ## 为什么状态机单独一个模块
//!
//! `Session` 的字段很多，模式判断散落在 `dispatch` 里极易漏判 —— 而漏判的后果
//! 是**静默的错误结果**（预渲染里参数被实时写入污染），不是崩溃。集中在这里，
//! 配合入口处的 [`ModeGuard`]，可以让「每条会改状态的路径都必须过闸」。

use am_anim::{AmAnimation, AnimationSet, ChannelKind, Track, TrackSample};
use am_model::Id;
use serde_json::json;

/// 错误码（规范 §9）。
pub mod codes {
    /// 模式冲突：当前模式不允许该操作。
    pub const MODE_CONFLICT: i32 = -32010;
    /// 尚未烘焙：需要先 `animation.bake`。
    pub const NOT_BAKED: i32 = -32011;
    /// 动画不存在。
    pub const ANIMATION_NOT_FOUND: i32 = -32012;
}

/// 运行模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// 实时模式：状态由动作 / 物理 / 宿主写入驱动。
    #[default]
    Live,
    /// 预渲染模式：状态由已烘焙的轨驱动。
    Prerender,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Live => "live",
            Mode::Prerender => "prerender",
        }
    }
}

/// 模式的**全部**可变状态。
#[derive(Debug, Clone, Default)]
pub struct ModeState {
    pub mode: Mode,
    /// 当前打开的动画 id（`prerender` 时必定有值）。
    pub open: Option<Id>,
    /// 预渲染时钟（秒）。
    pub time: f32,
    pub speed: f32,
    /// 播放下标是否在走。
    pub playing: bool,
    /// 循环覆盖（`None` 表示用动画自身的 `looping`）。
    pub looping: Option<bool>,
    /// 淡入剩余时长。
    pub fade_in: f32,
    /// 最近一次 `seek` 得到的结果缓存（避免每帧重复求值）。
    cache: Option<(f32, Track)>,
}

impl ModeState {
    pub fn live() -> Self {
        Self { mode: Mode::Live, speed: 1.0, ..Default::default() }
    }

    pub fn is_prerender(&self) -> bool {
        self.mode == Mode::Prerender
    }

    /// 求当前帧（带一层微小缓存，避免同一时间点重复求值）。
    pub fn sample(&mut self, track: &Track, time: f32) -> TrackSample {
        let hit = match &self.cache {
            Some((cached_time, cached_track)) => {
                *cached_time == time && cached_track.frames == track.frames
            }
            None => false,
        };
        if !hit {
            self.cache = Some((time, track.clone()));
        }
        track.sample(time)
    }

    pub fn invalidate(&mut self) {
        self.cache = None;
    }
}

/// 模式状态机的错误。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AnimationError {
    #[error("动画不存在：{0}")]
    NotFound(Id),
    #[error("当前是预渲染模式，{0} 不可用；请先 animation.close")]
    ModeConflict(String),
    #[error("动画尚未烘焙：{0}；请先 animation.bake")]
    NotBaked(Id),
    #[error("缺少 id")]
    MissingId,
    #[error("id 非法：{0}（只允许小写字母、数字、_ 和 -）")]
    BadId(String),
    #[error("动画已存在：{0}")]
    AlreadyExists(Id),
    #[error("参数非法：{0}")]
    InvalidParams(String),
    #[error("烘焙失败：{0}")]
    Bake(String),
}

impl AnimationError {
    pub fn code(&self) -> i32 {
        match self {
            AnimationError::NotFound(_) => codes::ANIMATION_NOT_FOUND,
            AnimationError::ModeConflict(_) => codes::MODE_CONFLICT,
            AnimationError::NotBaked(_) => codes::NOT_BAKED,
            AnimationError::MissingId
            | AnimationError::BadId(_)
            | AnimationError::AlreadyExists(_)
            | AnimationError::InvalidParams(_)
            | AnimationError::Bake(_) => crate::codes::INVALID_PARAMS,
        }
    }

    /// 规范 §9 的错误形状：`{code, message, hint?}`。
    pub fn to_value(&self) -> serde_json::Value {
        let mut value = json!({
            "code": self.code(),
            "message": self.to_string(),
        });
        if let Some(hint) = self.hint() {
            value["hint"] = json!(hint);
        }
        value
    }

    /// 可执行的修复建议（编辑器直接显示给用户）。
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            AnimationError::ModeConflict(_) => Some("animation.close"),
            AnimationError::NotBaked(_) => Some("animation.bake"),
            AnimationError::NotFound(_) => Some("animation.list"),
            _ => None,
        }
    }
}

impl From<am_anim::BakeError> for AnimationError {
    fn from(err: am_anim::BakeError) -> Self {
        AnimationError::Bake(err.to_string())
    }
}

// ------------------------------------------------------------------ 闸门

/// 模式闸门：进入 `prerender` 后，实时写操作一律拒绝。
///
/// 规范 §2.1 规则 3 例外：调试用的 `runtime.set_param {override: true}` 仍可用，
/// 因为它是「临时覆盖」而不是「改状态」—— 但**查看器不使用**它。
pub struct ModeGuard;

impl ModeGuard {
    /// 实时推进 / 写入参数前的检查。
    pub fn require_live(state: &ModeState, what: &str) -> Result<(), AnimationError> {
        if state.is_prerender() {
            return Err(AnimationError::ModeConflict(what.to_string()));
        }
        Ok(())
    }

    /// 需要已烘焙的动画。
    pub fn require_baked<'a>(
        animation: &'a AmAnimation,
        id: &Id,
    ) -> Result<&'a Track, AnimationError> {
        if !animation.is_baked() {
            return Err(AnimationError::NotBaked(id.clone()));
        }
        animation
            .track
            .as_ref()
            .ok_or_else(|| AnimationError::NotBaked(id.clone()))
    }
}

/// 取一个动画（不存在则报错）。动画集合的**唯一**查找入口。
pub fn find<'a>(set: &'a AnimationSet, id: &str) -> Result<&'a AmAnimation, AnimationError> {
    set.by_id(id).ok_or_else(|| AnimationError::NotFound(id.to_string()))
}

pub fn find_mut<'a>(
    set: &'a mut AnimationSet,
    id: &str,
) -> Result<&'a mut AmAnimation, AnimationError> {
    set.by_id_mut(id).ok_or_else(|| AnimationError::NotFound(id.to_string()))
}

/// 校验 id 是否可用于新建（存在性 + 字符合法）。
pub fn validate_new_id(set: &AnimationSet, id: &str) -> Result<(), AnimationError> {
    if id.is_empty() {
        return Err(AnimationError::MissingId);
    }
    if !am_anim::is_valid_animation_id(id) {
        return Err(AnimationError::BadId(id.to_string()));
    }
    if set.contains(id) {
        return Err(AnimationError::AlreadyExists(id.to_string()));
    }
    Ok(())
}

/// 通道种类解析（宿主传字符串）。
pub fn parse_channel_kind(raw: &str) -> Result<ChannelKind, AnimationError> {
    match raw {
        "parameter" => Ok(ChannelKind::Parameter),
        "visibility" => Ok(ChannelKind::Visibility),
        "draw_order" => Ok(ChannelKind::DrawOrder),
        other => Err(AnimationError::InvalidParams(format!("未知通道种类：{other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_anim::{AmAnimation, Range};

    fn baked_animation() -> AmAnimation {
        let mut a = AmAnimation::new("idle", "Idle");
        a.duration = 1.0;
        let mut track = Track {
            fps: 30.0,
            frames: 2,
            range: Range::new(0.0, 1.0 / 30.0),
            ..Default::default()
        };
        track.params = am_anim::TrackBlock::new(vec!["p".to_string()]);
        track.params.normalize(2);
        track.params.write(0, 0, 0.0);
        track.params.write(1, 0, 10.0);
        a.track = Some(track);
        a
    }

    #[test]
    fn default_mode_is_live() {
        let state = ModeState::live();
        assert_eq!(state.mode, Mode::Live);
        assert!(!state.is_prerender());
        assert_eq!(state.speed, 1.0);
        assert_eq!(Mode::default(), Mode::Live);
    }

    #[test]
    fn live_guard_allows_in_live_mode() {
        let state = ModeState::live();
        assert!(ModeGuard::require_live(&state, "runtime.advance").is_ok());
    }

    #[test]
    fn live_guard_blocks_in_prerender_mode() {
        let state = ModeState { mode: Mode::Prerender, ..ModeState::live() };
        let err = ModeGuard::require_live(&state, "runtime.advance").unwrap_err();
        assert_eq!(err.code(), codes::MODE_CONFLICT);
        assert_eq!(err.hint(), Some("animation.close"));
    }

    #[test]
    fn unbaked_animation_is_rejected() {
        let a = AmAnimation::new("idle", "Idle");
        let err = ModeGuard::require_baked(&a, &"idle".to_string()).unwrap_err();
        assert_eq!(err.code(), codes::NOT_BAKED);
        assert_eq!(err.hint(), Some("animation.bake"));
    }

    #[test]
    fn baked_animation_passes_guard() {
        let a = baked_animation();
        let track = ModeGuard::require_baked(&a, &"idle".to_string()).unwrap();
        assert_eq!(track.frames, 2);
    }

    #[test]
    fn find_reports_not_found_with_correct_code() {
        let set = AnimationSet::default();
        let err = find(&set, "ghost").unwrap_err();
        assert_eq!(err.code(), codes::ANIMATION_NOT_FOUND);
        assert_eq!(err.hint(), Some("animation.list"));
    }

    #[test]
    fn validate_new_id_rejects_duplicates_and_bad_chars() {
        let mut set = AnimationSet::default();
        set.upsert(AmAnimation::new("idle", "Idle"));

        assert_eq!(
            validate_new_id(&set, "idle").unwrap_err().code(),
            crate::codes::INVALID_PARAMS,
            "重复 id 是参数错误"
        );
        assert!(matches!(
            validate_new_id(&set, "Bad Id").unwrap_err(),
            AnimationError::BadId(_)
        ));
        assert!(matches!(
            validate_new_id(&set, "").unwrap_err(),
            AnimationError::MissingId
        ));
        assert!(validate_new_id(&set, "walk").is_ok());
    }

    #[test]
    fn channel_kind_parsing_covers_all_variants() {
        assert_eq!(parse_channel_kind("parameter").unwrap(), ChannelKind::Parameter);
        assert_eq!(parse_channel_kind("visibility").unwrap(), ChannelKind::Visibility);
        assert_eq!(parse_channel_kind("draw_order").unwrap(), ChannelKind::DrawOrder);
        assert!(parse_channel_kind("bogus").is_err());
    }

    #[test]
    fn error_codes_match_spec() {
        // 规范 §9 的三个新错误码
        assert_eq!(codes::MODE_CONFLICT, -32010);
        assert_eq!(codes::NOT_BAKED, -32011);
        assert_eq!(codes::ANIMATION_NOT_FOUND, -32012);
    }

    #[test]
    fn mode_conflict_error_carries_hint() {
        let err = AnimationError::ModeConflict("runtime.advance".into());
        let value = err.to_value();
        assert_eq!(value["code"], codes::MODE_CONFLICT);
        assert_eq!(value["hint"], "animation.close");
        assert!(value["message"].as_str().unwrap().contains("runtime.advance"));
    }

    #[test]
    fn sample_cache_returns_consistent_values() {
        let a = baked_animation();
        let track = a.track.clone().unwrap();
        let mut state = ModeState::live();
        let first = state.sample(&track, 0.0);
        let second = state.sample(&track, 0.0);
        assert_eq!(first.params["p"], second.params["p"], "缓存命中必须给同样结果");
        state.invalidate();
        assert_eq!(state.sample(&track, 0.0).params["p"], 0.0);
    }
}
