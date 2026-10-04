//! `am-anim`：预渲染动画（规范 `docs/animation-mode.md`）。
//!
//! 这一 crate 只做**纯数据 + 纯函数**：动画定义（§4.1）、通道（§4.2）、
//! 烘焙轨（§4.3）与求值算法。模式状态机在 `am-core`，因为它是会话状态。
//!
//! ```text
//! AmAnimation ──► channels（§4.2）──► sample(t)        编辑期求值
//!             └─► track（§4.3）   ──► sample_track(t)  播放期求值（烘焙后）
//! ```
//!
//! 求值算法是**冻结契约**（§4.3）：宿主与引擎必须逐位一致，所以这里不做任何
//! 「优化」—— 不预排序、不缓存、不重采样到最近关键帧。

use am_math::{Easing, Vec2};
use am_model::{Id, MotionKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 动画文件格式版本。
pub const ANIMATION_FORMAT_VERSION: u32 = 1;

/// 动画 id 的合法字符集（与文件名一致，§4.1）。
pub fn is_valid_animation_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// 把任意字符串规范化成合法动画 id（编辑器「按名字生成 id」用）。
pub fn normalize_animation_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
            out.push(c);
        } else if c.is_ascii_whitespace() {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

// ---------------------------------------------------------------- 通道

/// 通道类型（§4.2）。枚举**只增不改**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    /// 驱动一个模型参数。
    #[default]
    Parameter,
    /// 驱动节点的可见性（实时模式没有这条路）。
    Visibility,
    /// 驱动节点的绘制顺序。
    DrawOrder,
}

impl ChannelKind {
    /// 结构通道：写入的是「节点结构」而不是参数。
    pub fn is_structural(self) -> bool {
        !matches!(self, ChannelKind::Parameter)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ChannelKind::Parameter => "parameter",
            ChannelKind::Visibility => "visibility",
            ChannelKind::DrawOrder => "draw_order",
        }
    }
}

/// 一个关键帧（与 `MotionKey` 同构，复用 `am_math::Easing`）。
pub type ChannelKey = MotionKey;

/// 一条通道（§4.2）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    #[serde(default)]
    pub kind: ChannelKind,
    /// `parameter` 时是参数 id；结构通道时是节点 id。
    pub target: Id,
    /// 关键帧，按 `time` 升序，同时间点覆盖。
    #[serde(default)]
    pub keys: Vec<ChannelKey>,
    /// `false` 时该通道不参与求值与烘焙（编辑器「临时静音」）。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `#RRGGBB`，仅曲线编辑器显示用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Channel {
    pub fn new(kind: ChannelKind, target: impl Into<Id>) -> Self {
        Self {
            kind,
            target: target.into(),
            keys: Vec::new(),
            enabled: true,
            color: None,
            comment: None,
        }
    }

    /// 参数通道的便捷构造。
    pub fn parameter(target: impl Into<Id>) -> Self {
        Self::new(ChannelKind::Parameter, target)
    }

    /// 插入关键帧并保持时间升序（同时间点覆盖）。
    pub fn insert_key(&mut self, key: ChannelKey) {
        match self.keys.binary_search_by(|k| {
            k.time.partial_cmp(&key.time).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Ok(idx) => self.keys[idx] = key,
            Err(idx) => self.keys.insert(idx, key),
        }
    }

    pub fn remove_key_at(&mut self, time: f32, tolerance: f32) -> bool {
        match self.keys.iter().position(|k| (k.time - time).abs() <= tolerance) {
            Some(idx) => {
                self.keys.remove(idx);
                true
            }
            None => false,
        }
    }

    pub fn duration(&self) -> f32 {
        self.keys.last().map(|k| k.time).unwrap_or(0.0)
    }

    pub fn is_effectively_enabled(&self) -> bool {
        self.enabled
    }

    /// 通道求值（编辑期，未烘焙时用）。
    ///
    /// 语义与 `am_model::MotionCurve::sample` **完全一致**（同一套缓动、
    /// 同一套端点钳制），否则编辑器预览与实时播放会对不上。
    pub fn sample(&self, time: f32) -> Option<f32> {
        if !self.enabled || self.keys.is_empty() {
            return None;
        }
        if time <= self.keys[0].time {
            return Some(self.keys[0].value);
        }
        let last = self.keys.last()?;
        if time >= last.time {
            return Some(last.value);
        }
        let idx = match self
            .keys
            .binary_search_by(|k| k.time.partial_cmp(&time).unwrap_or(std::cmp::Ordering::Equal))
        {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let a = &self.keys[idx];
        let b = &self.keys[idx + 1];
        let span = b.time - a.time;
        if span.abs() <= f32::EPSILON {
            return Some(b.value);
        }
        let t = ((time - a.time) / span).clamp(0.0, 1.0);
        Some(a.value + (b.value - a.value) * a.easing.eval(t))
    }
}

// ---------------------------------------------------------------- overlay

/// 叠加动作引用（§4.4 ②）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotionRef {
    pub id: Id,
    #[serde(default = "default_one")]
    pub weight: f32,
}

fn default_one() -> f32 {
    1.0
}

/// 叠加表情引用（§4.4 ④）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpressionRef {
    pub id: Id,
    #[serde(default = "default_one")]
    pub weight: f32,
    /// 生效起点（秒）。`to <= from` 时整段生效。
    #[serde(default)]
    pub from: f32,
    #[serde(default)]
    pub to: f32,
}

impl ExpressionRef {
    /// 是否限定了生效区间。
    pub fn is_bounded(&self) -> bool {
        self.to > self.from
    }

    /// 区间内的淡入权重（与 `Expression::fade_in/out` 同语义）。
    pub fn fade_weight(&self, time: f32) -> f32 {
        if !self.is_bounded() {
            return 1.0;
        }
        if time < self.from || time > self.to {
            return 0.0;
        }
        let span = self.to - self.from;
        if span <= f32::EPSILON {
            return 1.0;
        }
        ((time - self.from) / span).clamp(0.0, 1.0)
    }
}

/// 烘焙源（§4.4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Overlay {
    #[serde(default)]
    pub motions: Vec<MotionRef>,
    #[serde(default)]
    pub expressions: Vec<ExpressionRef>,
    #[serde(default)]
    pub physics: bool,
    #[serde(default)]
    pub auto_effects: bool,
}

impl Default for Overlay {
    fn default() -> Self {
        Self { motions: Vec::new(), expressions: Vec::new(), physics: false, auto_effects: false }
    }
}

// ---------------------------------------------------------------- 区间

/// 有效区间（§4.6）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Range {
    #[serde(default)]
    pub start: f32,
    #[serde(default)]
    pub end: f32,
}

impl Range {
    pub fn new(start: f32, end: f32) -> Self {
        Self { start, end }
    }

    /// 区间长度（负值归零）。
    pub fn span(&self) -> f32 {
        (self.end - self.start).max(0.0)
    }

    /// 采样帧数（§4.3 冻结公式）：
    /// `frames = floor((end - start) * fps + 1e-6) + 1`
    pub fn frame_count(&self, fps: f32) -> u32 {
        if fps <= 0.0 || !fps.is_finite() {
            return 0;
        }
        let raw = (self.span() * fps + 1e-6).floor();
        if raw < 0.0 {
            0
        } else {
            raw as u32 + 1
        }
    }

    /// 把绝对时间映射为区间内的采样坐标 `x = u * fps`（§4.6 循环在时间域做）。
    pub fn sample_coordinate(&self, time: f32, fps: f32, looping: bool) -> f32 {
        let span = self.span();
        if span <= f32::EPSILON {
            return 0.0;
        }
        let mut local = time - self.start;
        if looping {
            local %= span;
            if local < 0.0 {
                local += span;
            }
        } else {
            local = local.clamp(0.0, span);
        }
        local * fps
    }
}

// ---------------------------------------------------------------- 烘焙轨

/// `ids` + 行优先扁平数组（§4.3）。
///
/// 用扁平数组而不是「每帧一个对象」是冻结的设计：体积小一个数量级，
/// 解析线性，且宿主能直接拿 `Float32List`/`TypedArray` 承接。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TrackBlock {
    /// 列的顺序：稳定、去重、升序。
    #[serde(default)]
    pub ids: Vec<Id>,
    /// 行优先 `frame * ids.len() + column`。
    #[serde(default)]
    pub data: Vec<f32>,
}

impl TrackBlock {
    pub fn new(ids: Vec<Id>) -> Self {
        Self { ids, data: Vec::new() }
    }

    pub fn columns(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn column_of(&self, id: &str) -> Option<usize> {
        self.ids.iter().position(|x| x == id)
    }

    /// 写入 `frame` 行、`column` 列。越界写被忽略（不 panic）。
    pub fn write(&mut self, frame: usize, column: usize, value: f32) {
        if column >= self.ids.len() {
            return;
        }
        let index = frame * self.ids.len() + column;
        if index >= self.data.len() {
            self.data.resize(index + 1, 0.0);
        }
        self.data[index] = value;
    }

    /// 读取 `frame` 行、`column` 列；越界返回 0（不 panic，坏数据不崩）。
    pub fn get(&self, frame: usize, column: usize) -> f32 {
        if column >= self.ids.len() {
            return 0.0;
        }
        self.data.get(frame * self.ids.len() + column).copied().unwrap_or(0.0)
    }

    /// 把长度补齐到 `frames * columns`，保证行优先布局自洽。
    pub fn normalize(&mut self, frames: usize) {
        self.data.resize(frames * self.ids.len(), 0.0);
    }
}

/// 求值结果：参数值（§4.3 的 lerp 语义）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackSample {
    /// 参数 id → 值。
    pub params: BTreeMap<Id, f32>,
    /// 节点 id → 可见性。
    pub visibility: BTreeMap<Id, bool>,
    /// 节点 id → 绘制顺序。
    pub draw_order: BTreeMap<Id, i32>,
}

/// 逐帧几何快照（§4.7，`geometry = "snapshot"` 时才有）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GeometryTrack {
    /// 绘制对象 id，稳定升序。
    #[serde(default)]
    pub mesh_ids: Vec<Id>,
    /// 变形器控制点：`frames * points * 2` 个 `f32`，frame-major。
    #[serde(default)]
    pub deformers: DeformerTrack,
    /// 绘制对象顶点。
    #[serde(default)]
    pub meshes: MeshTrack,
}

/// 变形器控制点轨。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DeformerTrack {
    #[serde(default)]
    pub ids: Vec<Id>,
    /// 每个变形器的行/列数（用于还原网格）。
    #[serde(default)]
    pub rows: Vec<u32>,
    #[serde(default)]
    pub cols: Vec<u32>,
    #[serde(default)]
    pub data: Vec<f32>,
}

/// 绘制对象几何轨。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MeshTrack {
    /// frame-major，按 `mesh_ids` 顺序拼接：`frames * (Σ顶点数) * 2`。
    #[serde(default)]
    pub vertices: Vec<f32>,
    /// 每个 mesh 的顶点起始下标，长度 = `mesh_ids.len() + 1`。
    #[serde(default)]
    pub offsets: Vec<u32>,
    /// `frames * mesh_ids.len()` 的不透明度。
    #[serde(default)]
    pub opacity: Vec<f32>,
}

/// 几何模式（§4.7）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryMode {
    /// 轨只存表演，几何实时从模型求值。
    #[default]
    ModelRef,
    /// 轨额外存每帧几何；改模型不影响已烘焙结果。
    Snapshot,
}

impl GeometryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            GeometryMode::ModelRef => "model_ref",
            GeometryMode::Snapshot => "snapshot",
        }
    }
}

/// 烘焙产物（§4.3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub fps: f32,
    pub frames: u32,
    #[serde(default)]
    pub duration: f32,
    /// Unix 秒（仅信息，不参与求值）。
    #[serde(default)]
    pub baked_at: u64,
    #[serde(default)]
    pub params: TrackBlock,
    #[serde(default)]
    pub visibility: TrackBlock,
    #[serde(default)]
    pub draw_order: TrackBlock,
    /// 仅 `geometry = "snapshot"` 时存在（§4.7）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<GeometryTrack>,
    /// 采样时所用的区间，用于 `seek` 复现坐标。
    #[serde(default)]
    pub range: Range,
    #[serde(default)]
    pub looping: bool,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            fps: 30.0,
            frames: 0,
            duration: 0.0,
            baked_at: 0,
            params: TrackBlock::default(),
            visibility: TrackBlock::default(),
            draw_order: TrackBlock::default(),
            geometry: None,
            range: Range::default(),
            looping: false,
        }
    }
}

impl Track {
    /// 轨的总字节数（估算，供 `diagnostics.stats` 与体积预警用）。
    pub fn bytes(&self) -> usize {
        let f32s = self.params.data.len()
            + self.visibility.data.len()
            + self.draw_order.data.len()
            + self
                .geometry
                .as_ref()
                .map(|g| {
                    g.deformers.data.len() + g.meshes.vertices.len() + g.meshes.opacity.len()
                })
                .unwrap_or(0);
        f32s * std::mem::size_of::<f32>()
    }

    /// 轨求值（§4.3 **冻结算法**）。
    ///
    /// ```text
    /// u = clamp(t - range.start, 0, range.end - range.start)   // 循环在 §4.6
    /// x = u * fps;  i = floor(x);  f = x - i
    /// i >= frames-1 → 取末帧（不外推）
    /// 参数：a + (b-a)*f       结构通道：四舍五入到最近帧，不插值
    /// ```
    pub fn sample(&self, time: f32) -> TrackSample {
        let mut out = TrackSample::default();
        if self.frames == 0 {
            return out;
        }
        let x = self.range.sample_coordinate(time, self.fps, self.looping);
        let i = x.floor();
        let f = x - i;
        let last = self.frames as i64 - 1;

        // 参数通道：一次查找 + 一次 lerp。
        for (c, id) in self.params.ids.iter().enumerate() {
            let value = if i >= last as f32 {
                self.params.get(last as usize, c)
            } else if i < 0.0 {
                self.params.get(0, c)
            } else {
                let a = self.params.get(i as usize, c);
                let b = self.params.get(i as usize + 1, c);
                a + (b - a) * f
            };
            out.params.insert(id.clone(), value);
        }

        // 结构通道：阶跃语义 —— 先四舍五入到最近帧，再钳，然后取整数原值。
        let step_frame = ((i + if f >= 0.5 { 1.0 } else { 0.0 }) as i64).clamp(0, last) as usize;
        for (c, id) in self.visibility.ids.iter().enumerate() {
            out.visibility.insert(id.clone(), self.visibility.get(step_frame, c) >= 0.5);
        }
        for (c, id) in self.draw_order.ids.iter().enumerate() {
            out.draw_order.insert(id.clone(), self.draw_order.get(step_frame, c).round() as i32);
        }
        out
    }
}

// ---------------------------------------------------------------- 顶层

/// 一段动画（§4.1）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AmAnimation {
    #[serde(default = "default_version")]
    pub version: u32,
    pub id: Id,
    pub name: String,
    /// 烘焙帧率，整数 1..240。
    #[serde(default = "default_fps")]
    pub fps: f32,
    /// 秒；0 表示由最长通道推断。
    #[serde(default)]
    pub duration: f32,
    #[serde(default, rename = "looping")]
    pub looping: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    #[serde(default)]
    pub overlay: Overlay,
    #[serde(default)]
    pub channels: Vec<Channel>,
    /// 烘焙产物；未烘焙时缺省。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<Track>,
    /// 几何模式（§4.7）。烘焙时写入轨，这里记录用户的选择。
    #[serde(default)]
    pub geometry: GeometryMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

fn default_version() -> u32 {
    ANIMATION_FORMAT_VERSION
}

fn default_fps() -> f32 {
    30.0
}

impl AmAnimation {
    pub fn new(id: impl Into<Id>, name: impl Into<String>) -> Self {
        Self {
            version: ANIMATION_FORMAT_VERSION,
            id: id.into(),
            name: name.into(),
            fps: 30.0,
            duration: 0.0,
            looping: false,
            range: None,
            overlay: Overlay::default(),
            channels: Vec::new(),
            track: None,
            geometry: GeometryMode::ModelRef,
            comment: None,
        }
    }

    /// 最长通道时间（`duration` 为 0 时用它推断）。
    pub fn channels_duration(&self) -> f32 {
        self.channels.iter().map(Channel::duration).fold(0.0, f32::max)
    }

    /// 实际时长。
    pub fn effective_duration(&self) -> f32 {
        if self.duration > 0.0 {
            self.duration
        } else {
            let from_channels = self.channels_duration();
            if from_channels > 0.0 {
                from_channels
            } else {
                self.track.as_ref().map(|t| t.duration).unwrap_or(0.0)
            }
        }
    }

    /// 有效区间：显式给出则使用，否则推断 `[0, duration]`。
    pub fn effective_range(&self) -> Range {
        match self.range {
            Some(r) if r.span() > f32::EPSILON => r,
            _ => Range::new(0.0, self.effective_duration()),
        }
    }

    /// 是否已烘焙且有可用帧。
    pub fn is_baked(&self) -> bool {
        self.track.as_ref().map(|t| t.frames > 0).unwrap_or(false)
    }

    pub fn channel(&self, kind: ChannelKind, target: &str) -> Option<&Channel> {
        self.channels.iter().find(|c| c.kind == kind && c.target == target)
    }

    pub fn channel_mut(&mut self, kind: ChannelKind, target: &str) -> Option<&mut Channel> {
        self.channels.iter_mut().find(|c| c.kind == kind && c.target == target)
    }

    /// 加入通道（同 kind+target 覆盖）。
    pub fn upsert_channel(&mut self, channel: Channel) {
        match self
            .channels
            .iter_mut()
            .find(|c| c.kind == channel.kind && c.target == channel.target)
        {
            Some(slot) => *slot = channel,
            None => self.channels.push(channel),
        }
    }

    pub fn remove_channel(&mut self, kind: ChannelKind, target: &str) -> bool {
        match self
            .channels
            .iter()
            .position(|c| c.kind == kind && c.target == target)
        {
            Some(idx) => {
                self.channels.remove(idx);
                true
            }
            None => false,
        }
    }

    /// 编辑期求值：未烘焙时也能预览（§4.2 的曲线 + §4.6 的循环）。
    pub fn sample_channels(&self, time: f32) -> TrackSample {
        let mut out = TrackSample::default();
        let range = self.effective_range();
        let x = range.sample_coordinate(time, self.fps, self.looping);
        let span = range.span();
        let local = if span <= f32::EPSILON {
            range.start
        } else {
            range.start + x / self.fps.max(1e-6)
        };

        for channel in &self.channels {
            if !channel.is_effectively_enabled() {
                continue;
            }
            let Some(value) = channel.sample(local) else {
                continue;
            };
            match channel.kind {
                ChannelKind::Parameter => {
                    out.params.insert(channel.target.clone(), value);
                }
                ChannelKind::Visibility => {
                    out.visibility.insert(channel.target.clone(), value >= 0.5);
                }
                ChannelKind::DrawOrder => {
                    out.draw_order.insert(channel.target.clone(), value.round() as i32);
                }
            }
        }
        out
    }
}

/// 一段动画的摘要（`animation.list` 用）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationSummary {
    pub id: Id,
    pub name: String,
    pub duration: f32,
    pub fps: f32,
    pub channels: usize,
    pub baked: bool,
    pub baked_at: u64,
}

impl AmAnimation {
    pub fn summary(&self) -> AnimationSummary {
        AnimationSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            duration: self.effective_duration(),
            fps: self.fps,
            channels: self.channels.len(),
            baked: self.is_baked(),
            baked_at: self.track.as_ref().map(|t| t.baked_at).unwrap_or(0),
        }
    }
}

/// 动画集合（一个工程里的全部动画）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnimationSet {
    #[serde(default)]
    pub animations: Vec<AmAnimation>,
}

impl AnimationSet {
    pub fn new(animations: Vec<AmAnimation>) -> Self {
        Self { animations }
    }

    pub fn is_empty(&self) -> bool {
        self.animations.is_empty()
    }

    pub fn len(&self) -> usize {
        self.animations.len()
    }

    pub fn by_id(&self, id: &str) -> Option<&AmAnimation> {
        self.animations.iter().find(|a| a.id == id)
    }

    pub fn by_id_mut(&mut self, id: &str) -> Option<&mut AmAnimation> {
        self.animations.iter_mut().find(|a| a.id == id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.by_id(id).is_some()
    }

    /// 加入（同 id 覆盖）。
    pub fn upsert(&mut self, animation: AmAnimation) {
        match self.by_id_mut(&animation.id) {
            Some(slot) => *slot = animation,
            None => self.animations.push(animation),
        }
    }

    pub fn remove(&mut self, id: &str) -> Option<AmAnimation> {
        let idx = self.animations.iter().position(|a| a.id == id)?;
        Some(self.animations.remove(idx))
    }

    /// 生成一个未占用的 id（`anim`、`anim2`…）。
    pub fn suggest_id(&self, base: &str) -> Id {
        let base = normalize_animation_id(base);
        let base = if base.is_empty() { "anim".to_string() } else { base };
        if !self.contains(&base) {
            return base;
        }
        for n in 2..100_000u32 {
            let candidate = format!("{base}-{n}");
            if !self.contains(&candidate) {
                return candidate;
            }
        }
        format!("{base}-{}", self.animations.len() + 1)
    }
}

/// 供 `am-model::Spec` 引用的几何常量（避免上层重复字面量）。
pub const GEOMETRY_MODEL_REF: &str = "model_ref";
pub const GEOMETRY_SNAPSHOT: &str = "snapshot";

/// 把 `Easing` 转成宿主可读的 JSON（与 `MotionKey.easing` 同形状，§4.5）。
pub fn easing_to_value(easing: &Easing) -> serde_json::Value {
    serde_json::to_value(easing).unwrap_or(serde_json::Value::Null)
}

/// 从宿主 JSON 解析 `Easing`（§4.5）。
pub fn easing_from_value(value: &serde_json::Value) -> Option<Easing> {
    serde_json::from_value(value.clone()).ok()
}

/// 供 §4.5 的切线映射：`p1 = (0.42, in*0.42)`、`p2 = (0.58, 1 - out*0.42)`。
///
/// 这条映射**两侧都必须一致**（编辑器曲线面板与引擎），所以放在这里当唯一真相。
pub fn easing_from_tangents(in_tangent: f32, out_tangent: f32) -> Easing {
    Easing::CubicBezier {
        p1: Vec2::new(0.42, in_tangent * 0.42),
        p2: Vec2::new(0.58, 1.0 - out_tangent * 0.42),
    }
}

pub mod bake;

#[cfg(test)]
mod bake_tests;
#[cfg(test)]
mod tests;

pub use bake::{
    apply_auto_effects, bake, geometry_budget, plan_geometry, BakeError, BakeOptions, BakeResult,
    GeometryPlan,
};
