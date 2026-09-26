//! 文档：模型 + 参数 + 撤销栈。
//!
//! 撤销采用**快照式**（每次可撤销操作前克隆模型）。理由：命令是任意组合的，
//! 逆向命令容易写错且难验证；模型体量在编辑器场景下完全可接受（几十 MB 量级才需要换方案）。
//! 快照深度可配置，超出后丢弃最旧的记录。

use crate::command::{Command, CommandError, Effect};
use am_eval::{evaluate, Evaluated, ParamStore};
use am_model::Model;

#[derive(Debug, Clone)]
struct Snapshot {
    model: Model,
    params: ParamStore,
    label: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error("命令 JSON 解析失败：{0}")]
    Json(#[from] serde_json::Error),
}

/// 一份可编辑的模型文档。
#[derive(Debug, Clone)]
pub struct Document {
    model: Model,
    params: ParamStore,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    max_depth: usize,
    revision: u64,
    dirty: bool,
}

/// 默认撤销深度。
pub const DEFAULT_MAX_DEPTH: usize = 128;

impl Document {
    /// 新建空文档。
    pub fn new(name: impl Into<String>) -> Self {
        Self::from_model(Model::new(name))
    }

    /// 由模型创建文档。
    pub fn from_model(model: Model) -> Self {
        let params = ParamStore::from_model(&model);
        Self {
            model,
            params,
            undo: Vec::new(),
            redo: Vec::new(),
            max_depth: DEFAULT_MAX_DEPTH,
            revision: 0,
            dirty: false,
        }
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    /// 逃生舱：绕过命令系统直接改模型（**不会**进入撤销栈）。
    /// 仅限导入/测试/引擎内部使用；编辑器必须走 [`Document::apply`]。
    pub fn model_mut(&mut self) -> &mut Model {
        self.dirty = true;
        self.revision += 1;
        &mut self.model
    }

    /// 取出模型（放弃文档状态）。
    pub fn into_model(self) -> Model {
        self.model
    }

    pub fn params(&self) -> &ParamStore {
        &self.params
    }

    /// 参数是运行时状态，**不进入撤销栈**。
    pub fn params_mut(&mut self) -> &mut ParamStore {
        &mut self.params
    }

    /// 写入参数（自动钳制到参数范围）。
    pub fn set_param(&mut self, id: &str, value: f32) -> bool {
        self.params.set_clamped(&self.model, id, value)
    }

    /// 求值当前文档。
    pub fn evaluate(&self) -> Evaluated {
        evaluate(&self.model, &self.params)
    }

    /// 应用一条命令（可撤销）。
    pub fn apply(&mut self, command: &Command) -> Result<Vec<Effect>, DocumentError> {
        let snapshot = Snapshot {
            model: self.model.clone(),
            params: self.params.clone(),
            label: command.op().to_string(),
        };
        match command.apply(&mut self.model) {
            Ok(effects) => {
                if effects.is_empty() {
                    return Ok(effects);
                }
                self.push_undo(snapshot);
                self.redo.clear();
                self.revision += 1;
                self.dirty = true;
                // 参数集合可能变化：把运行时取值与模型对齐
                self.params.sync_with_model(&self.model);
                Ok(effects)
            }
            Err(err) => {
                // 回滚到命令执行前
                self.model = snapshot.model;
                self.params = snapshot.params;
                Err(DocumentError::Command(err))
            }
        }
    }

    /// 从 JSON 应用命令（FFI 路径）。
    pub fn apply_json(&mut self, json: &str) -> Result<Vec<Effect>, DocumentError> {
        let command: Command = serde_json::from_str(json)?;
        self.apply(&command)
    }

    /// 撤销一步；返回被撤销的操作名。
    pub fn undo(&mut self) -> Option<String> {
        let snapshot = self.undo.pop()?;
        let label = snapshot.label.clone();
        let current = Snapshot {
            model: self.model.clone(),
            params: self.params.clone(),
            label: label.clone(),
        };
        self.model = snapshot.model;
        self.params = snapshot.params;
        self.redo.push(current);
        self.revision += 1;
        self.dirty = true;
        Some(label)
    }

    /// 重做一步；返回被重做的操作名。
    pub fn redo(&mut self) -> Option<String> {
        let snapshot = self.redo.pop()?;
        let label = snapshot.label.clone();
        let current = Snapshot {
            model: self.model.clone(),
            params: self.params.clone(),
            label: label.clone(),
        };
        self.model = snapshot.model;
        self.params = snapshot.params;
        self.undo.push(current);
        self.revision += 1;
        self.dirty = true;
        Some(label)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// 撤销栈里的操作名（旧 → 新）。
    pub fn undo_labels(&self) -> Vec<&str> {
        self.undo.iter().map(|s| s.label.as_str()).collect()
    }

    pub fn redo_labels(&self) -> Vec<&str> {
        self.redo.iter().map(|s| s.label.as_str()).collect()
    }

    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    pub fn set_max_depth(&mut self, depth: usize) {
        self.max_depth = depth.max(1);
        while self.undo.len() > self.max_depth {
            self.undo.remove(0);
        }
    }

    /// 每次成功修改都会自增；编辑器据此判断是否需要重建缓存。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    fn push_undo(&mut self, snapshot: Snapshot) {
        self.undo.push(snapshot);
        while self.undo.len() > self.max_depth {
            self.undo.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use am_math::{Rect, Vec2};
    use am_model::{BlendType, Id, Mesh, Node, NodeKind, TextureRef};

    fn quad(size: f32) -> Mesh {
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

    fn doc_with_part() -> (Document, Id) {
        let mut doc = Document::new("demo");
        let part = doc.model_mut().add_node(Node::part("Root", None));
        (doc, part)
    }

    #[test]
    fn create_node_via_command_is_undoable() {
        let (mut doc, part) = doc_with_part();
        let effects = doc
            .apply(&Command::NodeCreate {
                kind: NodeKind::Drawable,
                name: "Eye".into(),
                parent: Some(part.clone()),
                rect: Some(Rect::from_min_max(Vec2::ZERO, Vec2::splat(10.0))),
                rows: None,
                cols: None,
            })
            .unwrap();
        assert!(effects.contains(&Effect::Structure));
        assert_eq!(doc.model().nodes.len(), 2);
        assert!(doc.can_undo());

        assert_eq!(doc.undo().as_deref(), Some("node.create"));
        assert_eq!(doc.model().nodes.len(), 1);
        assert_eq!(doc.redo().as_deref(), Some("node.create"));
        assert_eq!(doc.model().nodes.len(), 2);
    }

    #[test]
    fn failed_command_leaves_model_untouched() {
        let (mut doc, _) = doc_with_part();
        let before = doc.model().clone();
        let err = doc.apply(&Command::NodeDelete { node: "ghost".into() }).unwrap_err();
        assert!(matches!(err, DocumentError::Command(CommandError::NodeNotFound(_))));
        assert_eq!(doc.model(), &before);
        assert!(!doc.can_undo(), "失败的命令不应留下撤销记录");
    }

    #[test]
    fn batch_is_one_undo_step() {
        let (mut doc, part) = doc_with_part();
        doc.apply(&Command::Batch {
            label: Some("一次拖拽".into()),
            commands: vec![
                Command::NodeCreate {
                    kind: NodeKind::Drawable,
                    name: "A".into(),
                    parent: Some(part.clone()),
                    rect: None,
                    rows: None,
                    cols: None,
                },
                Command::NodeCreate {
                    kind: NodeKind::Drawable,
                    name: "B".into(),
                    parent: Some(part.clone()),
                    rect: None,
                    rows: None,
                    cols: None,
                },
            ],
        })
        .unwrap();
        assert_eq!(doc.model().nodes.len(), 3);
        assert_eq!(doc.undo_labels(), vec!["batch"]);
        doc.undo().unwrap();
        assert_eq!(doc.model().nodes.len(), 1);
    }

    #[test]
    fn new_command_clears_redo() {
        let (mut doc, part) = doc_with_part();
        let create = Command::NodeCreate {
            kind: NodeKind::Part,
            name: "P".into(),
            parent: Some(part),
            rect: None,
            rows: None,
            cols: None,
        };
        doc.apply(&create).unwrap();
        doc.undo().unwrap();
        assert!(doc.can_redo());
        doc.apply(&create).unwrap();
        assert!(!doc.can_redo(), "新命令应清空重做栈");
    }

    #[test]
    fn undo_depth_is_capped() {
        let (mut doc, _) = doc_with_part();
        doc.set_max_depth(3);
        for i in 0..6 {
            doc.apply(&Command::NodeCreate {
                kind: NodeKind::Part,
                name: format!("P{i}"),
                parent: None,
                rect: None,
                rows: None,
                cols: None,
            })
            .unwrap();
        }
        assert_eq!(doc.undo_labels().len(), 3);
    }

    #[test]
    fn texture_remove_fixes_indices() {
        let (mut doc, _) = doc_with_part();
        doc.model_mut().add_texture(TextureRef::new("assets/a.png"));
        doc.model_mut().add_texture(TextureRef::new("assets/b.png"));
        doc.model_mut().add_texture(TextureRef::new("assets/c.png"));

        let ids: Vec<Id> = (0..3)
            .map(|i| {
                let id = doc.model_mut().add_node(Node::drawable(
                    format!("D{i}"),
                    None,
                    quad(1.0),
                ));
                doc.model_mut().node_mut(&id).unwrap().drawable.as_mut().unwrap().texture = Some(i);
                id
            })
            .collect();

        doc.apply(&Command::TextureRemove { index: 1 }).unwrap();
        assert_eq!(doc.model().textures.len(), 2);
        let tex = |i: usize| {
            doc.model()
                .node(&ids[i])
                .unwrap()
                .drawable
                .as_ref()
                .unwrap()
                .texture
        };
        assert_eq!(tex(0), Some(0));
        assert_eq!(tex(1), None, "被删除的纹理应置空");
        assert_eq!(tex(2), Some(1), "后续下标应前移");
    }

    #[test]
    fn keyform_record_validates_vertex_count() {
        let (mut doc, _) = doc_with_part();
        doc.apply(&Command::ParameterAdd {
            id: Some("p1".into()),
            name: "Angle".into(),
            min: -1.0,
            max: 1.0,
            default: 0.0,
            group: None,
            weight: None,
            repeat: false,
        })
        .unwrap();
        let eye = doc.model_mut().add_node(Node::drawable("Eye", None, quad(4.0)));
        let bad = doc.apply(&Command::KeyformRecord {
            node: eye.clone(),
            parameter: "p1".into(),
            value: 1.0,
            blend: None,
            vertices: Some(vec![Vec2::ZERO]),
            opacity: None,
            control_points: None,
            rotation: None,
            draw_order: None,
        });
        assert!(bad.is_err());

        doc.apply(&Command::KeyformRecord {
            node: eye.clone(),
            parameter: "p1".into(),
            value: 1.0,
            blend: Some(BlendType::Additive),
            vertices: Some(vec![Vec2::ZERO; 4]),
            opacity: Some(0.5),
            control_points: None,
            rotation: None,
            draw_order: None,
        })
        .unwrap();
        let node = doc.model().node(&eye).unwrap();
        assert_eq!(node.keyforms.get("p1").unwrap().len(), 1);
        assert!(doc.model().parameter("p1").unwrap().keys.contains(&1.0));
    }

    #[test]
    fn parameter_remove_cleans_keyforms() {
        let (mut doc, _) = doc_with_part();
        doc.apply(&Command::ParameterAdd {
            id: Some("p1".into()),
            name: "Angle".into(),
            min: -1.0,
            max: 1.0,
            default: 0.0,
            group: None,
            weight: None,
            repeat: false,
        })
        .unwrap();
        let eye = doc.model_mut().add_node(Node::drawable("Eye", None, quad(4.0)));
        doc.apply(&Command::KeyformRecord {
            node: eye.clone(),
            parameter: "p1".into(),
            value: 1.0,
            blend: None,
            vertices: Some(vec![Vec2::ZERO; 4]),
            opacity: None,
            control_points: None,
            rotation: None,
            draw_order: None,
        })
        .unwrap();
        doc.apply(&Command::ParameterRemove { parameter: "p1".into() }).unwrap();
        assert!(doc.model().parameter("p1").is_none());
        assert!(doc.model().node(&eye).unwrap().keyforms.is_empty());
        assert!(doc.params().try_get("p1").is_none(), "运行时取值也应清理");
    }

    #[test]
    fn node_delete_cleans_mask_references() {
        let (mut doc, _) = doc_with_part();
        let mask = doc.model_mut().add_node(Node::drawable("Mask", None, quad(4.0)));
        let target = doc.model_mut().add_node(Node::drawable("Target", None, quad(4.0)));
        doc.apply(&Command::DrawableSetMasks {
            node: target.clone(),
            masks: vec![mask.clone()],
            inverted: false,
        })
        .unwrap();
        doc.apply(&Command::NodeDelete { node: mask }).unwrap();
        assert!(doc
            .model()
            .node(&target)
            .unwrap()
            .drawable
            .as_ref()
            .unwrap()
            .masks
            .is_empty());
    }

    #[test]
    fn reparent_rejects_cycles() {
        let (mut doc, part) = doc_with_part();
        let child = doc.model_mut().add_node(Node::part("Child", Some(part.clone())));
        let err = doc.apply(&Command::NodeReparent {
            node: part.clone(),
            parent: Some(child),
        });
        assert!(matches!(err, Err(DocumentError::Command(CommandError::Cycle(_)))));
    }

    #[test]
    fn params_are_not_undoable() {
        let (mut doc, _) = doc_with_part();
        doc.apply(&Command::ParameterAdd {
            id: Some("p1".into()),
            name: "A".into(),
            min: 0.0,
            max: 1.0,
            default: 0.0,
            group: None,
            weight: None,
            repeat: false,
        })
        .unwrap();
        doc.clear_history();
        assert!(doc.set_param("p1", 0.7));
        assert_eq!(doc.params().get("p1"), 0.7);
        assert!(!doc.can_undo(), "参数变化不应产生撤销记录");
        assert!(!doc.set_param("ghost", 1.0));
    }

    #[test]
    fn apply_json_round_trip() {
        let (mut doc, part) = doc_with_part();
        let json = format!(
            r#"{{"op":"node_create","kind":"drawable","name":"Eye","parent":"{part}","rect":{{"min":[0.0,0.0],"max":[8.0,8.0]}}}}"#
        );
        doc.apply_json(&json).unwrap();
        assert_eq!(doc.model().nodes.len(), 2);
        assert!(doc.apply_json(r#"{"op":"nope"}"#).is_err());
    }

    #[test]
    fn dirty_flag_and_revision_track_changes() {
        let (mut doc, _) = doc_with_part();
        doc.mark_saved();
        assert!(!doc.is_dirty());
        let rev = doc.revision();
        doc.apply(&Command::SetCanvas { width: 100.0, height: 50.0, origin: None }).unwrap();
        assert!(doc.is_dirty());
        assert!(doc.revision() > rev);
        assert_eq!(doc.model().canvas.width, 100.0);
    }

    #[test]
    fn undo_redo_keeps_parameters_in_sync() {
        let (mut doc, _) = doc_with_part();
        doc.apply(&Command::ParameterAdd {
            id: Some("p1".into()),
            name: "A".into(),
            min: 0.0,
            max: 1.0,
            default: 0.25,
            group: None,
            weight: None,
            repeat: false,
        })
        .unwrap();
        assert_eq!(doc.params().get("p1"), 0.25);
        doc.undo().unwrap();
        assert!(doc.params().try_get("p1").is_none());
        doc.redo().unwrap();
        assert_eq!(doc.params().get("p1"), 0.25);
    }

    #[test]
    fn evaluate_reflects_document_state() {
        let (mut doc, _) = doc_with_part();
        let eye = doc.model_mut().add_node(Node::drawable("Eye", None, quad(4.0)));
        doc.model_mut().node_mut(&eye).unwrap().drawable.as_mut().unwrap().texture = None;
        let ev = doc.evaluate();
        assert_eq!(ev.scene.drawables.len(), 1);
        assert!(!ev.scene.drawables[0].is_renderable(), "没有纹理时不可渲染");
    }

    #[test]
    fn warp_resize_keeps_grid_consistent() {
        let (mut doc, _) = doc_with_part();
        let warp = doc.model_mut().add_node(Node::warp_deformer(
            "W",
            None,
            1,
            1,
            Rect::from_min_max(Vec2::ZERO, Vec2::splat(10.0)),
        ));
        doc.apply(&Command::WarpResize { node: warp.clone(), rows: 3, cols: 2 }).unwrap();
        let w = doc.model().node(&warp).unwrap().warp.as_ref().unwrap();
        assert_eq!((w.rows, w.cols), (3, 2));
        assert_eq!(w.control_points.len(), 4 * 3);
        assert!(doc
            .apply(&Command::WarpSetControlPoints { node: warp, points: vec![Vec2::ZERO] })
            .is_err());
    }
}
