//! 命令（编辑操作）：文档的**唯一**可撤销写入通道。
//!
//! 每条命令都是可序列化的 JSON 对象（`{"op": "...", ...}`），因此
//! 编辑器、脚本、自动化测试走的是同一条路径 —— 这也是引擎 FFI 契约里
//! `doc.command` 的实现基础。
//!
//! 约定：
//! * 命令**不做隐式级联**：写什么就是什么；需要联动时用 `batch` 显式组合，
//!   一次 `batch` 只产生一步撤销。
//! * 命令失败必须**不留痕迹**（调用方在失败时丢弃快照）。

use am_math::{Rect, Vec2};
use am_model::{
    id, BlendType, Id, Keyform, Mesh, Model, Node, NodeKind, Parameter, RotationData, RotationValue,
    TextureRef, WarpData,
};
use serde::{Deserialize, Serialize};

/// 一次编辑操作。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Command {
    /// 修改画布。
    SetCanvas {
        width: f32,
        height: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<Vec2>,
    },

    // ---------------------------------------------------------- 节点
    NodeCreate {
        kind: NodeKind,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<Id>,
        /// 绘制对象网格 / 变形器网格的初始矩形。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rect: Option<Rect>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rows: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cols: Option<u32>,
    },
    NodeDelete {
        node: Id,
    },
    NodeRename {
        node: Id,
        name: String,
    },
    NodeReparent {
        node: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<Id>,
    },
    NodeSetVisible {
        node: Id,
        visible: bool,
    },
    NodeSetLocked {
        node: Id,
        locked: bool,
    },
    NodeSetDrawOrder {
        node: Id,
        draw_order: i32,
    },
    /// 修改旋转变形（只写给出的字段）。
    NodeSetRotation {
        node: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        angle: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<Vec2>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scale: Option<Vec2>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<Vec2>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        handle_length: Option<f32>,
    },

    // ---------------------------------------------------------- 网格与变形器
    MeshSet {
        node: Id,
        vertices: Vec<Vec2>,
        uvs: Vec<Vec2>,
        indices: Vec<u32>,
    },
    MeshSetVertices {
        node: Id,
        vertices: Vec<Vec2>,
    },
    MeshSetUvs {
        node: Id,
        uvs: Vec<Vec2>,
    },
    MeshSetIndices {
        node: Id,
        indices: Vec<u32>,
    },
    WarpSetControlPoints {
        node: Id,
        points: Vec<Vec2>,
    },
    WarpResize {
        node: Id,
        rows: u32,
        cols: u32,
    },

    // ---------------------------------------------------------- 绘制对象
    DrawableSetTexture {
        node: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        texture: Option<u32>,
    },
    DrawableSetUvRect {
        node: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uv_rect: Option<Rect>,
    },
    DrawableSetBlend {
        node: Id,
        blend: BlendType,
    },
    DrawableSetOpacity {
        node: Id,
        opacity: f32,
    },
    DrawableSetMasks {
        node: Id,
        #[serde(default)]
        masks: Vec<Id>,
        #[serde(default)]
        inverted: bool,
    },
    DrawableSetCulling {
        node: Id,
        culling: bool,
    },

    // ---------------------------------------------------------- 关键形
    KeyformRecord {
        node: Id,
        parameter: Id,
        value: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blend: Option<BlendType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        vertices: Option<Vec<Vec2>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opacity: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        control_points: Option<Vec<Vec2>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation: Option<RotationValue>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        draw_order: Option<i32>,
    },
    KeyformRemove {
        node: Id,
        parameter: Id,
        value: f32,
    },

    // ---------------------------------------------------------- 参数
    ParameterAdd {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<Id>,
        name: String,
        min: f32,
        max: f32,
        default: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<Id>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weight: Option<f32>,
        #[serde(default)]
        repeat: bool,
    },
    ParameterRemove {
        parameter: Id,
    },
    ParameterSetRange {
        parameter: Id,
        min: f32,
        max: f32,
        default: f32,
    },

    // ---------------------------------------------------------- 纹理
    TextureAdd {
        asset: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<Id>,
        #[serde(default)]
        width: u32,
        #[serde(default)]
        height: u32,
        #[serde(default)]
        atlas: bool,
    },
    TextureRemove {
        index: u32,
    },

    // ---------------------------------------------------------- 组合
    /// 一批命令，作为**一步**撤销。
    Batch {
        commands: Vec<Command>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
}

/// 命令造成的影响（编辑器据此做增量刷新）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    /// 节点树结构变化（新增/删除/重排）。
    Structure,
    Node { node: Id },
    Parameters,
    Textures,
    Canvas,
    Physics,
    Motions,
    Expressions,
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("节点不存在：{0}")]
    NodeNotFound(Id),
    #[error("参数不存在：{0}")]
    ParameterNotFound(Id),
    #[error("纹理下标越界：{0}")]
    TextureOutOfRange(u32),
    #[error("命令不适用于节点 {0}（实际类型 {1:?}）")]
    WrongKind(Id, NodeKind),
    #[error("参数非法：{0}")]
    Invalid(String),
    #[error("父子关系非法（会造成环）：{0}")]
    Cycle(Id),
}

impl Command {
    /// 命令名（用于日志与错误信息）。
    pub fn op(&self) -> &'static str {
        match self {
            Command::SetCanvas { .. } => "set_canvas",
            Command::NodeCreate { .. } => "node.create",
            Command::NodeDelete { .. } => "node.delete",
            Command::NodeRename { .. } => "node.rename",
            Command::NodeReparent { .. } => "node.reparent",
            Command::NodeSetVisible { .. } => "node.set_visible",
            Command::NodeSetLocked { .. } => "node.set_locked",
            Command::NodeSetDrawOrder { .. } => "node.set_draw_order",
            Command::NodeSetRotation { .. } => "node.set_rotation",
            Command::MeshSet { .. } => "mesh.set",
            Command::MeshSetVertices { .. } => "mesh.set_vertices",
            Command::MeshSetUvs { .. } => "mesh.set_uvs",
            Command::MeshSetIndices { .. } => "mesh.set_indices",
            Command::WarpSetControlPoints { .. } => "warp.set_control_points",
            Command::WarpResize { .. } => "warp.resize",
            Command::DrawableSetTexture { .. } => "drawable.set_texture",
            Command::DrawableSetUvRect { .. } => "drawable.set_uv_rect",
            Command::DrawableSetBlend { .. } => "drawable.set_blend",
            Command::DrawableSetOpacity { .. } => "drawable.set_opacity",
            Command::DrawableSetMasks { .. } => "drawable.set_masks",
            Command::DrawableSetCulling { .. } => "drawable.set_culling",
            Command::KeyformRecord { .. } => "keyform.record",
            Command::KeyformRemove { .. } => "keyform.remove",
            Command::ParameterAdd { .. } => "parameter.add",
            Command::ParameterRemove { .. } => "parameter.remove",
            Command::ParameterSetRange { .. } => "parameter.set_range",
            Command::TextureAdd { .. } => "texture.add",
            Command::TextureRemove { .. } => "texture.remove",
            Command::Batch { .. } => "batch",
        }
    }

    /// 应用命令；失败时可能已部分修改，调用方负责回滚（`Document` 用快照回滚）。
    pub fn apply(&self, model: &mut Model) -> Result<Vec<Effect>, CommandError> {
        match self {
            Command::SetCanvas { width, height, origin } => {
                if !width.is_finite() || !height.is_finite() || *width <= 0.0 || *height <= 0.0 {
                    return Err(CommandError::Invalid("画布尺寸必须为正数".into()));
                }
                model.canvas.width = *width;
                model.canvas.height = *height;
                if let Some(o) = origin {
                    model.canvas.origin = *o;
                }
                Ok(vec![Effect::Canvas])
            }

            Command::NodeCreate { kind, name, parent, rect, rows, cols } => {
                if let Some(p) = parent {
                    if !model.has_node(p) {
                        return Err(CommandError::NodeNotFound(p.clone()));
                    }
                }
                let node = match kind {
                    NodeKind::Part => Node::part(name.clone(), parent.clone()),
                    NodeKind::Drawable => {
                        let mesh = match rect {
                            Some(r) => quad_mesh(*r),
                            None => Mesh::new(Vec::new(), Vec::new(), Vec::new()),
                        };
                        Node::drawable(name.clone(), parent.clone(), mesh)
                    }
                    NodeKind::WarpDeformer => {
                        let r = rect.unwrap_or_else(|| {
                            Rect::from_center_size(Vec2::ZERO, Vec2::splat(100.0))
                        });
                        Node::warp_deformer(
                            name.clone(),
                            parent.clone(),
                            rows.unwrap_or(2).max(1),
                            cols.unwrap_or(2).max(1),
                            r,
                        )
                    }
                    NodeKind::RotationDeformer => {
                        let mut n = Node::rotation_deformer(name.clone(), parent.clone());
                        n.rotation = Some(RotationData::default());
                        n
                    }
                };
                let created = model.add_node(node);
                Ok(vec![Effect::Structure, Effect::Node { node: created }])
            }
            Command::NodeDelete { node } => {
                if !model.has_node(node) {
                    return Err(CommandError::NodeNotFound(node.clone()));
                }
                model.remove_node(node);
                Ok(vec![Effect::Structure])
            }
            Command::NodeRename { node, name } => {
                node_mut(model, node)?.name = name.clone();
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::NodeReparent { node, parent } => {
                if let Some(p) = parent {
                    if !model.has_node(p) {
                        return Err(CommandError::NodeNotFound(p.clone()));
                    }
                    // 禁止把节点挂到自己的后代上
                    if model.ancestors(p).iter().any(|a| a == node) || p == node {
                        return Err(CommandError::Cycle(node.clone()));
                    }
                }
                let n = node_mut(model, node)?;
                n.parent = parent.clone();
                Ok(vec![Effect::Structure])
            }
            Command::NodeSetVisible { node, visible } => {
                node_mut(model, node)?.visible = *visible;
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::NodeSetLocked { node, locked } => {
                node_mut(model, node)?.locked = *locked;
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::NodeSetDrawOrder { node, draw_order } => {
                node_mut(model, node)?.draw_order = *draw_order;
                Ok(vec![Effect::Structure])
            }
            Command::NodeSetRotation { node, angle, position, scale, origin, handle_length } => {
                let n = node_mut(model, node)?;
                if n.kind != NodeKind::RotationDeformer {
                    return Err(CommandError::WrongKind(node.clone(), n.kind));
                }
                let r = n.rotation.get_or_insert_with(RotationData::default);
                if let Some(v) = angle {
                    r.angle = *v;
                }
                if let Some(v) = position {
                    r.position = *v;
                }
                if let Some(v) = scale {
                    r.scale = *v;
                }
                if let Some(v) = origin {
                    r.origin = *v;
                }
                if let Some(v) = handle_length {
                    r.handle_length = *v;
                }
                Ok(vec![Effect::Node { node: node.clone() }])
            }

            Command::MeshSet { node, vertices, uvs, indices } => {
                let d = drawable_mut(model, node)?;
                if vertices.len() != uvs.len() {
                    return Err(CommandError::Invalid("顶点数与 UV 数不一致".into()));
                }
                d.mesh = Mesh::new(vertices.clone(), uvs.clone(), indices.clone());
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::MeshSetVertices { node, vertices } => {
                let d = drawable_mut(model, node)?;
                if vertices.len() != d.mesh.vertices.len() {
                    return Err(CommandError::Invalid(format!(
                        "顶点数必须保持 {}（拓扑变更请用 mesh.set）",
                        d.mesh.vertices.len()
                    )));
                }
                d.mesh.vertices = vertices.clone();
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::MeshSetUvs { node, uvs } => {
                let d = drawable_mut(model, node)?;
                if uvs.len() != d.mesh.uvs.len() {
                    return Err(CommandError::Invalid(format!(
                        "UV 数必须保持 {}",
                        d.mesh.uvs.len()
                    )));
                }
                d.mesh.uvs = uvs.clone();
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::MeshSetIndices { node, indices } => {
                let d = drawable_mut(model, node)?;
                let count = d.mesh.vertices.len() as u32;
                if indices.iter().any(|i| *i >= count) {
                    return Err(CommandError::Invalid("索引超出顶点范围".into()));
                }
                d.mesh.indices = indices.clone();
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::WarpSetControlPoints { node, points } => {
                let w = warp_mut(model, node)?;
                let expected = ((w.rows + 1) * (w.cols + 1)) as usize;
                if points.len() != expected {
                    return Err(CommandError::Invalid(format!(
                        "控制点数量必须为 {expected}（当前网格 {}x{}）",
                        w.rows, w.cols
                    )));
                }
                w.control_points = points.clone();
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::WarpResize { node, rows, cols } => {
                let w = warp_mut(model, node)?;
                let rows = (*rows).max(1);
                let cols = (*cols).max(1);
                let rect = w
                    .rest_rect
                    .or_else(|| Rect::from_points(w.control_points.iter().copied()))
                    .unwrap_or_else(|| Rect::from_center_size(Vec2::ZERO, Vec2::splat(100.0)));
                let rebuilt = WarpData::from_rect(rows, cols, rect);
                w.rows = rows;
                w.cols = cols;
                w.rest_rect = Some(rect);
                w.control_points = rebuilt.control_points;
                Ok(vec![Effect::Node { node: node.clone() }])
            }

            Command::DrawableSetTexture { node, texture } => {
                if let Some(i) = texture {
                    if *i as usize >= model.textures.len() {
                        return Err(CommandError::TextureOutOfRange(*i));
                    }
                }
                drawable_mut(model, node)?.texture = *texture;
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::DrawableSetUvRect { node, uv_rect } => {
                drawable_mut(model, node)?.uv_rect = *uv_rect;
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::DrawableSetBlend { node, blend } => {
                drawable_mut(model, node)?.blend = *blend;
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::DrawableSetOpacity { node, opacity } => {
                if !opacity.is_finite() {
                    return Err(CommandError::Invalid("不透明度必须是有限数".into()));
                }
                drawable_mut(model, node)?.opacity = opacity.clamp(0.0, 1.0);
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::DrawableSetMasks { node, masks, inverted } => {
                for m in masks {
                    if !model.has_node(m) {
                        return Err(CommandError::NodeNotFound(m.clone()));
                    }
                    if m == node {
                        return Err(CommandError::Invalid("不能把自己作为遮罩".into()));
                    }
                }
                let d = drawable_mut(model, node)?;
                d.masks = masks.clone();
                d.inverted_mask = *inverted;
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::DrawableSetCulling { node, culling } => {
                drawable_mut(model, node)?.culling = *culling;
                Ok(vec![Effect::Node { node: node.clone() }])
            }

            Command::KeyformRecord {
                node,
                parameter,
                value,
                blend,
                vertices,
                opacity,
                control_points,
                rotation,
                draw_order,
            } => {
                if !value.is_finite() {
                    return Err(CommandError::Invalid("关键形参数值必须是有限数".into()));
                }
                if !model.parameters.iter().any(|p| &p.id == parameter) {
                    return Err(CommandError::ParameterNotFound(parameter.clone()));
                }
                let keyform = Keyform {
                    value: *value,
                    blend: blend.unwrap_or(BlendType::Normal),
                    vertices: vertices.clone(),
                    opacity: *opacity,
                    control_points: control_points.clone(),
                    rotation: rotation.clone(),
                    draw_order: *draw_order,
                };
                // 结构校验：长度必须与目标一致
                {
                    let n = model.node(node).ok_or_else(|| CommandError::NodeNotFound(node.clone()))?;
                    if let (Some(v), Some(m)) = (&keyform.vertices, n.mesh()) {
                        if v.len() != m.vertices.len() {
                            return Err(CommandError::Invalid(format!(
                                "关键形顶点数必须为 {}",
                                m.vertices.len()
                            )));
                        }
                    }
                    if let (Some(cp), Some(w)) = (&keyform.control_points, &n.warp) {
                        let expected = ((w.rows + 1) * (w.cols + 1)) as usize;
                        if cp.len() != expected {
                            return Err(CommandError::Invalid(format!(
                                "关键形控制点数必须为 {expected}"
                            )));
                        }
                    }
                }
                let n = model.node_mut(node).ok_or_else(|| CommandError::NodeNotFound(node.clone()))?;
                n.set_keyform(parameter, keyform);
                if let Some(p) = model.parameters.iter_mut().find(|p| &p.id == parameter) {
                    if !p.keys.iter().any(|k| (k - value).abs() < 1e-6) {
                        p.keys.push(*value);
                        p.keys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    }
                }
                Ok(vec![Effect::Node { node: node.clone() }])
            }
            Command::KeyformRemove { node, parameter, value } => {
                let n = model.node_mut(node).ok_or_else(|| CommandError::NodeNotFound(node.clone()))?;
                if let Some(list) = n.keyforms.get_mut(parameter) {
                    list.retain(|k| (k.value - value).abs() >= 1e-6);
                    if list.is_empty() {
                        n.keyforms.remove(parameter);
                    }
                }
                Ok(vec![Effect::Node { node: node.clone() }])
            }

            Command::ParameterAdd { id, name, min, max, default, group, weight, repeat } => {
                if !min.is_finite() || !max.is_finite() || !default.is_finite() {
                    return Err(CommandError::Invalid("参数范围必须是有限数".into()));
                }
                let pid = id.clone().unwrap_or_else(id::parameter_id);
                if model.parameter(&pid).is_some() {
                    return Err(CommandError::Invalid(format!("参数已存在：{pid}")));
                }
                let mut p = Parameter::new(pid, name.clone(), *min, *max, *default);
                p.group = group.clone();
                p.repeat = *repeat;
                if let Some(w) = weight {
                    p.weight = *w;
                }
                model.add_parameter(p);
                Ok(vec![Effect::Parameters])
            }
            Command::ParameterRemove { parameter } => {
                if model.parameter(parameter).is_none() {
                    return Err(CommandError::ParameterNotFound(parameter.clone()));
                }
                model.parameters.retain(|p| &p.id != parameter);
                for g in &mut model.parameter_groups {
                    g.parameters.retain(|p| p != parameter);
                }
                for n in &mut model.nodes {
                    n.keyforms.remove(parameter);
                }
                Ok(vec![Effect::Parameters, Effect::Structure])
            }
            Command::ParameterSetRange { parameter, min, max, default } => {
                if !min.is_finite() || !max.is_finite() || !default.is_finite() {
                    return Err(CommandError::Invalid("参数范围必须是有限数".into()));
                }
                let p = model
                    .parameters
                    .iter_mut()
                    .find(|p| &p.id == parameter)
                    .ok_or_else(|| CommandError::ParameterNotFound(parameter.clone()))?;
                *p = Parameter::new(p.id.clone(), p.name.clone(), *min, *max, *default);
                Ok(vec![Effect::Parameters])
            }

            Command::TextureAdd { asset, id, width, height, atlas } => {
                let mut t = TextureRef::new(asset.clone());
                if let Some(i) = id {
                    t.id = i.clone();
                }
                t.width = *width;
                t.height = *height;
                t.atlas = *atlas;
                model.add_texture(t);
                Ok(vec![Effect::Textures])
            }
            Command::TextureRemove { index } => {
                if *index as usize >= model.textures.len() {
                    return Err(CommandError::TextureOutOfRange(*index));
                }
                model.textures.remove(*index as usize);
                // 修正所有绘制对象的纹理下标
                for n in &mut model.nodes {
                    if let Some(d) = &mut n.drawable {
                        match d.texture {
                            Some(i) if i == *index => d.texture = None,
                            Some(i) if i > *index => d.texture = Some(i - 1),
                            _ => {}
                        }
                    }
                }
                Ok(vec![Effect::Textures, Effect::Structure])
            }

            Command::Batch { commands, .. } => {
                let mut effects = Vec::new();
                for c in commands {
                    for e in c.apply(model)? {
                        if !effects.contains(&e) {
                            effects.push(e);
                        }
                    }
                }
                Ok(effects)
            }
        }
    }
}

fn node_mut<'a>(model: &'a mut Model, id: &str) -> Result<&'a mut Node, CommandError> {
    model.node_mut(id).ok_or_else(|| CommandError::NodeNotFound(id.to_string()))
}

fn drawable_mut<'a>(
    model: &'a mut Model,
    id: &str,
) -> Result<&'a mut am_model::DrawableData, CommandError> {
    let node = node_mut(model, id)?;
    if node.kind != NodeKind::Drawable {
        return Err(CommandError::WrongKind(id.to_string(), node.kind));
    }
    node.drawable
        .as_mut()
        .ok_or_else(|| CommandError::Invalid(format!("绘制对象缺少网格数据：{id}")))
}

fn warp_mut<'a>(model: &'a mut Model, id: &str) -> Result<&'a mut WarpData, CommandError> {
    let node = node_mut(model, id)?;
    if node.kind != NodeKind::WarpDeformer {
        return Err(CommandError::WrongKind(id.to_string(), node.kind));
    }
    node.warp
        .as_mut()
        .ok_or_else(|| CommandError::Invalid(format!("自由变形缺少网格数据：{id}")))
}

/// 由矩形生成一个四边形网格（左下单 → 右上，逆时针）。
pub fn quad_mesh(rect: Rect) -> Mesh {
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
