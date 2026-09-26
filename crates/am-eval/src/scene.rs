//! 场景求值：把「模型 + 参数」变成「一份可直接渲染的画面」。
//!
//! 管线：`参数 → 关键形形变 → 节点变换链（仿射 + 自由变形） → 画布空间顶点`。
//!
//! 变换链的定义（关键）：每个节点持有 `C(node) = C(parent) ++ [L(node)]`，
//! 其中 `L` 是该节点自身的局部操作（旋转变形 = 仿射；自由变形 = 网格映射；其余 = 无）。
//! 应用时按列表顺序折叠，因此自由变形可以与旋转变形任意交错嵌套。

use crate::keyform::{evaluate_node, Deform};
use crate::params::ParamStore;
use crate::warp::WarpMap;
use am_math::{Mat3, Rect, Vec2};
use am_model::{BlendType, Canvas, Id, Model, Node, NodeKind, RotationValue};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// 变换链中的一步。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PointOp {
    Affine(Mat3),
    Warp(WarpMap),
}

impl PointOp {
    pub fn apply(&self, p: Vec2) -> Vec2 {
        match self {
            PointOp::Affine(m) => m.transform_point(p),
            PointOp::Warp(w) => w.map(p),
        }
    }

    pub fn as_affine(&self) -> Option<&Mat3> {
        match self {
            PointOp::Affine(m) => Some(m),
            PointOp::Warp(_) => None,
        }
    }
}

/// 折叠应用整条变换链。
///
/// 链条按**从根到叶**排列（`[祖先操作..., 自身操作]`），应用时从叶向根回放，
/// 与 [`fold_affine`] 的矩阵乘法顺序严格一致（`apply_ops == fold_affine.transform_point`）。
pub fn apply_ops(ops: &[PointOp], p: Vec2) -> Vec2 {
    let mut cur = p;
    for op in ops.iter().rev() {
        cur = op.apply(cur);
    }
    cur
}

/// 若整条链都是仿射，则折叠成一个矩阵。
pub fn fold_affine(ops: &[PointOp]) -> Option<Mat3> {
    let mut m = Mat3::IDENTITY;
    for op in ops {
        m = m * *op.as_affine()?;
    }
    Some(m)
}

/// 一条渲染指令。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawableInstance {
    pub node: Id,
    pub name: String,
    /// `Model::textures` 下标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uv_rect: Option<Rect>,
    /// 画布空间顶点。
    pub vertices: Vec<Vec2>,
    pub uvs: Vec<Vec2>,
    pub indices: Vec<u32>,
    /// 已乘上层级不透明度。
    pub opacity: f32,
    pub blend: BlendType,
    pub masks: Vec<Id>,
    pub inverted_mask: bool,
    pub culling: bool,
    pub visible: bool,
    /// 解析后的绘制顺序（已考虑关键形覆盖）。
    pub draw_order: i32,
}

impl DrawableInstance {
    /// 是否值得提交给渲染器。
    pub fn is_renderable(&self) -> bool {
        self.visible
            && self.opacity > 1e-4
            && self.texture.is_some()
            && self.vertices.len() >= 3
            && self.indices.len() >= 3
    }

    pub fn bounds(&self) -> Option<Rect> {
        Rect::from_points(self.vertices.iter().copied())
    }
}

/// 节点在画布上的状态（编辑器绘制手柄/网格、拾取用）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeView {
    pub id: Id,
    pub name: String,
    pub kind: NodeKind,
    pub visible: bool,
    /// 该节点生效的不透明度（绘制对象为自身与祖先的乘积）。
    pub opacity: f32,
    /// 祖先链无自由变形时给出的画布仿射矩阵（编辑器手柄直接用它）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world_affine: Option<Mat3>,
    /// 自由变形控制点（父空间）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_points_parent: Option<Vec<Vec2>>,
    /// 自由变形控制点（画布空间）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_points_canvas: Option<Vec<Vec2>>,
    /// 旋转变形轴心（画布空间）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_pivot_canvas: Option<Vec2>,
    /// 旋转变形手柄末端（画布空间）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_handle_canvas: Option<Vec2>,
    /// 该节点是否被关键形改动过。
    pub deformed: bool,
    pub draw_order: i32,
}

/// 求值结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub canvas: Canvas,
    pub drawables: Vec<DrawableInstance>,
    #[serde(default)]
    pub nodes: BTreeMap<Id, NodeView>,
}

impl Scene {
    /// 可渲染的绘制指令（已按绘制顺序）。
    pub fn renderables(&self) -> impl Iterator<Item = &DrawableInstance> {
        self.drawables.iter().filter(|d| d.is_renderable())
    }

    pub fn node(&self, id: &str) -> Option<&NodeView> {
        self.nodes.get(id)
    }

    /// 全部绘制对象的包围盒。
    pub fn bounds(&self) -> Option<Rect> {
        self.drawables.iter().filter_map(DrawableInstance::bounds).reduce(|a, b| a.union(b))
    }
}

/// 完整求值结果（含查询用的中间数据）。
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluated {
    pub scene: Scene,
    /// 每个节点的变换链。
    pub ops: BTreeMap<Id, Vec<PointOp>>,
    /// 每个节点的关键形形变。
    pub deforms: BTreeMap<Id, Deform>,
}

impl Evaluated {
    /// 把某节点局部空间中的点变换到画布空间。
    pub fn transform_point(&self, node: &str, p: Vec2) -> Vec2 {
        match self.ops.get(node) {
            Some(ops) => apply_ops(ops, p),
            None => p,
        }
    }

    /// 节点自身局部操作（不含祖先）。
    pub fn local_op(&self, node: &str) -> Option<&PointOp> {
        self.ops.get(node).and_then(|o| o.last())
    }

    pub fn node_view(&self, id: &str) -> Option<&NodeView> {
        self.scene.nodes.get(id)
    }

    pub fn drawable(&self, id: &str) -> Option<&DrawableInstance> {
        self.scene.drawables.iter().find(|d| d.node == id)
    }
}

// ---------------------------------------------------------------- 主流程

/// 求值一份模型。
pub fn evaluate(model: &Model, params: &ParamStore) -> Evaluated {
    // 1. 关键形形变
    let mut deforms: BTreeMap<Id, Deform> = BTreeMap::new();
    for node in &model.nodes {
        if node.keyforms.is_empty() {
            continue;
        }
        let d = evaluate_node(model, node, params);
        if !d.is_empty() {
            deforms.insert(node.id.clone(), d);
        }
    }

    // 2. 解析绘制顺序（关键形可覆盖）
    let mut resolved_order: BTreeMap<Id, i32> = BTreeMap::new();
    for node in &model.nodes {
        let order = deforms
            .get(&node.id)
            .and_then(|d| d.draw_order)
            .unwrap_or(node.draw_order);
        resolved_order.insert(node.id.clone(), order);
    }
    let order = resolved_paint_order(model, &resolved_order);

    // 3. 逐节点构建变换链
    let mut ops: BTreeMap<Id, Vec<PointOp>> = BTreeMap::new();
    for id in &order {
        let Some(node) = model.node(id) else { continue };
        let mut chain = node
            .parent
            .as_ref()
            .and_then(|p| ops.get(p))
            .cloned()
            .unwrap_or_default();
        if let Some(op) = local_op_of(node, &deforms) {
            chain.push(op);
        }
        ops.insert(id.clone(), chain);
    }

    // 4. 组装场景
    let mut drawables = Vec::new();
    let mut nodes = BTreeMap::new();

    for id in &order {
        let Some(node) = model.node(id) else { continue };
        let chain = ops.get(id).cloned().unwrap_or_default();
        let parent_chain = node
            .parent
            .as_ref()
            .and_then(|p| ops.get(p))
            .cloned()
            .unwrap_or_default();
        let visible = is_visible(model, node);
        let deform = deforms.get(id);

        if node.kind == NodeKind::Drawable {
            if let Some(d) = &node.drawable {
                let rest_vertices =
                    crate::keyform::RestValues::of(node).vertices;
                let vertices_local: Vec<Vec2> = match deform.and_then(|d| d.vertices.as_ref()) {
                    Some(v) if v.len() == rest_vertices.len() => v.clone(),
                    _ => rest_vertices.clone(),
                };
                let vertices: Vec<Vec2> =
                    vertices_local.iter().map(|p| apply_ops(&chain, *p)).collect();
                let opacity = chain_opacity(model, node, &deforms);
                drawables.push(DrawableInstance {
                    node: node.id.clone(),
                    name: node.name.clone(),
                    texture: d.texture,
                    uv_rect: d.uv_rect,
                    vertices,
                    uvs: d.mesh.uvs.clone(),
                    indices: d.mesh.indices.clone(),
                    opacity,
                    blend: d.blend,
                    masks: d.masks.clone(),
                    inverted_mask: d.inverted_mask,
                    culling: d.culling,
                    visible,
                    draw_order: resolved_order.get(id).copied().unwrap_or(node.draw_order),
                });
            }
        }

        // 编辑器视图数据
        let mut view = NodeView {
            id: node.id.clone(),
            name: node.name.clone(),
            kind: node.kind,
            visible,
            opacity: chain_opacity(model, node, &deforms),
            world_affine: fold_affine(&chain),
            control_points_parent: None,
            control_points_canvas: None,
            rotation_pivot_canvas: None,
            rotation_handle_canvas: None,
            deformed: deform.is_some(),
            draw_order: resolved_order.get(id).copied().unwrap_or(node.draw_order),
        };

        match node.kind {
            NodeKind::WarpDeformer => {
                if node.warp.is_some() {
                    let rest = crate::keyform::RestValues::of(node).control_points;
                    let points: Vec<Vec2> = match deform.and_then(|d| d.control_points.as_ref()) {
                        Some(v) if v.len() == rest.len() => v.clone(),
                        _ => rest.clone(),
                    };
                    view.control_points_canvas =
                        Some(points.iter().map(|p| apply_ops(&parent_chain, *p)).collect());
                    view.control_points_parent = Some(points);
                }
            }
            NodeKind::RotationDeformer => {
                if let Some(r) = &node.rotation {
                    let rest = RotationValue {
                        angle: r.angle,
                        position: r.position,
                        scale: r.scale,
                        origin: r.origin,
                    };
                    let rv = deform.map(|d| d.rotation_or(&rest)).unwrap_or(rest);
                    let pivot = rv.position + rv.origin;
                    view.rotation_pivot_canvas = Some(apply_ops(&parent_chain, pivot));
                    view.rotation_handle_canvas = Some(apply_ops(
                        &parent_chain,
                        pivot + Vec2::from_angle(rv.angle) * (r.handle_length * rv.scale.x),
                    ));
                }
            }
            _ => {}
        }

        nodes.insert(node.id.clone(), view);
    }

    Evaluated {
        scene: Scene { canvas: model.canvas.clone(), drawables, nodes },
        ops,
        deforms,
    }
}

/// 节点的局部操作。
fn local_op_of(node: &Node, deforms: &BTreeMap<Id, Deform>) -> Option<PointOp> {
    match node.kind {
        NodeKind::RotationDeformer => {
            let r = node.rotation.as_ref()?;
            let rest = RotationValue {
                angle: r.angle,
                position: r.position,
                scale: r.scale,
                origin: r.origin,
            };
            let rv = deforms
                .get(&node.id)
                .map(|d| d.rotation_or(&rest))
                .unwrap_or(rest);
            Some(PointOp::Affine(Mat3::from_trs_around(
                rv.position,
                rv.angle,
                rv.scale,
                rv.origin,
            )))
        }
        NodeKind::WarpDeformer => {
            let w = node.warp.as_ref()?;
            let rest = crate::keyform::RestValues::of(node).control_points;
            let points = match deforms.get(&node.id).and_then(|d| d.control_points.as_ref()) {
                Some(v) if v.len() == rest.len() => v.clone(),
                _ => rest,
            };
            let map = WarpMap::new(
                w.rows,
                w.cols,
                w.rest_rect
                    .or_else(|| Rect::from_points(points.iter().copied()))
                    .unwrap_or(Rect::ZERO),
                points,
            );
            Some(PointOp::Warp(map))
        }
        _ => None,
    }
}

fn is_visible(model: &Model, node: &Node) -> bool {
    if !node.visible {
        return false;
    }
    let mut cur = node.parent.clone();
    let mut guard = 0;
    while let Some(p) = cur {
        guard += 1;
        if guard > model.nodes.len() + 1 {
            return false;
        }
        match model.node(&p) {
            Some(parent) => {
                if !parent.visible {
                    return false;
                }
                cur = parent.parent.clone();
            }
            None => break,
        }
    }
    true
}

/// 节点生效的不透明度：自身（若是绘制对象）与祖先绘制对象之不透明度的乘积。
fn chain_opacity(model: &Model, node: &Node, deforms: &BTreeMap<Id, Deform>) -> f32 {
    let mut value = 1.0f32;
    let mut cur = Some(node);
    let mut guard = 0;
    while let Some(n) = cur {
        guard += 1;
        if guard > model.nodes.len() + 1 {
            break;
        }
        if let Some(d) = &n.drawable {
            let rest = d.opacity;
            let own = deforms
                .get(&n.id)
                .map(|df| df.opacity_or(rest))
                .unwrap_or(rest);
            value *= own;
        }
        cur = n.parent.as_ref().and_then(|p| model.node(p));
    }
    value.clamp(0.0, 1.0)
}

/// 考虑关键形覆盖后的绘制顺序。
fn resolved_paint_order(model: &Model, resolved: &BTreeMap<Id, i32>) -> Vec<Id> {
    let order_of = |n: &Node| resolved.get(&n.id).copied().unwrap_or(n.draw_order);
    let mut out = Vec::with_capacity(model.nodes.len());
    let mut visited: BTreeSet<Id> = BTreeSet::new();

    let mut roots: Vec<&Node> = model.nodes.iter().filter(|n| n.parent.is_none()).collect();
    roots.sort_by(|a, b| order_of(a).cmp(&order_of(b)).then_with(|| a.id.cmp(&b.id)));

    let mut stack: Vec<&Node> = roots.into_iter().rev().collect();
    while let Some(n) = stack.pop() {
        if !visited.insert(n.id.clone()) {
            continue; // 防御环
        }
        out.push(n.id.clone());
        let mut kids: Vec<&Node> = model
            .nodes
            .iter()
            .filter(|k| k.parent.as_deref() == Some(n.id.as_str()))
            .collect();
        kids.sort_by(|a, b| order_of(a).cmp(&order_of(b)).then_with(|| a.id.cmp(&b.id)));
        for k in kids.into_iter().rev() {
            stack.push(k);
        }
    }

    // 防御：父节点缺失或用环排除的节点，按声明顺序补齐
    for n in &model.nodes {
        if !visited.contains(&n.id) {
            visited.insert(n.id.clone());
            out.push(n.id.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_model::{
        DrawableData, Keyform, Mesh, Node, Parameter, RotationData, RotationValue,
    };

    fn quad_mesh(size: f32) -> Mesh {
        Mesh::new(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(size, 0.0),
                Vec2::new(size, size),
                Vec2::new(0.0, size),
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

    fn simple_model() -> (Model, Id, Id) {
        let mut m = Model::new("demo");
        let root = m.add_node(Node::part("Root", None));
        let eye = m.add_node(Node::drawable("Eye", Some(root.clone()), quad_mesh(10.0)));
        m.node_mut(&eye).unwrap().drawable.as_mut().unwrap().texture = Some(0);
        m.add_texture(am_model::TextureRef::new("assets/images/atlas.png"));
        (m, root, eye)
    }

    #[test]
    fn apply_ops_agrees_with_fold_affine() {
        // 根→叶顺序的纯仿射链：apply_ops 必须等价于 fold_affine 的矩阵
        let outer = Mat3::from_trs(Vec2::new(3.0, -1.0), 0.4, Vec2::splat(2.0));
        let inner = Mat3::from_trs(Vec2::new(-2.0, 5.0), -1.1, Vec2::new(0.5, 1.5));
        let ops = vec![PointOp::Affine(outer), PointOp::Affine(inner)];
        let folded = fold_affine(&ops).unwrap();
        for p in [Vec2::ZERO, Vec2::new(1.0, 2.0), Vec2::new(-7.5, 3.25)] {
            let a = apply_ops(&ops, p);
            let b = folded.transform_point(p);
            assert!((a - b).length() < 1e-5, "{p:?}: {a:?} vs {b:?}");
        }
    }

    #[test]
    fn rest_pose_is_identity() {
        let (m, _root, eye) = simple_model();
        let params = ParamStore::from_model(&m);
        let ev = evaluate(&m, &params);
        let d = ev.drawable(&eye).unwrap();
        assert_eq!(d.vertices[0], Vec2::ZERO);
        assert_eq!(d.vertices[2], Vec2::new(10.0, 10.0));
        assert!((d.opacity - 1.0).abs() < 1e-6);
        assert!(d.is_renderable());
        assert_eq!(ev.scene.drawables.len(), 1);
    }

    #[test]
    fn rotation_deformer_moves_children() {
        let mut m = Model::new("demo");
        let rot = m.add_node(Node::rotation_deformer("R", None));
        let child = m.add_node(Node::drawable("Eye", Some(rot.clone()), quad_mesh(1.0)));
        m.add_parameter(Parameter::new("p", "Angle", 0.0, 1.0, 0.0));
        m.node_mut(&rot).unwrap().rotation =
            Some(RotationData { angle: 0.0, origin: Vec2::ZERO, ..Default::default() });
        m.node_mut(&rot).unwrap().set_keyform("p", Keyform::with_rotation(0.0, RotationValue::default()));
        m.node_mut(&rot).unwrap().set_keyform(
            "p",
            Keyform::with_rotation(
                1.0,
                RotationValue { angle: std::f32::consts::FRAC_PI_2, ..Default::default() },
            ),
        );

        let mut params = ParamStore::from_model(&m);
        params.set("p", 1.0);
        let ev = evaluate(&m, &params);
        let d = ev.drawable(&child).unwrap();
        let p = d.vertices[1]; // (1,0) 旋转 90° → (0,1)
        assert!((p - Vec2::new(0.0, 1.0)).length() < 1e-5, "got {p:?}");
        assert!(ev.node_view(&rot).unwrap().deformed);
        assert!(ev.node_view(&rot).unwrap().rotation_handle_canvas.is_some());
    }

    #[test]
    fn warp_deformer_bends_children() {
        let mut m = Model::new("demo");
        let warp = m.add_node(Node::warp_deformer(
            "W",
            None,
            1,
            1,
            Rect::from_min_max(Vec2::ZERO, Vec2::splat(10.0)),
        ));
        let child = m.add_node(Node::drawable("Eye", Some(warp.clone()), quad_mesh(10.0)));
        // 静止网格 = 当前格点（恒等）
        let params = ParamStore::from_model(&m);
        let ev = evaluate(&m, &params);
        assert_eq!(ev.drawable(&child).unwrap().vertices[2], Vec2::new(10.0, 10.0));

        // 关键形把网格整体平移 → 子节点跟随
        m.add_parameter(Parameter::new("p", "Bend", 0.0, 1.0, 0.0));
        let base = m.node(&warp).unwrap().warp.clone().unwrap().control_points.clone();
        let moved: Vec<Vec2> = base.iter().map(|p| *p + Vec2::new(0.0, 5.0)).collect();
        m.node_mut(&warp).unwrap().set_keyform("p", Keyform::with_control_points(0.0, base));
        m.node_mut(&warp).unwrap().set_keyform("p", Keyform::with_control_points(1.0, moved));
        let mut params = ParamStore::from_model(&m);
        params.set("p", 1.0);
        let ev = evaluate(&m, &params);
        let v = ev.drawable(&child).unwrap().vertices[2];
        assert!((v - Vec2::new(10.0, 15.0)).length() < 1e-4, "got {v:?}");
    }

    #[test]
    fn nested_deformers_compose() {
        let mut m = Model::new("demo");
        let rot = m.add_node(Node::rotation_deformer("R", None));
        let warp = m.add_node(Node::warp_deformer(
            "W",
            Some(rot.clone()),
            1,
            1,
            Rect::from_min_max(Vec2::ZERO, Vec2::splat(2.0)),
        ));
        let child = m.add_node(Node::drawable("Eye", Some(warp.clone()), quad_mesh(2.0)));
        m.add_parameter(Parameter::new("p", "Angle", 0.0, 1.0, 0.0));
        m.node_mut(&rot).unwrap().set_keyform("p", Keyform::with_rotation(0.0, RotationValue::default()));
        m.node_mut(&rot).unwrap().set_keyform(
            "p",
            Keyform::with_rotation(
                1.0,
                RotationValue {
                    angle: std::f32::consts::FRAC_PI_2,
                    position: Vec2::ZERO,
                    scale: Vec2::ONE,
                    origin: Vec2::ZERO,
                },
            ),
        );
        let mut params = ParamStore::from_model(&m);
        params.set("p", 1.0);
        let ev = evaluate(&m, &params);
        let v = ev.drawable(&child).unwrap().vertices[2]; // (2,2) → 旋转90° → (-2,2)
        assert!((v - Vec2::new(-2.0, 2.0)).length() < 1e-4, "got {v:?}");
        // 祖先链含 warp 时不再给出单一仿射矩阵
        assert!(ev.node_view(&warp).unwrap().world_affine.is_none());
        assert!(ev.node_view(&rot).unwrap().world_affine.is_some());
    }

    #[test]
    fn invisible_ancestor_hides_descendants() {
        let (mut m, root, eye) = simple_model();
        m.node_mut(&root).unwrap().visible = false;
        let params = ParamStore::from_model(&m);
        let ev = evaluate(&m, &params);
        let d = ev.drawable(&eye).unwrap();
        assert!(!d.visible);
        assert!(!d.is_renderable());
        assert_eq!(ev.scene.renderables().count(), 0);
    }

    #[test]
    fn opacity_multiplies_along_ancestors() {
        let mut m = Model::new("demo");
        let outer = m.add_node(Node::drawable("Outer", None, quad_mesh(1.0)));
        let inner = m.add_node(Node::drawable("Inner", Some(outer.clone()), quad_mesh(1.0)));
        m.node_mut(&outer).unwrap().drawable.as_mut().unwrap().opacity = 0.5;
        m.node_mut(&inner).unwrap().drawable.as_mut().unwrap().opacity = 0.5;
        let params = ParamStore::from_model(&m);
        let ev = evaluate(&m, &params);
        assert!((ev.drawable(&inner).unwrap().opacity - 0.25).abs() < 1e-5);
    }

    #[test]
    fn keyform_can_override_draw_order() {
        let mut m = Model::new("demo");
        let a = m.add_node(Node::drawable("A", None, quad_mesh(1.0)));
        let b = m.add_node(Node::drawable("B", None, quad_mesh(1.0)));
        m.add_parameter(Parameter::new("p", "Swap", 0.0, 1.0, 0.0));
        // 默认 A(0) 在 B(1) 前；参数=1 时 B 提到前面
        m.node_mut(&b).unwrap().set_keyform("p", Keyform::with_draw_order(0.0, 1));
        m.node_mut(&b).unwrap().set_keyform("p", Keyform::with_draw_order(1.0, -1));

        let mut params = ParamStore::from_model(&m);
        let ev = evaluate(&m, &params);
        assert_eq!(ev.scene.drawables[0].node, a);
        params.set("p", 1.0);
        let ev = evaluate(&m, &params);
        assert_eq!(ev.scene.drawables[0].node, b);
    }

    #[test]
    fn transform_point_queries_local_space() {
        let mut m = Model::new("demo");
        let rot = m.add_node(Node::rotation_deformer("R", None));
        m.node_mut(&rot).unwrap().rotation = Some(RotationData {
            position: Vec2::new(5.0, 0.0),
            ..Default::default()
        });
        let params = ParamStore::from_model(&m);
        let ev = evaluate(&m, &params);
        assert_eq!(ev.transform_point(&rot, Vec2::ZERO), Vec2::new(5.0, 0.0));
        assert_eq!(ev.local_op(&rot).unwrap().as_affine().is_some(), true);
    }

    #[test]
    fn dangling_parent_nodes_are_still_evaluated() {
        let (mut m, _root, eye) = simple_model();
        m.node_mut(&eye).unwrap().parent = Some("ghost".into());
        let params = ParamStore::from_model(&m);
        let ev = evaluate(&m, &params);
        assert_eq!(ev.scene.drawables.len(), 1, "父节点缺失的节点也要有渲染指令");
    }

    #[test]
    fn cyclic_parents_do_not_hang() {
        let (mut m, root, eye) = simple_model();
        m.node_mut(&root).unwrap().parent = Some(eye.clone());
        let params = ParamStore::from_model(&m);
        let ev = evaluate(&m, &params);
        assert_eq!(ev.scene.drawables.len(), 1);
    }

    #[test]
    fn scene_bounds_cover_all_drawables() {
        let (m, _root, _eye) = simple_model();
        let params = ParamStore::from_model(&m);
        let ev = evaluate(&m, &params);
        let b = ev.scene.bounds().unwrap();
        assert_eq!(b.min, Vec2::ZERO);
        assert_eq!(b.max, Vec2::new(10.0, 10.0));
    }

    #[test]
    fn empty_scene_has_no_bounds() {
        let m = Model::new("empty");
        let params = ParamStore::from_model(&m);
        let ev = evaluate(&m, &params);
        assert!(ev.scene.bounds().is_none());
        assert_eq!(ev.scene.renderables().count(), 0);
        assert!(ev.scene.node("nope").is_none());
    }

    #[test]
    fn uv_and_indices_are_carried_through() {
        let (m, _root, eye) = simple_model();
        let params = ParamStore::from_model(&m);
        let ev = evaluate(&m, &params);
        let d = ev.drawable(&eye).unwrap();
        assert_eq!(d.uvs.len(), 4);
        assert_eq!(d.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(d.texture, Some(0));
        let _ = DrawableData::default();
    }
}
