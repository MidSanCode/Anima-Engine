//! 模型文档的结构校验（`spec/model.json` 自洽性）。
//!
//! 与 `am-format` 的**工程级校验**（资源哈希、目录镜像）互补：
//! 这里只关心模型内部是否自洽，能否被渲染与求值。

use crate::model::{Model, NodeKind};
use crate::id::{is_valid_id, Id};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// 一条模型结构问题。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelIssue {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<Id>,
}

impl ModelIssue {
    fn new(code: &str, message: impl Into<String>, node: Option<Id>) -> Self {
        Self { code: code.to_string(), message: message.into(), node }
    }
}

impl std::fmt::Display for ModelIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] ", self.code)?;
        if let Some(n) = &self.node {
            write!(f, "{n} ")?;
        }
        write!(f, "{}", self.message)
    }
}

/// 模型校验报告。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ModelReport {
    pub issues: Vec<ModelIssue>,
}

impl ModelReport {
    pub fn ok(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn codes(&self) -> Vec<&str> {
        self.issues.iter().map(|i| i.code.as_str()).collect()
    }

    pub fn summary(&self) -> String {
        if self.ok() {
            return "OK（0 个问题）".to_string();
        }
        self.issues.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("\n")
    }
}

impl Model {
    /// 全量结构校验。
    pub fn validate(&self) -> ModelReport {
        let mut rep = ModelReport::default();

        if !is_valid_id(&self.id) {
            rep.issues.push(ModelIssue::new("MODEL_ID_INVALID", format!("模型 id 非法: {:?}", self.id), None));
        }
        if self.name.trim().is_empty() {
            rep.issues.push(ModelIssue::new("MODEL_NAME_EMPTY", "模型名称为空", None));
        }

        // ---- 节点 id 唯一性与合法性
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for node in &self.nodes {
            if !is_valid_id(&node.id) {
                rep.issues.push(ModelIssue::new(
                    "NODE_ID_INVALID",
                    format!("节点 id 非法: {:?}", node.id),
                    Some(node.id.clone()),
                ));
            }
            if !seen.insert(&node.id) {
                rep.issues.push(ModelIssue::new(
                    "NODE_ID_DUPLICATE",
                    "节点 id 重复",
                    Some(node.id.clone()),
                ));
            }
            if node.name.trim().is_empty() {
                rep.issues.push(ModelIssue::new("NODE_NAME_EMPTY", "节点名称为空", Some(node.id.clone())));
            }
            // 类型与载荷一致
            match node.kind {
                NodeKind::Drawable => {
                    if node.drawable.is_none() {
                        rep.issues.push(ModelIssue::new(
                            "NODE_DATA_MISSING",
                            "kind 为 drawable 但缺少 drawable 数据",
                            Some(node.id.clone()),
                        ));
                    }
                }
                NodeKind::WarpDeformer => {
                    if node.warp.is_none() {
                        rep.issues.push(ModelIssue::new(
                            "NODE_DATA_MISSING",
                            "kind 为 warp_deformer 但缺少 warp 数据",
                            Some(node.id.clone()),
                        ));
                    }
                }
                NodeKind::RotationDeformer => {
                    if node.rotation.is_none() {
                        rep.issues.push(ModelIssue::new(
                            "NODE_DATA_MISSING",
                            "kind 为 rotation_deformer 但缺少 rotation 数据",
                            Some(node.id.clone()),
                        ));
                    }
                }
                NodeKind::Part => {
                    if node.drawable.is_some() || node.warp.is_some() || node.rotation.is_some() {
                        rep.issues.push(ModelIssue::new(
                            "NODE_DATA_UNEXPECTED",
                            "kind 为 part 但带有绘制/变形数据",
                            Some(node.id.clone()),
                        ));
                    }
                }
            }
        }

        // ---- 父子一致性（含成环检测）
        let ids: BTreeSet<&str> = self.nodes.iter().map(|n| n.id.as_str()).collect();
        for node in &self.nodes {
            if let Some(parent) = &node.parent {
                if parent == &node.id {
                    rep.issues.push(ModelIssue::new("NODE_SELF_PARENT", "节点以自身为父节点", Some(node.id.clone())));
                } else if !ids.contains(parent.as_str()) {
                    rep.issues.push(ModelIssue::new(
                        "NODE_PARENT_MISSING",
                        format!("父节点不存在: {parent}"),
                        Some(node.id.clone()),
                    ));
                }
            }
            // 沿父链上溯，超过节点总数必然成环
            let mut cur = node.parent.clone();
            let mut steps = 0usize;
            while let Some(p) = cur {
                steps += 1;
                if steps > self.nodes.len() {
                    rep.issues.push(ModelIssue::new(
                        "NODE_PARENT_CYCLE",
                        "父链存在环",
                        Some(node.id.clone()),
                    ));
                    break;
                }
                cur = self.node(&p).and_then(|n| n.parent.clone());
            }
        }

        // ---- 网格与关键形
        for node in &self.nodes {
            if let Some(d) = &node.drawable {
                if let Err(e) = d.mesh.validate() {
                    rep.issues.push(ModelIssue::new("MESH_INVALID", e, Some(node.id.clone())));
                }
                if !d.mesh.is_finite() {
                    rep.issues.push(ModelIssue::new(
                        "MESH_NOT_FINITE",
                        "网格包含非有限数值",
                        Some(node.id.clone()),
                    ));
                }
                if let Some(idx) = d.texture {
                    if self.texture(idx).is_none() {
                        rep.issues.push(ModelIssue::new(
                            "TEXTURE_INDEX_OUT_OF_RANGE",
                            format!("纹理下标 {idx} 超出范围"),
                            Some(node.id.clone()),
                        ));
                    }
                }
                for mask in &d.masks {
                    match self.node(mask) {
                        None => rep.issues.push(ModelIssue::new(
                            "MASK_NODE_MISSING",
                            format!("遮罩节点不存在: {mask}"),
                            Some(node.id.clone()),
                        )),
                        Some(m) if m.id == node.id => rep.issues.push(ModelIssue::new(
                            "MASK_SELF_REFERENCE",
                            "遮罩不能引用自身",
                            Some(node.id.clone()),
                        )),
                        _ => {}
                    }
                }
            }
            if let Some(w) = &node.warp {
                if !w.is_complete() {
                    rep.issues.push(ModelIssue::new(
                        "WARP_CONTROL_POINTS_INCOMPLETE",
                        format!(
                            "控制点数量 {} 与 {}x{} 网格要求的 {} 不一致",
                            w.control_points.len(),
                            w.rows + 1,
                            w.cols + 1,
                            w.expected_points()
                        ),
                        Some(node.id.clone()),
                    ));
                }
            }

            // 关键形
            let params: BTreeSet<&str> = self.parameters.iter().map(|p| p.id.as_str()).collect();
            let vertex_count = node.vertex_count();
            let expected_points = node.warp.as_ref().map(|w| w.expected_points());
            for (param, list) in &node.keyforms {
                if !params.contains(param.as_str()) {
                    rep.issues.push(ModelIssue::new(
                        "KEYFORM_ORPHAN_PARAMETER",
                        format!("关键形引用了不存在的参数: {param}"),
                        Some(node.id.clone()),
                    ));
                }
                if list.is_empty() {
                    rep.issues.push(ModelIssue::new(
                        "KEYFORM_LIST_EMPTY",
                        format!("参数 {param} 的关键形列表为空"),
                        Some(node.id.clone()),
                    ));
                }
                let mut sorted = true;
                for pair in list.windows(2) {
                    if pair[0].value > pair[1].value {
                        sorted = false;
                    }
                }
                if !sorted {
                    rep.issues.push(ModelIssue::new(
                        "KEYFORM_NOT_SORTED",
                        format!("参数 {param} 的关键形未按参数值升序"),
                        Some(node.id.clone()),
                    ));
                }
                for kf in list {
                    if !kf.value.is_finite() {
                        rep.issues.push(ModelIssue::new(
                            "KEYFORM_VALUE_NOT_FINITE",
                            format!("参数 {param} 的关键形取值不是有限数"),
                            Some(node.id.clone()),
                        ));
                    }
                    if let Some(v) = &kf.vertices {
                        if v.len() != vertex_count {
                            rep.issues.push(ModelIssue::new(
                                "KEYFORM_VERTEX_COUNT_MISMATCH",
                                format!(
                                    "参数 {param} @ {} 的顶点覆盖数量 {} 与网格顶点数 {vertex_count} 不一致",
                                    kf.value,
                                    v.len()
                                ),
                                Some(node.id.clone()),
                            ));
                        }
                    }
                    if let Some(cp) = &kf.control_points {
                        if let Some(expected) = expected_points {
                            if cp.len() != expected {
                                rep.issues.push(ModelIssue::new(
                                    "KEYFORM_CONTROL_POINT_COUNT_MISMATCH",
                                    format!(
                                        "参数 {param} @ {} 的控制点数量 {} 与期望 {expected} 不一致",
                                        kf.value,
                                        cp.len()
                                    ),
                                    Some(node.id.clone()),
                                ));
                            }
                        } else {
                            rep.issues.push(ModelIssue::new(
                                "KEYFORM_CONTROL_POINT_UNEXPECTED",
                                format!("参数 {param} 有控制点覆盖，但节点不是自由变形"),
                                Some(node.id.clone()),
                            ));
                        }
                    }
                    if kf.rotation.is_some() && node.kind != NodeKind::RotationDeformer {
                        rep.issues.push(ModelIssue::new(
                            "KEYFORM_ROTATION_UNEXPECTED",
                            format!("参数 {param} 有旋转变形覆盖，但节点不是旋转变形"),
                            Some(node.id.clone()),
                        ));
                    }
                }
            }
        }

        // ---- 参数
        let mut param_ids: BTreeSet<&str> = BTreeSet::new();
        let mut param_names: BTreeMap<&str, &str> = BTreeMap::new();
        for p in &self.parameters {
            if !is_valid_id(&p.id) {
                rep.issues.push(ModelIssue::new("PARAMETER_ID_INVALID", format!("参数 id 非法: {:?}", p.id), None));
            }
            if !param_ids.insert(&p.id) {
                rep.issues.push(ModelIssue::new("PARAMETER_ID_DUPLICATE", format!("参数 id 重复: {}", p.id), None));
            }
            if p.name.trim().is_empty() {
                rep.issues.push(ModelIssue::new("PARAMETER_NAME_EMPTY", format!("参数 {} 名称为空", p.id), None));
            }
            if let Some(prev) = param_names.insert(&p.name, &p.id) {
                rep.issues.push(ModelIssue::new(
                    "PARAMETER_NAME_DUPLICATE",
                    format!("参数名重复: {}（{} 与 {}）", p.name, prev, p.id),
                    None,
                ));
            }
            if p.min > p.max {
                rep.issues.push(ModelIssue::new(
                    "PARAMETER_RANGE_INVALID",
                    format!("参数 {} 的 min > max", p.id),
                    None,
                ));
            }
            if !(p.default.is_finite() && p.min.is_finite() && p.max.is_finite()) {
                rep.issues.push(ModelIssue::new(
                    "PARAMETER_RANGE_NOT_FINITE",
                    format!("参数 {} 的范围包含非有限数", p.id),
                    None,
                ));
            } else if p.default < p.min || p.default > p.max {
                rep.issues.push(ModelIssue::new(
                    "PARAMETER_DEFAULT_OUT_OF_RANGE",
                    format!("参数 {} 的默认值超出范围", p.id),
                    None,
                ));
            }
        }

        // ---- 参数分组引用
        for g in &self.parameter_groups {
            for pid in &g.parameters {
                if !param_ids.contains(pid.as_str()) {
                    rep.issues.push(ModelIssue::new(
                        "PARAMETER_GROUP_ORPHAN",
                        format!("分组 {} 引用了不存在的参数 {pid}", g.id),
                        None,
                    ));
                }
            }
        }

        // ---- 纹理
        let mut tex_ids: BTreeSet<&str> = BTreeSet::new();
        for t in &self.textures {
            if !tex_ids.insert(&t.id) {
                rep.issues.push(ModelIssue::new("TEXTURE_ID_DUPLICATE", format!("纹理 id 重复: {}", t.id), None));
            }
            if t.asset.trim().is_empty() {
                rep.issues.push(ModelIssue::new(
                    "TEXTURE_ASSET_EMPTY",
                    format!("纹理 {} 未指定资源路径", t.id),
                    None,
                ));
            } else if !t.asset.starts_with("assets/") {
                rep.issues.push(ModelIssue::new(
                    "TEXTURE_ASSET_NOT_IN_ASSETS",
                    format!("纹理资源应位于 assets/ 下: {}", t.asset),
                    None,
                ));
            }
        }

        rep
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Keyform, Mesh, Node, WarpData};
    use crate::param::Parameter;
    use am_math::{Rect, Vec2};

    fn quad() -> Mesh {
        Mesh::new(
            vec![Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)],
            vec![Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    fn clean_model() -> Model {
        let mut m = Model::new("clean");
        let root = m.add_node(Node::part("Root", None));
        let eye = m.add_node(Node::drawable("Eye", Some(root.clone()), quad()));
        let p = m.add_parameter(Parameter::new("p1", "AngleX", -30.0, 30.0, 0.0));
        m.node_mut(&eye).unwrap().set_keyform(&p, Keyform::with_vertices(0.0, vec![Vec2::ZERO; 4]));
        m
    }

    #[test]
    fn clean_model_passes() {
        let rep = clean_model().validate();
        assert!(rep.ok(), "{}", rep.summary());
    }

    #[test]
    fn dangling_parent_is_reported() {
        let mut m = clean_model();
        m.nodes[0].parent = Some("missing".into());
        let rep = m.validate();
        assert!(rep.codes().contains(&"NODE_PARENT_MISSING"));
    }

    #[test]
    fn cycles_are_detected_without_hanging() {
        let mut m = clean_model();
        m.nodes[0].parent = Some(m.nodes[1].id.clone());
        m.nodes[1].parent = Some(m.nodes[0].id.clone());
        let rep = m.validate();
        assert!(rep.codes().contains(&"NODE_PARENT_CYCLE"), "{}", rep.summary());
    }

    #[test]
    fn duplicate_ids_are_reported() {
        let mut m = clean_model();
        let dup = m.nodes[1].id.clone();
        m.nodes[0].id = dup;
        let rep = m.validate();
        assert!(rep.codes().contains(&"NODE_ID_DUPLICATE"));
    }

    #[test]
    fn keyform_vertex_count_mismatch_is_reported() {
        let mut m = clean_model();
        let eye = m.nodes[1].id.clone();
        m.node_mut(&eye).unwrap().set_keyform("p1", Keyform::with_vertices(0.0, vec![Vec2::ZERO; 3]));
        let rep = m.validate();
        assert!(rep.codes().contains(&"KEYFORM_VERTEX_COUNT_MISMATCH"));
    }

    #[test]
    fn orphan_keyform_parameter_is_reported() {
        let mut m = clean_model();
        let eye = m.nodes[1].id.clone();
        m.node_mut(&eye).unwrap().set_keyform("ghost", Keyform::marker(0.0));
        let rep = m.validate();
        assert!(rep.codes().contains(&"KEYFORM_ORPHAN_PARAMETER"));
    }

    #[test]
    fn mask_and_texture_references_are_checked() {
        let mut m = clean_model();
        let eye = m.nodes[1].id.clone();
        {
            let d = m.node_mut(&eye).unwrap().drawable.as_mut().unwrap();
            d.masks = vec!["missing".into(), eye.clone()];
            d.texture = Some(7);
        }
        let rep = m.validate();
        let codes = rep.codes();
        assert!(codes.contains(&"MASK_NODE_MISSING"));
        assert!(codes.contains(&"MASK_SELF_REFERENCE"));
        assert!(codes.contains(&"TEXTURE_INDEX_OUT_OF_RANGE"));
    }

    #[test]
    fn warp_point_count_is_checked() {
        let mut m = clean_model();
        let mut w = Node::warp_deformer("W", None, 1, 1, Rect::from_min_size(Vec2::ZERO, Vec2::splat(10.0)));
        w.warp = Some(WarpData { rows: 1, cols: 1, control_points: vec![Vec2::ZERO], show_grid: true });
        m.add_node(w);
        let rep = m.validate();
        assert!(rep.codes().contains(&"WARP_CONTROL_POINTS_INCOMPLETE"));
    }

    #[test]
    fn node_kind_and_data_must_agree() {
        let mut m = clean_model();
        m.nodes[1].drawable = None;
        let rep = m.validate();
        let codes = rep.codes();
        assert!(codes.contains(&"NODE_DATA_MISSING"));
    }

    #[test]
    fn parameter_range_and_default_are_checked() {
        let mut m = clean_model();
        let mut p = Parameter::new("p2", "AngleY", -1.0, 1.0, 0.0);
        p.min = 5.0;
        p.max = -5.0;
        m.add_parameter(p);
        let rep = m.validate();
        let codes = rep.codes();
        assert!(codes.contains(&"PARAMETER_RANGE_INVALID"));
    }

    #[test]
    fn duplicate_parameter_names_are_reported() {
        let mut m = clean_model();
        m.add_parameter(Parameter::new("p3", "AngleX", 0.0, 1.0, 0.0));
        let rep = m.validate();
        assert!(rep.codes().contains(&"PARAMETER_NAME_DUPLICATE"));
    }

    #[test]
    fn texture_asset_must_live_under_assets() {
        let mut m = clean_model();
        m.textures.push(crate::model::TextureRef::new("images/atlas.png"));
        let rep = m.validate();
        let codes = rep.codes();
        assert!(codes.contains(&"TEXTURE_ASSET_NOT_IN_ASSETS"));
    }
}
