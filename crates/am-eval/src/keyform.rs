//! 关键形求值：把「参数值」变成「节点形变」。
//!
//! 两层语义，缺一不可：
//!
//! 1. **单参数插值**：某参数的一组关键形按参数值排序，取值落在两个关键形之间时线性插值
//!    （`draw_order` 是阶梯式，不插值）。插值段的混合类型由**左端点**关键形决定。
//! 2. **多参数合成**：同一个节点可能被多个参数驱动，按模型参数声明顺序依次合成，
//!    每个关键形用自己的混合类型参与合成。静止值（网格本身、变形器当前值）是合成的基准。
//!
//! 合成规则（本项目自定义规范，务必与编辑器表现一致）：
//!
//! | 类型 | 顶点 / 控制点 / 位移 | 不透明度 |
//! | --- | --- | --- |
//! | `normal` | 直接替换 | 替换 |
//! | `additive` | `base + (v - rest)` | `base + v` |
//! | `screen` | 坐标上等同 `additive`（screen 只对不透明度有意义） | `1-(1-base)(1-v)` |
//! | `multiply` | `base * v / rest`（`rest ≈ 0` 时退化为 `additive`） | `base * v` |
//!
//! 参数自身的 `weight` 会把该参数的影响从静止值向关键形值插值，因此 `weight = 0`
//! 表示该参数完全不参与合成。

use crate::params::ParamStore;
use am_math::{lerp_angle, Vec2};
use am_model::{BlendType, Keyform, Model, Node, RotationValue};
use serde::{Deserialize, Serialize};

const EPS: f32 = 1e-6;

/// 关键形插值：返回参数值 `value` 处的关键形。
pub fn sample_keyforms(keys: &[Keyform], value: f32) -> Option<Keyform> {
    if keys.is_empty() {
        return None;
    }
    if value <= keys[0].value {
        return Some(keys[0].clone());
    }
    let last = keys.last()?;
    if value >= last.value {
        return Some(last.clone());
    }
    let idx = match keys
        .binary_search_by(|k| k.value.partial_cmp(&value).unwrap_or(std::cmp::Ordering::Equal))
    {
        Ok(i) => i,
        Err(i) => i - 1,
    };
    let a = &keys[idx];
    let b = &keys[idx + 1];
    let span = b.value - a.value;
    if span.abs() <= EPS {
        return Some(b.clone());
    }
    let t = ((value - a.value) / span).clamp(0.0, 1.0);
    Some(blend_keyforms(a, b, t))
}

/// 两个关键形之间的插值；混合类型取左端点。
pub fn blend_keyforms(a: &Keyform, b: &Keyform, t: f32) -> Keyform {
    Keyform {
        value: a.value + (b.value - a.value) * t,
        blend: a.blend,
        vertices: blend_vec2_opt(&a.vertices, &b.vertices, t),
        opacity: match (a.opacity, b.opacity) {
            (Some(x), Some(y)) => Some(x + (y - x) * t),
            (Some(x), None) => Some(x),
            (None, Some(y)) => Some(y),
            (None, None) => None,
        },
        control_points: blend_vec2_opt(&a.control_points, &b.control_points, t),
        rotation: blend_rotation_opt(a.rotation.as_ref(), b.rotation.as_ref(), t),
        // 绘制顺序是离散值：取左端点（阶梯）
        draw_order: a.draw_order.or(b.draw_order),
    }
}

fn blend_vec2_opt(a: &Option<Vec<Vec2>>, b: &Option<Vec<Vec2>>, t: f32) -> Option<Vec<Vec2>> {
    match (a, b) {
        (Some(x), Some(y)) if x.len() == y.len() => {
            Some(x.iter().zip(y.iter()).map(|(p, q)| p.lerp(*q, t)).collect())
        }
        (Some(x), _) => Some(x.clone()),
        (None, Some(y)) => Some(y.clone()),
        (None, None) => None,
    }
}

fn blend_rotation_opt(a: Option<&RotationValue>, b: Option<&RotationValue>, t: f32) -> Option<RotationValue> {
    match (a, b) {
        (Some(x), Some(y)) => Some(RotationValue {
            angle: lerp_angle(x.angle, y.angle, t),
            position: x.position.lerp(y.position, t),
            scale: x.scale.lerp(y.scale, t),
            origin: x.origin.lerp(y.origin, t),
        }),
        (Some(x), None) => Some(x.clone()),
        (None, Some(y)) => Some(y.clone()),
        (None, None) => None,
    }
}

/// 节点的静止值（关键形合成的基准）。
#[derive(Debug, Clone, PartialEq)]
pub struct RestValues {
    pub vertices: Vec<Vec2>,
    pub control_points: Vec<Vec2>,
    pub opacity: f32,
    pub rotation: RotationValue,
    pub draw_order: i32,
}

impl RestValues {
    pub fn of(node: &Node) -> Self {
        let mut rest = RestValues {
            vertices: Vec::new(),
            control_points: Vec::new(),
            opacity: 1.0,
            rotation: RotationValue::default(),
            draw_order: node.draw_order,
        };
        if let Some(d) = &node.drawable {
            rest.vertices = d.mesh.vertices.clone();
            rest.opacity = d.opacity;
        }
        if let Some(w) = &node.warp {
            rest.control_points = w.control_points.clone();
        }
        if let Some(r) = &node.rotation {
            rest.rotation = RotationValue {
                angle: r.angle,
                position: r.position,
                scale: r.scale,
                origin: r.origin,
            };
        }
        rest
    }
}

/// 一个节点在当前参数下的形变（只保存被关键形覆盖的字段）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Deform {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertices: Option<Vec<Vec2>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_points: Option<Vec<Vec2>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<RotationValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_order: Option<i32>,
}

impl Deform {
    pub fn is_empty(&self) -> bool {
        self.vertices.is_none()
            && self.control_points.is_none()
            && self.opacity.is_none()
            && self.rotation.is_none()
            && self.draw_order.is_none()
    }

    /// 合成一个关键形贡献。
    pub fn apply(&mut self, kf: &Keyform, rest: &RestValues, weight: f32) {
        let w = if weight.is_finite() { weight.clamp(0.0, 1.0) } else { 1.0 };
        if let Some(v) = &kf.vertices {
            self.vertices = Some(combine_vec2(
                self.vertices.take(),
                &rest.vertices,
                v,
                kf.blend,
                w,
            ));
        }
        if let Some(cp) = &kf.control_points {
            self.control_points = Some(combine_vec2(
                self.control_points.take(),
                &rest.control_points,
                cp,
                kf.blend,
                w,
            ));
        }
        if let Some(o) = kf.opacity {
            self.opacity = Some(combine_scalar(self.opacity, rest.opacity, o, kf.blend, w));
        }
        if let Some(r) = &kf.rotation {
            self.rotation = Some(combine_rotation(
                self.rotation.take(),
                &rest.rotation,
                r,
                kf.blend,
                w,
            ));
        }
        if let Some(d) = kf.draw_order {
            self.draw_order =
                Some(combine_int(self.draw_order, rest.draw_order, d, kf.blend, w));
        }
    }

    /// 顶点（缺省时回退到静止值）。
    pub fn vertices_or<'a>(&'a self, rest: &'a [Vec2]) -> &'a [Vec2] {
        self.vertices.as_deref().unwrap_or(rest)
    }

    pub fn control_points_or<'a>(&'a self, rest: &'a [Vec2]) -> &'a [Vec2] {
        self.control_points.as_deref().unwrap_or(rest)
    }

    pub fn opacity_or(&self, rest: f32) -> f32 {
        self.opacity.unwrap_or(rest)
    }

    pub fn rotation_or(&self, rest: &RotationValue) -> RotationValue {
        self.rotation.clone().unwrap_or_else(|| rest.clone())
    }

    pub fn draw_order_or(&self, rest: i32) -> i32 {
        self.draw_order.unwrap_or(rest)
    }
}

fn combine_vec2(
    acc: Option<Vec<Vec2>>,
    rest: &[Vec2],
    contribution: &[Vec2],
    blend: BlendType,
    weight: f32,
) -> Vec<Vec2> {
    let base = acc.unwrap_or_else(|| rest.to_vec());
    if contribution.len() != rest.len() || base.len() != rest.len() {
        // 结构不匹配（长度不一致）时忽略该贡献；`am-model` 的校验会单独报告
        return base;
    }
    let mut out = Vec::with_capacity(rest.len());
    for i in 0..rest.len() {
        let r = rest[i];
        let c = r.lerp(contribution[i], weight);
        out.push(match blend {
            BlendType::Normal => c,
            BlendType::Additive | BlendType::Screen => base[i] + (c - r),
            BlendType::Multiply => {
                let b = &base[i];
                Vec2::new(mul_component(b.x, c.x, r.x), mul_component(b.y, c.y, r.y))
            }
        });
    }
    out
}

fn mul_component(base: f32, value: f32, rest: f32) -> f32 {
    if rest.abs() <= EPS {
        base + (value - rest)
    } else {
        base * value / rest
    }
}

fn combine_scalar(acc: Option<f32>, rest: f32, contribution: f32, blend: BlendType, weight: f32) -> f32 {
    let base = acc.unwrap_or(rest);
    let c = rest + (contribution - rest) * weight;
    let v = match blend {
        BlendType::Normal => c,
        BlendType::Multiply => base * c,
        BlendType::Screen => 1.0 - (1.0 - base) * (1.0 - c),
        BlendType::Additive => base + c,
    };
    v.clamp(0.0, 1.0)
}

fn combine_int(acc: Option<i32>, rest: i32, contribution: i32, blend: BlendType, weight: f32) -> i32 {
    let base = acc.unwrap_or(rest);
    let c = rest + ((contribution - rest) as f32 * weight).round() as i32;
    match blend {
        BlendType::Normal => c,
        _ => base + (c - rest),
    }
}

fn combine_rotation(
    acc: Option<RotationValue>,
    rest: &RotationValue,
    contribution: &RotationValue,
    blend: BlendType,
    weight: f32,
) -> RotationValue {
    let base = acc.unwrap_or_else(|| rest.clone());
    let c = RotationValue {
        angle: lerp_angle(rest.angle, contribution.angle, weight),
        position: rest.position.lerp(contribution.position, weight),
        scale: rest.scale.lerp(contribution.scale, weight),
        origin: rest.origin.lerp(contribution.origin, weight),
    };
    match blend {
        BlendType::Normal => c,
        BlendType::Additive | BlendType::Screen => RotationValue {
            angle: base.angle + (c.angle - rest.angle),
            position: base.position + (c.position - rest.position),
            scale: base.scale + (c.scale - rest.scale),
            origin: base.origin + (c.origin - rest.origin),
        },
        BlendType::Multiply => RotationValue {
            angle: base.angle * c.angle / if rest.angle.abs() <= EPS { 1.0 } else { rest.angle },
            position: Vec2::new(
                mul_component(base.position.x, c.position.x, rest.position.x),
                mul_component(base.position.y, c.position.y, rest.position.y),
            ),
            scale: Vec2::new(
                mul_component(base.scale.x, c.scale.x, rest.scale.x),
                mul_component(base.scale.y, c.scale.y, rest.scale.y),
            ),
            origin: Vec2::new(
                mul_component(base.origin.x, c.origin.x, rest.origin.x),
                mul_component(base.origin.y, c.origin.y, rest.origin.y),
            ),
        },
    }
}

/// 求一个节点在当前参数下的形变。
///
/// 参数按 `model.parameters` 的声明顺序参与合成，保证结果可复现。
pub fn evaluate_node(model: &Model, node: &Node, params: &ParamStore) -> Deform {
    let mut deform = Deform::default();
    if node.keyforms.is_empty() {
        return deform;
    }
    let rest = RestValues::of(node);
    for param in &model.parameters {
        let Some(keys) = node.keyforms.get(&param.id) else {
            continue;
        };
        let Some(kf) = sample_keyforms(keys, params.get(&param.id)) else {
            continue;
        };
        deform.apply(&kf, &rest, param.weight);
    }
    deform
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_model::{DrawableData, Mesh, Node, Parameter, RotationData};
    use std::collections::BTreeMap;

    fn vec2(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y)
    }

    fn vertex_keys() -> Vec<Keyform> {
        vec![
            Keyform::with_vertices(-1.0, vec![vec2(-1.0, 0.0), vec2(0.0, 0.0)]),
            Keyform::with_vertices(1.0, vec![vec2(1.0, 0.0), vec2(2.0, 0.0)]),
        ]
    }

    #[test]
    fn single_parameter_interpolates_between_keys() {
        let keys = vertex_keys();
        let k = sample_keyforms(&keys, 0.0).unwrap();
        assert_eq!(k.vertices.unwrap(), vec![vec2(0.0, 0.0), vec2(1.0, 0.0)]);
        let k = sample_keyforms(&keys, -2.0).unwrap();
        assert_eq!(k.vertices.unwrap()[0], vec2(-1.0, 0.0), "越界取下界关键形");
        let k = sample_keyforms(&keys, 2.0).unwrap();
        assert_eq!(k.vertices.unwrap()[0], vec2(1.0, 0.0), "越界取上界关键形");
    }

    #[test]
    fn empty_keyform_list_returns_none() {
        assert!(sample_keyforms(&[], 0.0).is_none());
    }

    #[test]
    fn duplicate_key_values_do_not_divide_by_zero() {
        let keys = vec![
            Keyform::with_vertices(0.0, vec![vec2(0.0, 0.0)]),
            Keyform::with_vertices(0.0, vec![vec2(5.0, 0.0)]),
        ];
        assert!(sample_keyforms(&keys, 0.0).is_some());
    }

    #[test]
    fn rotation_interpolation_takes_short_arc() {
        use std::f32::consts::PI;
        let a = Keyform::with_rotation(
            -1.0,
            RotationValue { angle: PI - 0.1, ..Default::default() },
        );
        let b = Keyform::with_rotation(
            1.0,
            RotationValue { angle: -PI + 0.1, ..Default::default() },
        );
        let mid = blend_keyforms(&a, &b, 0.5);
        let angle = mid.rotation.unwrap().angle;
        assert!(angle.abs() > PI - 0.2, "应走最短弧: {angle}");
    }

    #[test]
    fn draw_order_is_step_not_interpolated() {
        let a = Keyform::with_draw_order(0.0, 3);
        let b = Keyform::with_draw_order(10.0, 9);
        assert_eq!(blend_keyforms(&a, &b, 0.9).draw_order, Some(3));
    }

    #[test]
    fn blend_type_of_segment_follows_left_key() {
        let mut a = Keyform::with_opacity(0.0, 0.0);
        a.blend = BlendType::Multiply;
        let b = Keyform::with_opacity(1.0, 1.0);
        assert_eq!(blend_keyforms(&a, &b, 0.5).blend, BlendType::Multiply);
    }

    fn node_with_mesh() -> Node {
        let mut node = Node::drawable(
            "Eye",
            None,
            Mesh::new(
                vec![vec2(0.0, 0.0), vec2(1.0, 0.0)],
                vec![vec2(0.0, 0.0), vec2(1.0, 0.0)],
                vec![],
            ),
        );
        node.drawable = Some(DrawableData { mesh: node.mesh().unwrap().clone(), ..Default::default() });
        node
    }

    fn model_with_one_param() -> Model {
        let mut m = Model::new("m");
        m.add_parameter(Parameter::new("p1", "AngleX", -1.0, 1.0, 0.0));
        m
    }

    #[test]
    fn add_parameter_key_registers_every_target() {
        // 覆盖 `Model::add_parameter_key` 与求值的配合
        let model = model_with_one_param();
        let mut store = ParamStore::from_model(&model);
        store.set("p1", 0.5);

        let mut node = node_with_mesh();
        node.set_keyform("p1", Keyform::with_vertices(-1.0, vec![vec2(-1.0, 0.0), vec2(-1.0, 0.0)]));
        node.set_keyform("p1", Keyform::with_vertices(1.0, vec![vec2(1.0, 0.0), vec2(1.0, 0.0)]));

        let d = evaluate_node(&model, &node, &store);
        assert_eq!(d.vertices.unwrap()[0], vec2(0.5, 0.0));
    }

    #[test]
    fn node_without_keyforms_is_untouched() {
        let model = model_with_one_param();
        let store = ParamStore::from_model(&model);
        let node = node_with_mesh();
        let d = evaluate_node(&model, &node, &store);
        assert!(d.is_empty());
    }

    #[test]
    fn additive_blend_accumulates_offsets() {
        let mut model = Model::new("m");
        model.add_parameter(Parameter::new("p1", "A", 0.0, 1.0, 0.0));
        model.add_parameter(Parameter::new("p2", "B", 0.0, 1.0, 0.0));
        let mut store = ParamStore::from_model(&model);
        store.set("p1", 1.0);
        store.set("p2", 1.0);

        let mut node = node_with_mesh();
        let mut k1 = Keyform::with_vertices(1.0, vec![vec2(1.0, 0.0), vec2(1.0, 0.0)]);
        k1.blend = BlendType::Additive;
        node.set_keyform("p1", Keyform::with_vertices(0.0, vec![vec2(0.0, 0.0), vec2(0.0, 0.0)]));
        node.set_keyform("p1", k1);
        let mut k2 = Keyform::with_vertices(1.0, vec![vec2(2.0, 0.0), vec2(2.0, 0.0)]);
        k2.blend = BlendType::Additive;
        node.set_keyform("p2", Keyform::with_vertices(0.0, vec![vec2(0.0, 0.0), vec2(0.0, 0.0)]));
        node.set_keyform("p2", k2);

        let d = evaluate_node(&model, &node, &store);
        // p1: additive → 1.0；p2: additive → +2.0 → 3.0
        assert_eq!(d.vertices.unwrap()[0], vec2(3.0, 0.0));
    }

    #[test]
    fn multiply_blend_scales_relative_to_rest() {
        let mut model = Model::new("m");
        model.add_parameter(Parameter::new("p1", "A", 0.0, 1.0, 0.0));
        let mut store = ParamStore::from_model(&model);
        store.set("p1", 1.0);

        let mut node = Node::drawable(
            "Eye",
            None,
            Mesh::new(
                vec![vec2(2.0, 0.0), vec2(4.0, 0.0)],
                vec![vec2(0.0, 0.0), vec2(1.0, 0.0)],
                vec![],
            ),
        );
        node.drawable = Some(DrawableData { mesh: node.mesh().unwrap().clone(), ..Default::default() });
        let mut k = Keyform::with_vertices(1.0, vec![vec2(4.0, 0.0), vec2(8.0, 0.0)]);
        k.blend = BlendType::Multiply;
        node.set_keyform("p1", Keyform::with_vertices(0.0, vec![vec2(2.0, 0.0), vec2(4.0, 0.0)]));
        node.set_keyform("p1", k);

        let d = evaluate_node(&model, &node, &store);
        assert_eq!(d.vertices.unwrap(), vec![vec2(4.0, 0.0), vec2(8.0, 0.0)]);
    }

    #[test]
    fn multiply_degenerates_to_additive_at_zero_rest() {
        let mut model = Model::new("m");
        model.add_parameter(Parameter::new("p1", "A", 0.0, 1.0, 0.0));
        let mut store = ParamStore::from_model(&model);
        store.set("p1", 1.0);
        // 静止顶点 (1,0) 与 (0,0)：前者走比例，后者退化
        let mut node = Node::drawable(
            "Eye",
            None,
            Mesh::new(
                vec![vec2(1.0, 0.0), vec2(0.0, 0.0)],
                vec![vec2(0.0, 0.0), vec2(1.0, 0.0)],
                vec![],
            ),
        );
        node.drawable = Some(DrawableData { mesh: node.mesh().unwrap().clone(), ..Default::default() });
        let mut k = Keyform::with_vertices(1.0, vec![vec2(2.0, 0.0), vec2(3.0, 0.0)]);
        k.blend = BlendType::Multiply;
        node.set_keyform("p1", Keyform::with_vertices(0.0, vec![vec2(1.0, 0.0), vec2(0.0, 0.0)]));
        node.set_keyform("p1", k);
        let d = evaluate_node(&model, &node, &store);
        let v = d.vertices.unwrap();
        assert_eq!(v[0], vec2(2.0, 0.0), "rest.x=1 时按比例 1*2/1");
        assert_eq!(v[1], vec2(3.0, 0.0), "rest.x=0 时退化为 additive 0+(3-0)");
    }

    #[test]
    fn opacity_blend_modes() {
        let mut model = Model::new("m");
        model.add_parameter(Parameter::new("p", "A", 0.0, 1.0, 0.0));
        let mut store = ParamStore::from_model(&model);
        store.set("p", 1.0);

        let mut node = node_with_mesh();
        node.drawable.as_mut().unwrap().opacity = 0.5;
        let mut k = Keyform::with_opacity(1.0, 0.5);
        k.blend = BlendType::Multiply;
        node.set_keyform("p", Keyform::with_opacity(0.0, 1.0));
        node.set_keyform("p", k);
        let d = evaluate_node(&model, &node, &store);
        // 静止不透明度 0.5，关键形 0.5，multiply → 0.25
        assert!((d.opacity.unwrap() - 0.25).abs() < 1e-5);

        let mut node2 = node_with_mesh();
        node2.drawable.as_mut().unwrap().opacity = 0.5;
        let mut k2 = Keyform::with_opacity(1.0, 0.5);
        k2.blend = BlendType::Screen;
        node2.set_keyform("p", Keyform::with_opacity(0.0, 0.5));
        node2.set_keyform("p", k2);
        let d2 = evaluate_node(&model, &node2, &store);
        assert!((d2.opacity.unwrap() - 0.75).abs() < 1e-5);
    }

    #[test]
    fn parameter_weight_scales_influence() {
        let mut model = Model::new("m");
        let mut p = Parameter::new("p1", "A", 0.0, 1.0, 0.0);
        p.weight = 0.5;
        model.add_parameter(p);
        let mut store = ParamStore::from_model(&model);
        store.set("p1", 1.0);

        let mut node = node_with_mesh();
        node.set_keyform("p1", Keyform::with_vertices(0.0, vec![vec2(0.0, 0.0), vec2(0.0, 0.0)]));
        node.set_keyform("p1", Keyform::with_vertices(1.0, vec![vec2(2.0, 0.0), vec2(2.0, 0.0)]));
        let d = evaluate_node(&model, &node, &store);
        assert_eq!(d.vertices.unwrap()[0], vec2(1.0, 0.0), "weight=0.5 应只走一半");
    }

    #[test]
    fn mismatched_vertex_count_is_ignored_not_panicking() {
        let model = model_with_one_param();
        let mut store = ParamStore::from_model(&model);
        store.set("p1", 1.0);
        let mut node = node_with_mesh();
        node.set_keyform("p1", Keyform::with_vertices(1.0, vec![vec2(1.0, 1.0)])); // 长度不符
        let d = evaluate_node(&model, &node, &store);
        // 应回退到静止顶点，而不是崩溃
        assert_eq!(d.vertices.unwrap().len(), 2);
    }

    #[test]
    fn rotation_only_applies_to_rotation_deformers() {
        let mut model = Model::new("m");
        model.add_parameter(Parameter::new("p", "A", 0.0, 1.0, 0.0));
        let mut store = ParamStore::from_model(&model);
        store.set("p", 1.0);

        let mut node = Node::rotation_deformer("R", None);
        node.rotation = Some(RotationData { angle: 0.0, ..Default::default() });
        node.set_keyform("p", Keyform::with_rotation(0.0, RotationValue::default()));
        node.set_keyform(
            "p",
            Keyform::with_rotation(1.0, RotationValue { angle: 0.5, ..Default::default() }),
        );
        let d = evaluate_node(&model, &node, &store);
        assert!((d.rotation.unwrap().angle - 0.5).abs() < 1e-5);
    }

    #[test]
    fn deform_helpers_fall_back_to_rest() {
        let d = Deform::default();
        let rest = [vec2(1.0, 2.0)];
        assert_eq!(d.vertices_or(&rest), &rest);
        assert_eq!(d.control_points_or(&rest), &rest);
        assert_eq!(d.opacity_or(0.3), 0.3);
        assert_eq!(d.draw_order_or(7), 7);
        assert_eq!(d.rotation_or(&RotationValue::default()).angle, 0.0);
    }

    #[test]
    fn rest_values_read_from_node() {
        let mut node = node_with_mesh();
        node.drawable.as_mut().unwrap().opacity = 0.75;
        node.draw_order = 4;
        let rest = RestValues::of(&node);
        assert_eq!(rest.vertices.len(), 2);
        assert_eq!(rest.opacity, 0.75);
        assert_eq!(rest.draw_order, 4);
        assert!(rest.control_points.is_empty());

        let mut keyforms = BTreeMap::new();
        keyforms.insert("p".to_string(), vec![Keyform::marker(0.0)]);
        node.keyforms = keyforms;
        assert!(!RestValues::of(&node).vertices.is_empty());
    }
}
