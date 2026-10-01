//! 模型文档数据模型（`spec/model.json`）—— 引擎的核心数据结构。
//!
//! 设计要点：
//!
//! - **扁平节点表 + `parent` 引用**：而不是深层嵌套 JSON。这样增量编辑、命令流、
//!   撤销重做与跨 FFI 查询都不需要重建整棵树。
//! - **关键形挂在节点上**：`Node::keyforms` 是 `参数 id → 关键形列表（按参数值升序）`，
//!   与同类软件的「每个变形器/绘制对象各持一套关键形」一致。
//! - **绘制顺序**：同级内按 `(draw_order, id)` 排序，保证顺序确定且可复现。

use crate::id::{self, Id};
use crate::param::{Parameter, ParameterGroup};
use am_math::{Rect, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `spec/model.json` 的结构版本。
pub const MODEL_FORMAT_VERSION: u32 = 1;

fn default_one() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------- 画布

/// 画布定义。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    pub width: f32,
    pub height: f32,
    /// 画布原点（通常为画布中心）。
    #[serde(default)]
    pub origin: Vec2,
    /// 1 单位等于多少画布像素（导出到其它引擎时使用）。
    #[serde(default = "default_one")]
    pub pixels_per_unit: f32,
}

impl Default for Canvas {
    fn default() -> Self {
        Self { width: 0.0, height: 0.0, origin: Vec2::ZERO, pixels_per_unit: 1.0 }
    }
}

impl Canvas {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height, ..Default::default() }
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    pub fn size(&self) -> Vec2 {
        Vec2::new(self.width, self.height)
    }

    /// 画布矩形（以 `origin` 为中心）。
    pub fn rect(&self) -> Rect {
        Rect::from_center_size(self.origin, self.size())
    }

    /// 从包围盒自动确定画布（用于分层位图导入后没有显式画布的情况）。
    pub fn fit_to(&mut self, bounds: Rect) {
        let size = bounds.size();
        self.width = size.x.max(1.0);
        self.height = size.y.max(1.0);
        self.origin = bounds.center();
    }
}

// ---------------------------------------------------------------- 纹理

/// 纹理引用：指向 `assets/` 下的一张图。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextureRef {
    pub id: Id,
    /// 资源相对路径，例如 `assets/images/atlas-0.png`。
    pub asset: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// 是否为图集（多张原图打包在一起）。
    #[serde(default)]
    pub atlas: bool,
}

impl TextureRef {
    pub fn new(asset: impl Into<String>) -> Self {
        Self { id: id::texture_id(), asset: asset.into(), width: 0, height: 0, atlas: false }
    }
}

// ---------------------------------------------------------------- 混合模式

/// 绘制混合模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendType {
    #[default]
    Normal,
    Multiply,
    Screen,
    Additive,
}

impl BlendType {
    /// 着色器分支索引（与 `am-render` 的管线编号一致）。
    pub fn shader_index(self) -> u32 {
        match self {
            BlendType::Normal => 0,
            BlendType::Multiply => 1,
            BlendType::Screen => 2,
            BlendType::Additive => 3,
        }
    }
}

// ---------------------------------------------------------------- 网格

/// 三角网格。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    #[serde(default)]
    pub vertices: Vec<Vec2>,
    #[serde(default)]
    pub uvs: Vec<Vec2>,
    #[serde(default)]
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn new(vertices: Vec<Vec2>, uvs: Vec<Vec2>, indices: Vec<u32>) -> Self {
        Self { vertices, uvs, indices }
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    pub fn bounds(&self) -> Option<Rect> {
        Rect::from_points(self.vertices.iter().copied())
    }

    /// 结构校验：索引越界、UV 数量不匹配、索引数不是 3 的倍数。
    pub fn validate(&self) -> Result<(), String> {
        if self.indices.len() % 3 != 0 {
            return Err(format!("索引数量 {} 不是 3 的倍数", self.indices.len()));
        }
        if !self.uvs.is_empty() && self.uvs.len() != self.vertices.len() {
            return Err(format!(
                "UV 数量 {} 与顶点数量 {} 不一致",
                self.uvs.len(),
                self.vertices.len()
            ));
        }
        let n = self.vertices.len() as u32;
        if let Some(bad) = self.indices.iter().find(|i| **i >= n) {
            return Err(format!("索引 {bad} 超出顶点范围 (0..{n})"));
        }
        Ok(())
    }

    /// 顶点是否全部有限。
    pub fn is_finite(&self) -> bool {
        self.vertices.iter().all(|v| v.is_finite()) && self.uvs.iter().all(|v| v.is_finite())
    }
}

// ---------------------------------------------------------------- 节点数据

/// 绘制对象数据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawableData {
    /// `Model::textures` 的下标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<u32>,
    /// 图集内的 UV 子矩形；`None` 表示整张纹理。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uv_rect: Option<Rect>,
    pub mesh: Mesh,
    #[serde(default = "default_one")]
    pub opacity: f32,
    #[serde(default)]
    pub blend: BlendType,
    /// 作为裁剪遮罩源的节点 id（多个为逻辑与）。
    #[serde(default)]
    pub masks: Vec<Id>,
    /// 遮罩取反。
    #[serde(default)]
    pub inverted_mask: bool,
    /// 背面剔除（部件被旋转/翻转时是否丢弃）。
    #[serde(default)]
    pub culling: bool,
}

impl Default for DrawableData {
    fn default() -> Self {
        Self {
            texture: None,
            uv_rect: None,
            mesh: Mesh::default(),
            opacity: 1.0,
            blend: BlendType::Normal,
            masks: Vec::new(),
            inverted_mask: false,
            culling: false,
        }
    }
}

/// 自由变形（Warps）数据：`(rows+1) × (cols+1)` 个控制点。
///
/// 语义：`rest_rect` 是**静止网格**（均匀分布、位于本节点局部空间）；
/// `control_points` 是这些格点在**父空间**中的当前位置。两者在静止姿态下重合，
/// 因此映射退化为恒等 —— 这是求值器 `am-eval::WarpMap` 的基础。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WarpData {
    pub rows: u32,
    pub cols: u32,
    /// 静止网格的矩形范围；缺省时由控制点包围盒推断（退化情况）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rest_rect: Option<Rect>,
    #[serde(default)]
    pub control_points: Vec<Vec2>,
    /// 是否显示网格（仅编辑器）。
    #[serde(default = "default_true")]
    pub show_grid: bool,
}

impl Default for WarpData {
    fn default() -> Self {
        Self { rows: 1, cols: 1, rest_rect: None, control_points: Vec::new(), show_grid: true }
    }
}

impl WarpData {
    /// 期望的控制点数量。
    pub fn expected_points(&self) -> usize {
        ((self.rows + 1) * (self.cols + 1)) as usize
    }

    pub fn is_complete(&self) -> bool {
        self.control_points.len() == self.expected_points()
    }

    /// 用矩形边界生成均匀控制点网格。
    pub fn from_rect(rows: u32, cols: u32, rect: Rect) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let mut control_points = Vec::with_capacity(((rows + 1) * (cols + 1)) as usize);
        for r in 0..=rows {
            for c in 0..=cols {
                let u = c as f32 / cols as f32;
                let v = r as f32 / rows as f32;
                control_points.push(Vec2::new(
                    rect.min.x + rect.width() * u,
                    rect.min.y + rect.height() * v,
                ));
            }
        }
        Self { rows, cols, rest_rect: Some(rect), control_points, show_grid: true }
    }

    /// 控制点索引。
    pub fn point_index(&self, row: u32, col: u32) -> Option<usize> {
        if row > self.rows || col > self.cols {
            return None;
        }
        Some((row * (self.cols + 1) + col) as usize)
    }
}

/// 旋转变形数据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RotationData {
    /// 弧度。
    #[serde(default)]
    pub angle: f32,
    #[serde(default)]
    pub position: Vec2,
    #[serde(default = "one_vec2")]
    pub scale: Vec2,
    /// 旋转轴心（局部坐标）。
    #[serde(default)]
    pub origin: Vec2,
    /// 编辑器手柄长度。
    #[serde(default = "default_handle")]
    pub handle_length: f32,
}

fn one_vec2() -> Vec2 {
    Vec2::ONE
}

fn default_handle() -> f32 {
    40.0
}

impl Default for RotationData {
    fn default() -> Self {
        Self {
            angle: 0.0,
            position: Vec2::ZERO,
            scale: Vec2::ONE,
            origin: Vec2::ZERO,
            handle_length: 40.0,
        }
    }
}

/// 关键形中对旋转变形的覆盖值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RotationValue {
    #[serde(default)]
    pub angle: f32,
    #[serde(default)]
    pub position: Vec2,
    #[serde(default = "one_vec2")]
    pub scale: Vec2,
    #[serde(default)]
    pub origin: Vec2,
}

impl Default for RotationValue {
    fn default() -> Self {
        Self { angle: 0.0, position: Vec2::ZERO, scale: Vec2::ONE, origin: Vec2::ZERO }
    }
}

/// 一个关键形：某参数取 `value` 时对某个节点的局部覆盖。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyform {
    /// 参数取值。
    pub value: f32,
    /// 关键形混合类型。
    #[serde(default)]
    pub blend: BlendType,
    /// 顶点覆盖（绘制对象；长度须与网格顶点数一致）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertices: Option<Vec<Vec2>>,
    /// 不透明度覆盖。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    /// 自由变形控制点覆盖（长度须与 `WarpData::expected_points` 一致）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_points: Option<Vec<Vec2>>,
    /// 旋转变形覆盖。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<RotationValue>,
    /// 绘制顺序覆盖。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_order: Option<i32>,
}

impl Keyform {
    /// 一个只有参数位置、没有覆盖内容的关键形（用于登记关键点）。
    pub fn marker(value: f32) -> Self {
        Self {
            value,
            blend: BlendType::Normal,
            vertices: None,
            opacity: None,
            control_points: None,
            rotation: None,
            draw_order: None,
        }
    }

    pub fn with_vertices(value: f32, vertices: Vec<Vec2>) -> Self {
        Self { vertices: Some(vertices), ..Self::marker(value) }
    }

    pub fn with_control_points(value: f32, points: Vec<Vec2>) -> Self {
        Self { control_points: Some(points), ..Self::marker(value) }
    }

    pub fn with_rotation(value: f32, rotation: RotationValue) -> Self {
        Self { rotation: Some(rotation), ..Self::marker(value) }
    }

    pub fn with_opacity(value: f32, opacity: f32) -> Self {
        Self { opacity: Some(opacity), ..Self::marker(value) }
    }

    pub fn with_draw_order(value: f32, order: i32) -> Self {
        Self { draw_order: Some(order), ..Self::marker(value) }
    }
}

// ---------------------------------------------------------------- 节点

/// 节点类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// 纯层级分组节点。
    Part,
    WarpDeformer,
    RotationDeformer,
    Drawable,
}

impl NodeKind {
    pub fn is_deformer(self) -> bool {
        matches!(self, NodeKind::WarpDeformer | NodeKind::RotationDeformer)
    }
}

/// 层级树上的一个节点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: Id,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id>,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    /// 同级绘制顺序（小的先画）。
    #[serde(default)]
    pub draw_order: i32,
    pub kind: NodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drawable: Option<DrawableData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warp: Option<WarpData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<RotationData>,
    /// 关键形：参数 id → 关键形列表（按参数值升序）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keyforms: BTreeMap<Id, Vec<Keyform>>,
}

impl Node {
    pub fn new(kind: NodeKind, name: impl Into<String>, parent: Option<Id>) -> Self {
        Self {
            id: id::node_id(),
            name: name.into(),
            parent,
            visible: true,
            locked: false,
            draw_order: 0,
            kind,
            drawable: None,
            warp: None,
            rotation: None,
            keyforms: BTreeMap::new(),
        }
    }

    pub fn part(name: impl Into<String>, parent: Option<Id>) -> Self {
        Self::new(NodeKind::Part, name, parent)
    }

    pub fn drawable(name: impl Into<String>, parent: Option<Id>, mesh: Mesh) -> Self {
        let mut node = Self::new(NodeKind::Drawable, name, parent);
        node.drawable = Some(DrawableData { mesh, ..Default::default() });
        node
    }

    pub fn warp_deformer(
        name: impl Into<String>,
        parent: Option<Id>,
        rows: u32,
        cols: u32,
        rect: Rect,
    ) -> Self {
        let mut node = Self::new(NodeKind::WarpDeformer, name, parent);
        node.warp = Some(WarpData::from_rect(rows, cols, rect));
        node
    }

    pub fn rotation_deformer(name: impl Into<String>, parent: Option<Id>) -> Self {
        let mut node = Self::new(NodeKind::RotationDeformer, name, parent);
        node.rotation = Some(RotationData::default());
        node
    }

    pub fn mesh(&self) -> Option<&Mesh> {
        self.drawable.as_ref().map(|d| &d.mesh)
    }

    /// 本节点的顶点数（绘制对象）。
    pub fn vertex_count(&self) -> usize {
        self.drawable.as_ref().map(|d| d.mesh.vertices.len()).unwrap_or(0)
    }

    /// 关键形列表（按参数值升序）。
    pub fn keyform_list(&self, parameter: &str) -> Option<&Vec<Keyform>> {
        self.keyforms.get(parameter)
    }

    /// 插入/覆盖关键形，并保持升序。
    pub fn set_keyform(&mut self, parameter: &str, keyform: Keyform) {
        let list = self.keyforms.entry(parameter.to_string()).or_default();
        match list.binary_search_by(|k| {
            k.value.partial_cmp(&keyform.value).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Ok(idx) => list[idx] = keyform,
            Err(idx) => list.insert(idx, keyform),
        }
    }

    pub fn remove_keyform(&mut self, parameter: &str, value: f32, tolerance: f32) -> bool {
        let Some(list) = self.keyforms.get_mut(parameter) else {
            return false;
        };
        if let Some(idx) = list.iter().position(|k| (k.value - value).abs() <= tolerance) {
            list.remove(idx);
            if list.is_empty() {
                self.keyforms.remove(parameter);
            }
            true
        } else {
            false
        }
    }

    pub fn clear_keyforms(&mut self) {
        self.keyforms.clear();
    }
}

// ---------------------------------------------------------------- 模型

/// 模型文档。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    #[serde(default = "default_format_version")]
    pub version: u32,
    #[serde(default)]
    pub id: Id,
    pub name: String,
    #[serde(default)]
    pub canvas: Canvas,
    #[serde(default)]
    pub textures: Vec<TextureRef>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub parameter_groups: Vec<ParameterGroup>,
}

fn default_format_version() -> u32 {
    MODEL_FORMAT_VERSION
}

impl Default for Model {
    fn default() -> Self {
        Self {
            version: MODEL_FORMAT_VERSION,
            id: id::new_id("model"),
            name: String::new(),
            canvas: Canvas::default(),
            textures: Vec::new(),
            nodes: Vec::new(),
            parameters: Vec::new(),
            parameter_groups: Vec::new(),
        }
    }
}

impl Model {
    /// 新建空模型。
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), ..Default::default() }
    }

    // ------------------------------------------------------------ 节点查询

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    pub fn has_node(&self, id: &str) -> bool {
        self.node_index(id).is_some()
    }

    /// 直接子节点，按 `(draw_order, id)` 升序。
    pub fn children_of(&self, parent: Option<&str>) -> Vec<&Node> {
        let mut v: Vec<&Node> =
            self.nodes.iter().filter(|n| n.parent.as_deref() == parent).collect();
        v.sort_by(|a, b| a.draw_order.cmp(&b.draw_order).then_with(|| a.id.cmp(&b.id)));
        v
    }

    pub fn child_ids(&self, parent: Option<&str>) -> Vec<Id> {
        self.children_of(parent).iter().map(|n| n.id.clone()).collect()
    }

    /// 从根到该节点的祖先链（不含自身）。
    pub fn ancestors(&self, id: &str) -> Vec<Id> {
        let mut out = Vec::new();
        let mut cur = self.node(id).and_then(|n| n.parent.clone());
        let mut guard = 0;
        while let Some(p) = cur {
            out.push(p.clone());
            guard += 1;
            if guard > self.nodes.len() + 1 {
                break; // 防御环
            }
            cur = self.node(&p).and_then(|n| n.parent.clone());
        }
        out
    }

    /// 全部后代（不含自身），深度优先。
    pub fn descendants(&self, id: &str) -> Vec<Id> {
        let mut out = Vec::new();
        let mut stack = self.child_ids(Some(id));
        while let Some(child) = stack.pop() {
            out.push(child.clone());
            stack.extend(self.child_ids(Some(&child)));
        }
        out
    }

    /// 绘制顺序（全部节点的深度优先前序遍历）。
    pub fn paint_order(&self) -> Vec<Id> {
        let mut out = Vec::new();
        let mut stack: Vec<Id> = self.child_ids(None).into_iter().rev().collect();
        while let Some(id) = stack.pop() {
            out.push(id.clone());
            let kids = self.child_ids(Some(&id));
            for k in kids.into_iter().rev() {
                stack.push(k);
            }
        }
        out
    }

    /// 需要真正绘制的节点（按绘制顺序）。
    pub fn drawables_in_order(&self) -> Vec<&Node> {
        self.paint_order()
            .into_iter()
            .filter_map(|id| self.node(&id))
            .filter(|n| n.kind == NodeKind::Drawable)
            .collect()
    }

    /// 从根到叶的深度。
    pub fn depth_of(&self, id: &str) -> usize {
        self.ancestors(id).len()
    }

    // ------------------------------------------------------------ 节点编辑

    /// 追加节点；`draw_order` 若为 0 则自动排在同级末尾。
    pub fn add_node(&mut self, mut node: Node) -> Id {
        if node.draw_order == 0 {
            node.draw_order = self.next_draw_order(node.parent.as_deref());
        }
        let id = node.id.clone();
        self.nodes.push(node);
        id
    }

    /// 同级下一个可用的绘制顺序。
    pub fn next_draw_order(&self, parent: Option<&str>) -> i32 {
        self.children_of(parent).iter().map(|n| n.draw_order).max().map(|m| m + 1).unwrap_or(0)
    }

    /// 删除节点及其全部后代；同时清理其它节点对这些节点的遮罩引用。
    /// 返回被删除的 id 列表（含传入的 id）。
    pub fn remove_node(&mut self, id: &str) -> Vec<Id> {
        if !self.has_node(id) {
            return Vec::new();
        }
        let mut removed = vec![id.to_string()];
        removed.extend(self.descendants(id));
        self.nodes.retain(|n| !removed.contains(&n.id));
        for node in &mut self.nodes {
            if let Some(d) = &mut node.drawable {
                d.masks.retain(|m| !removed.contains(m));
            }
        }
        removed
    }

    /// 是否可以安全地把 `id` 挂到 `new_parent` 下（不能成环、不能挂到自身）。
    pub fn can_reparent(&self, id: &str, new_parent: Option<&str>) -> bool {
        match new_parent {
            None => true,
            Some(p) => p != id && self.has_node(p) && !self.descendants(id).iter().any(|d| d == p),
        }
    }

    /// 改变父节点。成环时返回 `false` 且不做任何修改。
    pub fn reparent(&mut self, id: &str, new_parent: Option<&str>) -> bool {
        if !self.can_reparent(id, new_parent) {
            return false;
        }
        let order = self.next_draw_order(new_parent);
        if let Some(node) = self.node_mut(id) {
            node.parent = new_parent.map(str::to_string);
            node.draw_order = order;
            true
        } else {
            false
        }
    }

    /// 设置绘制顺序。
    pub fn set_draw_order(&mut self, id: &str, order: i32) -> bool {
        match self.node_mut(id) {
            Some(n) => {
                n.draw_order = order;
                true
            }
            None => false,
        }
    }

    /// 把节点在兄弟之间上移/下移一位。
    pub fn nudge_order(&mut self, id: &str, delta: i32) -> bool {
        let Some(idx) = self.node_index(id) else {
            return false;
        };
        let parent = self.nodes[idx].parent.clone();
        let sibling_orders: Vec<i32> =
            self.children_of(parent.as_deref()).iter().map(|n| n.draw_order).collect();
        if sibling_orders.len() < 2 {
            return false;
        }
        let cur = self.nodes[idx].draw_order;
        let pos = sibling_orders.iter().position(|o| *o == cur).unwrap_or(0) as i32;
        let target = (pos + delta).clamp(0, sibling_orders.len() as i32 - 1);
        if target == pos {
            return false;
        }
        let target_order = sibling_orders[target as usize];
        let other_id = self
            .children_of(parent.as_deref())
            .get(target as usize)
            .map(|n| n.id.clone());
        self.nodes[idx].draw_order = target_order;
        if let Some(other) = other_id {
            if let Some(o) = self.node_mut(&other) {
                o.draw_order = cur;
            }
        }
        true
    }

    // ------------------------------------------------------------ 参数

    pub fn parameter(&self, id: &str) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.id == id)
    }

    pub fn parameter_mut(&mut self, id: &str) -> Option<&mut Parameter> {
        self.parameters.iter_mut().find(|p| p.id == id)
    }

    pub fn parameter_by_name(&self, name: &str) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.name == name)
    }

    pub fn add_parameter(&mut self, parameter: Parameter) -> Id {
        let id = parameter.id.clone();
        self.parameters.push(parameter);
        id
    }

    /// 删除参数，并清理全部节点上该参数的关键形。
    pub fn remove_parameter(&mut self, id: &str) -> bool {
        let before = self.parameters.len();
        self.parameters.retain(|p| p.id != id);
        for node in &mut self.nodes {
            node.keyforms.remove(id);
        }
        for group in &mut self.parameter_groups {
            group.parameters.retain(|p| p != id);
        }
        self.parameters.len() != before
    }

    /// 追加一个关键点（无覆盖内容）到所有 `targets` 节点。
    pub fn add_parameter_key(&mut self, parameter: &str, value: f32, targets: &[Id]) -> usize {
        let mut n = 0;
        for id in targets {
            if let Some(node) = self.node_mut(id) {
                if node.keyforms.get(parameter).map(|l| l.iter().any(|k| (k.value - value).abs() <= 1e-6)).unwrap_or(false) {
                    continue;
                }
                node.set_keyform(parameter, Keyform::marker(value));
                n += 1;
            }
        }
        n
    }

    pub fn parameters_by_group(&self, group: &str) -> Vec<&Parameter> {
        self.parameters.iter().filter(|p| p.group.as_deref() == Some(group)).collect()
    }

    // ------------------------------------------------------------ 纹理

    pub fn texture(&self, index: u32) -> Option<&TextureRef> {
        self.textures.get(index as usize)
    }

    pub fn add_texture(&mut self, texture: TextureRef) -> u32 {
        self.textures.push(texture);
        (self.textures.len() - 1) as u32
    }

    // ------------------------------------------------------------ 统计

    /// 结构统计（查看器的性能检查与编辑器状态栏使用）。
    pub fn stats(&self) -> ModelStats {
        let mut s = ModelStats {
            nodes: self.nodes.len(),
            parameters: self.parameters.len(),
            textures: self.textures.len(),
            ..Default::default()
        };
        for n in &self.nodes {
            match n.kind {
                NodeKind::Part => s.parts += 1,
                NodeKind::WarpDeformer => s.warp_deformers += 1,
                NodeKind::RotationDeformer => s.rotation_deformers += 1,
                NodeKind::Drawable => {
                    s.drawables += 1;
                    if let Some(d) = &n.drawable {
                        s.vertices += d.mesh.vertices.len();
                        s.triangles += d.mesh.triangle_count();
                        s.masked_drawables += (!d.masks.is_empty()) as usize;
                    }
                }
            }
            s.keyforms += n.keyforms.values().map(Vec::len).sum::<usize>();
            s.max_depth = s.max_depth.max(self.depth_of(&n.id));
        }
        s
    }

    /// 所有绘制对象的包围盒。
    pub fn bounds(&self) -> Option<Rect> {
        let mut acc: Option<Rect> = None;
        for n in &self.nodes {
            if let Some(mesh) = n.mesh() {
                if let Some(b) = mesh.bounds() {
                    acc = Some(match acc {
                        Some(a) => a.union(b),
                        None => b,
                    });
                }
            }
        }
        acc
    }
}

/// 模型结构统计。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelStats {
    pub nodes: usize,
    pub parts: usize,
    pub warp_deformers: usize,
    pub rotation_deformers: usize,
    pub drawables: usize,
    pub masked_drawables: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub keyforms: usize,
    pub parameters: usize,
    pub textures: usize,
    pub max_depth: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh_quad() -> Mesh {
        Mesh::new(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, 1.0),
            ],
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, 1.0),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    fn demo_model() -> Model {
        let mut m = Model::new("demo");
        let root = m.add_node(Node::part("Root", None));
        let face = m.add_node(Node::part("Face", Some(root.clone())));
        let eye = m.add_node(Node::drawable("Eye", Some(face.clone()), mesh_quad()));
        let _hair = m.add_node(Node::drawable("Hair", Some(root.clone()), mesh_quad()));
        for (id, order) in [(&root, 0), (&face, 0), (&eye, 0)] {
            m.set_draw_order(id, order);
        }
        m
    }

    #[test]
    fn hierarchy_queries_are_consistent() {
        let m = demo_model();
        assert_eq!(m.nodes.len(), 4);
        let root = m.nodes.iter().find(|n| n.name == "Root").unwrap().id.clone();
        let face = m.nodes.iter().find(|n| n.name == "Face").unwrap().id.clone();
        let eye = m.nodes.iter().find(|n| n.name == "Eye").unwrap().id.clone();

        assert_eq!(m.child_ids(None), vec![root.clone()]);
        assert_eq!(m.children_of(Some(&root)).len(), 2);
        assert_eq!(m.descendants(&root).len(), 3);
        assert_eq!(m.ancestors(&eye), vec![face.clone(), root.clone()]);
        assert_eq!(m.depth_of(&eye), 2);
        assert_eq!(m.depth_of(&root), 0);
    }

    #[test]
    fn paint_order_is_depth_first_and_respects_draw_order() {
        let mut m = demo_model();
        let root = m.nodes.iter().find(|n| n.name == "Root").unwrap().id.clone();
        let hair = m.nodes.iter().find(|n| n.name == "Hair").unwrap().id.clone();
        // 让 Hair 画在 Face 之前
        m.set_draw_order(&hair, -1);
        let order = m.paint_order();
        let hair_pos = order.iter().position(|i| *i == hair).unwrap();
        let face_pos = order.iter().position(|i| *i == m.nodes.iter().find(|n| n.name == "Face").unwrap().id).unwrap();
        assert!(hair_pos < face_pos, "绘制顺序未生效: {order:?}");
        assert_eq!(m.drawables_in_order().len(), 2);
        assert_eq!(order[0], root);
    }

    #[test]
    fn reparent_rejects_cycles() {
        let mut m = demo_model();
        let root = m.nodes.iter().find(|n| n.name == "Root").unwrap().id.clone();
        let face = m.nodes.iter().find(|n| n.name == "Face").unwrap().id.clone();
        let eye = m.nodes.iter().find(|n| n.name == "Eye").unwrap().id.clone();

        assert!(!m.can_reparent(&root, Some(&eye)), "把祖先挂到后代下应被拒绝");
        assert!(!m.reparent(&root, Some(&eye)));
        assert!(m.node(&root).unwrap().parent.is_none(), "失败的 reparent 不应改动数据");

        assert!(m.can_reparent(&eye, None));
        assert!(m.reparent(&eye, Some(&root)));
        assert_eq!(m.node(&eye).unwrap().parent.as_deref(), Some(root.as_str()));
        assert!(m.can_reparent(&face, None));
        assert!(!m.can_reparent(&face, Some("nonexistent")));
    }

    #[test]
    fn removing_node_removes_subtree_and_mask_references() {
        let mut m = demo_model();
        let root = m.nodes.iter().find(|n| n.name == "Root").unwrap().id.clone();
        let eye = m.nodes.iter().find(|n| n.name == "Eye").unwrap().id.clone();
        let hair = m.nodes.iter().find(|n| n.name == "Hair").unwrap().id.clone();
        m.node_mut(&hair).unwrap().drawable.as_mut().unwrap().masks = vec![eye.clone()];

        let removed = m.remove_node(&root);
        assert_eq!(removed.len(), 4);
        assert!(m.nodes.is_empty());
        assert!(m.remove_node("nope").is_empty());
    }

    #[test]
    fn mask_references_are_cleaned_on_partial_removal() {
        let mut m = demo_model();
        let eye = m.nodes.iter().find(|n| n.name == "Eye").unwrap().id.clone();
        let hair = m.nodes.iter().find(|n| n.name == "Hair").unwrap().id.clone();
        m.node_mut(&hair).unwrap().drawable.as_mut().unwrap().masks = vec![eye.clone()];
        m.remove_node(&eye);
        assert!(m.node(&hair).unwrap().drawable.as_ref().unwrap().masks.is_empty());
    }

    #[test]
    fn keyforms_stay_sorted_and_overwrite() {
        let mut node = Node::drawable("Eye", None, mesh_quad());
        node.set_keyform("p1", Keyform::with_opacity(1.0, 1.0));
        node.set_keyform("p1", Keyform::with_opacity(-1.0, 0.0));
        node.set_keyform("p1", Keyform::with_opacity(0.0, 0.5));
        node.set_keyform("p1", Keyform::with_opacity(0.0, 0.9));
        let list = node.keyform_list("p1").unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list.iter().map(|k| k.value).collect::<Vec<_>>(), vec![-1.0, 0.0, 1.0]);
        assert_eq!(list[1].opacity, Some(0.9));

        assert!(node.remove_keyform("p1", 0.0, 1e-6));
        assert_eq!(node.keyform_list("p1").unwrap().len(), 2);
        node.remove_keyform("p1", -1.0, 1e-6);
        node.remove_keyform("p1", 1.0, 1e-6);
        assert!(node.keyform_list("p1").is_none(), "清空后应移除键");
    }

    #[test]
    fn removing_parameter_clears_keyforms() {
        let mut m = demo_model();
        let eye = m.nodes.iter().find(|n| n.name == "Eye").unwrap().id.clone();
        let p = m.add_parameter(Parameter::new("p1", "AngleX", -30.0, 30.0, 0.0));
        m.add_parameter_key(&p, 0.0, &[eye.clone()]);
        m.add_parameter_key(&p, 0.0, &[eye.clone()]); // 重复不应增加
        assert_eq!(m.node(&eye).unwrap().keyform_list(&p).unwrap().len(), 1);
        assert!(m.remove_parameter(&p));
        assert!(m.node(&eye).unwrap().keyform_list(&p).is_none());
    }

    #[test]
    fn nudge_order_swaps_neighbours() {
        let mut m = demo_model();
        let root = m.nodes.iter().find(|n| n.name == "Root").unwrap().id.clone();
        let face = m.nodes.iter().find(|n| n.name == "Face").unwrap().id.clone();
        let hair = m.nodes.iter().find(|n| n.name == "Hair").unwrap().id.clone();
        // Face(0) 在 Hair(1) 之前
        let before: Vec<Id> = m.children_of(Some(&root)).iter().map(|n| n.id.clone()).collect();
        assert_eq!(before, vec![face.clone(), hair.clone()]);
        assert!(m.nudge_order(&hair, -1));
        let after: Vec<Id> = m.children_of(Some(&root)).iter().map(|n| n.id.clone()).collect();
        assert_eq!(after, vec![hair.clone(), face]);
        assert!(!m.nudge_order(&hair, -5), "越界移动应返回 false");
    }

    #[test]
    fn warp_control_points_are_complete() {
        let w = WarpData::from_rect(2, 3, Rect::from_min_size(Vec2::ZERO, Vec2::new(30.0, 20.0)));
        assert_eq!(w.expected_points(), 12);
        assert!(w.is_complete());
        assert_eq!(w.point_index(0, 0), Some(0));
        assert_eq!(w.point_index(2, 3), Some(11));
        assert_eq!(w.point_index(3, 0), None);
        assert_eq!(w.control_points[0], Vec2::ZERO);
        assert_eq!(w.control_points[11], Vec2::new(30.0, 20.0));
    }

    #[test]
    fn mesh_validation_catches_bad_data() {
        assert!(mesh_quad().validate().is_ok());
        let bad_index = Mesh::new(vec![Vec2::ZERO], vec![], vec![0, 1, 2]);
        assert!(bad_index.validate().is_err());
        let bad_uv = Mesh::new(vec![Vec2::ZERO], vec![Vec2::ZERO, Vec2::ZERO], vec![]);
        assert!(bad_uv.validate().is_err());
        let bad_tri = Mesh::new(vec![Vec2::ZERO], vec![], vec![0, 1]);
        assert!(bad_tri.validate().is_err());
    }

    #[test]
    fn stats_and_bounds_are_computed() {
        let m = demo_model();
        let s = m.stats();
        assert_eq!(s.nodes, 4);
        assert_eq!(s.parts, 2);
        assert_eq!(s.drawables, 2);
        assert_eq!(s.vertices, 8);
        assert_eq!(s.triangles, 4);
        assert_eq!(s.max_depth, 2);
        assert_eq!(m.bounds().unwrap().size(), Vec2::new(1.0, 1.0));
    }

    #[test]
    fn model_json_round_trips() {
        let mut m = demo_model();
        m.canvas = Canvas::new(1920.0, 1080.0);
        m.add_texture(TextureRef::new("assets/images/atlas-0.png"));
        let p = m.add_parameter(Parameter::new("p1", "AngleX", -30.0, 30.0, 0.0));
        let eye = m.nodes.iter().find(|n| n.name == "Eye").unwrap().id.clone();
        m.node_mut(&eye).unwrap().set_keyform(&p, Keyform::with_vertices(10.0, vec![Vec2::ZERO; 4]));
        m.node_mut(&eye).unwrap().drawable.as_mut().unwrap().texture = Some(0);
        m.node_mut(&eye).unwrap().drawable.as_mut().unwrap().blend = BlendType::Multiply;

        let text = serde_json::to_string_pretty(&m).unwrap();
        let back: Model = serde_json::from_str(&text).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn canvas_fit_to_bounds_centers_origin() {
        let mut c = Canvas::default();
        assert!(c.is_empty());
        c.fit_to(Rect::from_min_max(Vec2::new(-10.0, -20.0), Vec2::new(30.0, 40.0)));
        assert_eq!(c.width, 40.0);
        assert_eq!(c.height, 60.0);
        assert_eq!(c.origin, Vec2::new(10.0, 10.0));
        assert_eq!(c.rect().center(), Vec2::new(10.0, 10.0));
    }

    #[test]
    fn blend_type_shader_indices_are_stable() {
        assert_eq!(BlendType::Normal.shader_index(), 0);
        assert_eq!(BlendType::Multiply.shader_index(), 1);
        assert_eq!(BlendType::Screen.shader_index(), 2);
        assert_eq!(BlendType::Additive.shader_index(), 3);
    }
}
