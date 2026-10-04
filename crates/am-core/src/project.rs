//! 描述层（`spec/`）与工程对象之间的桥接。
//!
//! `am-format` 只认识 JSON 与文件路径，不认识模型类型；`am-model` 只有纯数据，
//! 不做 I/O。读写整份描述层这件事需要同时知道两边，因此放在门面层。

use am_format::{FormatError, Project, SPEC_DIR};
use am_model::{
    Expression, Model, Motion, PhysicsSettings, Pose, ProjectConfig, Spec, ANIMATIONS_DIR,
    EXPRESSIONS_DIR, MOTIONS_DIR, SPEC_CONFIG_FILE, SPEC_MODEL_FILE, SPEC_PHYSICS_FILE,
    SPEC_POSE_FILE, SPEC_SETTINGS_FILE,
};
use std::path::PathBuf;

/// 从工程目录读取完整描述层。
///
/// `spec/model.json` 缺失视为损坏工程；其余文件缺失一律取默认值，
/// 这样手工维护的工程也能被打开。
pub fn read_spec(project: &Project) -> Result<Spec, FormatError> {
    let model: Model = project
        .read_spec_json(SPEC_MODEL_FILE)?
        .ok_or_else(|| FormatError::other("缺少 spec/model.json"))?;
    let mut spec = Spec::new(model);
    if let Some(physics) = project.read_spec_json::<PhysicsSettings>(SPEC_PHYSICS_FILE)? {
        spec.physics = physics;
    }
    if let Some(pose) = project.read_spec_json::<Pose>(SPEC_POSE_FILE)? {
        spec.pose = pose;
    }
    if let Some(settings) = project.read_spec_json(SPEC_SETTINGS_FILE)? {
        spec.settings = settings;
    }
    if let Some(config) = project.read_spec_json::<ProjectConfig>(SPEC_CONFIG_FILE)? {
        spec.config = Some(config);
    }
    spec.motions = read_list::<Motion>(project, MOTIONS_DIR, "motion.json")?;
    spec.expressions = read_list::<Expression>(project, EXPRESSIONS_DIR, "exp.json")?;
    spec.animations = read_list::<serde_json::Value>(project, ANIMATIONS_DIR, "anim.json")?;
    Ok(spec)
}

/// 把完整描述层写回工程目录。
pub fn write_spec(project: &Project, spec: &Spec) -> Result<(), FormatError> {
    project.write_spec_json(SPEC_MODEL_FILE, &spec.model)?;
    if !spec.physics.settings.is_empty() {
        project.write_spec_json(SPEC_PHYSICS_FILE, &spec.physics)?;
    }
    if !spec.pose.groups.is_empty() {
        project.write_spec_json(SPEC_POSE_FILE, &spec.pose)?;
    }
    project.write_spec_json(SPEC_SETTINGS_FILE, &spec.settings)?;
    if let Some(config) = &spec.config {
        project.write_spec_json(SPEC_CONFIG_FILE, config)?;
    }
    write_list(project, MOTIONS_DIR, "motion.json", &spec.motions)?;
    write_list(project, EXPRESSIONS_DIR, "exp.json", &spec.expressions)?;
    write_list(project, ANIMATIONS_DIR, "anim.json", &spec.animations)?;
    prune_stale_animations(project, &spec.animations)?;
    Ok(())
}

/// 删除 `spec/animations/` 下不再被引用的文件。
///
/// **为什么必须做**：只写不删的话，用户在编辑器里删掉一段动画、重开工程，
/// 它又会从磁盘上「复活」—— 因为读回时是扫目录，不是按索引。
/// 这一条有专门测试（`deleted_animation_does_not_resurrect`）。
fn prune_stale_animations(
    project: &Project,
    animations: &[serde_json::Value],
) -> Result<(), FormatError> {
    let dir = project.fs_path(&format!("{SPEC_DIR}/{ANIMATIONS_DIR}"));
    if !dir.is_dir() {
        return Ok(());
    }
    let mut keep: Vec<String> = Vec::new();
    for value in animations {
        if let Some(id) = value.get("id").and_then(|v| v.as_str()) {
            keep.push(format!("{id}.anim.json"));
        }
    }
    for entry in std::fs::read_dir(&dir)?.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        if !name.ends_with("anim.json") {
            continue;
        }
        if !keep.iter().any(|k| k == &name) {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

fn read_list<T: serde::de::DeserializeOwned>(
    project: &Project,
    sub: &str,
    suffix: &str,
) -> Result<Vec<T>, FormatError> {
    let dir = project.fs_path(&format!("{SPEC_DIR}/{sub}"));
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.to_string_lossy().ends_with(suffix))
        .collect();
    files.sort();
    let mut out = Vec::with_capacity(files.len());
    for path in files {
        let text = std::fs::read_to_string(&path)?;
        let value = serde_json::from_str(&text)
            .map_err(|e| FormatError::other(format!("{} 解析失败: {e}", path.display())))?;
        out.push(value);
    }
    Ok(out)
}

fn write_list<T: serde::Serialize>(
    project: &Project,
    sub: &str,
    suffix: &str,
    items: &[T],
) -> Result<(), FormatError> {
    for item in items {
        let value = serde_json::to_value(item)
            .map_err(|e| FormatError::other(format!("序列化失败: {e}")))?;
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| FormatError::other("描述层条目缺少 id"))?
            .to_string();
        if !am_model::is_valid_id(&id) {
            return Err(FormatError::invalid(format!("id 不能作为文件名: {id:?}")));
        }
        project.write_spec_json(&format!("{sub}/{id}.{suffix}"), &value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_format::CreateOptions;
    use am_model::{Motion, Node, Parameter};

    fn temp_project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("demo");
        let project = Project::create("demo", &root, CreateOptions::default()).unwrap();
        (dir, project)
    }

    #[test]
    fn spec_round_trips_through_a_project() {
        let (_dir, project) = temp_project();
        let mut spec = Spec::new(Model::new("demo"));
        spec.model.add_node(Node::part("Root", None));
        spec.model.add_parameter(Parameter::new("AngleX", "Angle X", -30.0, 30.0, 0.0));
        spec.motions.push(Motion::new("idle", "Idle"));
        spec.expressions.push(Expression::new("smile", "Smile"));
        spec.physics.settings.push(am_model::PhysicsSetting::new("hair", "Hair"));
        write_spec(&project, &spec).unwrap();

        let back = read_spec(&project).unwrap();
        assert_eq!(back, spec);
    }

    #[test]
    fn missing_model_file_is_an_error() {
        let (_dir, project) = temp_project();
        let err = read_spec(&project).unwrap_err();
        assert!(err.to_string().contains("model.json"), "got {err}");
    }

    #[test]
    fn empty_optional_files_are_skipped() {
        let (_dir, project) = temp_project();
        let spec = Spec::new(Model::new("demo"));
        write_spec(&project, &spec).unwrap();
        assert!(!project.fs_path(&format!("{SPEC_DIR}/{SPEC_PHYSICS_FILE}")).exists());
        assert!(!project.fs_path(&format!("{SPEC_DIR}/{SPEC_POSE_FILE}")).exists());
        let back = read_spec(&project).unwrap();
        assert_eq!(back.model.name, "demo");
        assert!(back.motions.is_empty());
    }

    #[test]
    fn invalid_motion_id_is_rejected() {
        let (_dir, project) = temp_project();
        let mut spec = Spec::new(Model::new("demo"));
        spec.motions.push(Motion::new("bad id!", "Bad"));
        assert!(write_spec(&project, &spec).is_err());
    }

    #[test]
    fn animations_round_trip_through_a_project() {
        let (_dir, project) = temp_project();
        let mut spec = Spec::new(Model::new("demo"));
        let mut animation = am_anim::AmAnimation::new("idle", "待机");
        animation.duration = 3.0;
        spec.animations.push(serde_json::to_value(&animation).unwrap());
        write_spec(&project, &spec).unwrap();

        let back = read_spec(&project).unwrap();
        assert_eq!(back.animations.len(), 1);
        let decoded: am_anim::AmAnimation =
            serde_json::from_value(back.animations[0].clone()).unwrap();
        assert_eq!(decoded, animation, "动画应无损往返");
    }

    #[test]
    fn deleted_animation_does_not_resurrect() {
        // 只写不删会导致「删掉的动画重开又回来」，这是编辑器侧已修过的同类缺陷。
        let (_dir, project) = temp_project();
        let mut spec = Spec::new(Model::new("demo"));
        spec.animations.push(serde_json::to_value(am_anim::AmAnimation::new("idle", "Idle")).unwrap());
        spec.animations.push(serde_json::to_value(am_anim::AmAnimation::new("walk", "Walk")).unwrap());
        write_spec(&project, &spec).unwrap();

        let dir = project.fs_path(&format!("{SPEC_DIR}/{ANIMATIONS_DIR}"));
        assert!(dir.join("idle.anim.json").exists());
        assert!(dir.join("walk.anim.json").exists());

        // 删掉 walk 后重写
        spec.animations.retain(|a| a.get("id").and_then(|v| v.as_str()) != Some("walk"));
        write_spec(&project, &spec).unwrap();

        let back = read_spec(&project).unwrap();
        assert_eq!(back.animations.len(), 1, "被删的动画不应复活");
        assert!(!dir.join("walk.anim.json").exists(), "磁盘上的陈旧文件应被清理");
    }

    #[test]
    fn empty_animation_dir_is_not_created() {
        let (_dir, project) = temp_project();
        let spec = Spec::new(Model::new("demo"));
        write_spec(&project, &spec).unwrap();
        let dir = project.fs_path(&format!("{SPEC_DIR}/{ANIMATIONS_DIR}"));
        assert!(!dir.exists() || std::fs::read_dir(&dir).unwrap().count() == 0);
    }
}
