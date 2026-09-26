//! 工程全量校验（对应规范第 10 章）。
//!
//! 校验函数只读文件，不修改任何内容。问题码与参考实现保持一致，
//! 便于工具链与编辑器统一展示。

use crate::error::Result;
use crate::model::{AssetMetadata, Info, Registry};
use crate::path::{asset_rel_for_meta, meta_rel_for_asset};
use crate::{json, FormatError, SDK_VERSION};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 单条校验问题。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValidationIssue {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl ValidationIssue {
    pub fn new(code: &str, message: impl Into<String>, path: Option<String>) -> Self {
        Self { code: code.to_string(), message: message.into(), path }
    }
}

impl std::fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] ", self.code)?;
        if let Some(p) = &self.path {
            write!(f, "{p} ")?;
        }
        write!(f, "{}", self.message)
    }
}

/// 校验报告。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ValidationReport {
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    pub fn ok(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn add(&mut self, code: &str, message: impl Into<String>, path: Option<String>) {
        self.issues.push(ValidationIssue::new(code, message, path));
    }

    /// 问题码清单（便于测试断言）。
    pub fn codes(&self) -> Vec<&str> {
        self.issues.iter().map(|i| i.code.as_str()).collect()
    }

    pub fn summary(&self) -> String {
        if self.ok() {
            return "OK（0 个问题）".to_string();
        }
        self.issues.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("\n")
    }

    /// 转换为 `FormatError`（校验失败时使用）。
    pub fn into_error(self) -> FormatError {
        FormatError::ValidationFailed(self.summary())
    }
}

/// 枚举某目录下的全部文件，键为相对工程根的 `/` 路径。
pub(crate) fn list_files(root: &Path, sub: &str) -> BTreeMap<String, PathBuf> {
    let mut out = BTreeMap::new();
    let base = root.join(sub);
    if !base.is_dir() {
        return out;
    }
    let mut stack = vec![base];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(root) {
                out.insert(rel.to_string_lossy().replace('\\', "/"), p);
            }
        }
    }
    out
}

fn check_text_is_utf8(path: &Path) -> bool {
    std::fs::read(path).map(|b| std::str::from_utf8(&b).is_ok()).unwrap_or(false)
}

/// 从工程目录读取 `info.json` / `registry.json` 并做全量校验。
pub fn validate_dir(root: &Path) -> Result<ValidationReport> {
    let info_path = root.join("info.json");
    if !info_path.is_file() {
        let mut rep = ValidationReport::default();
        rep.add("MISSING_INFO", "缺少 info.json", Some("info.json".into()));
        return Ok(rep);
    }
    let info: Info = match json::read(&info_path) {
        Ok(v) => v,
        Err(e) => {
            let mut rep = ValidationReport::default();
            rep.add("BAD_INFO_JSON", format!("info.json 解析失败: {e}"), Some("info.json".into()));
            return Ok(rep);
        }
    };
    let registry = match std::fs::read_to_string(root.join("registry.json")) {
        Ok(text) => json::from_str::<Registry>(&text).ok(),
        Err(_) => None,
    };
    Ok(validate_with(root, &info, registry.as_ref()))
}

/// 使用已知的 `Info` / `Registry` 做全量校验。
pub fn validate_with(
    root: &Path,
    info: &Info,
    registry: Option<&Registry>,
) -> ValidationReport {
    let mut rep = ValidationReport::default();

    // ---- 0. 基本字段
    if info.format != crate::FORMAT_ID {
        rep.add(
            "FORMAT_MISMATCH",
            format!("format 应为 '{}'，实际 {:?}", crate::FORMAT_ID, info.format),
            Some("info.json".into()),
        );
    }
    if !crate::path::is_valid_name(&info.name) {
        rep.add("NAME_INVALID", format!("name 非法: {:?}", info.name), Some("info.json".into()));
    }
    if info.last_update_time < info.created_time {
        rep.add("TIME_ORDER", "last_update_time 早于 created_time", Some("info.json".into()));
    }
    if info.min_sdk > SDK_VERSION {
        rep.add(
            "MIN_SDK_TOO_NEW",
            format!("工程要求 min_sdk={}，本 SDK 版本为 {SDK_VERSION}", info.min_sdk),
            Some("info.json".into()),
        );
    }
    if info.version < 1 {
        rep.add("VERSION_INVALID", format!("version 必须 >= 1，实际 {}", info.version), Some("info.json".into()));
    }

    // ---- 1. registry.json
    let owned_registry;
    let registry = match registry {
        Some(r) => r,
        None => {
            let reg_path = root.join("registry.json");
            if !reg_path.is_file() {
                rep.add("MISSING_REGISTRY", "缺少 registry.json", Some("registry.json".into()));
                return rep;
            }
            match json::read::<Registry>(&reg_path) {
                Ok(r) => {
                    owned_registry = r;
                    &owned_registry
                }
                Err(e) => {
                    rep.add(
                        "BAD_REGISTRY_JSON",
                        format!("registry.json 解析失败: {e}"),
                        Some("registry.json".into()),
                    );
                    return rep;
                }
            }
        }
    };

    let reg_list = &registry.registered_files;
    let unique: std::collections::BTreeSet<&String> = reg_list.iter().collect();
    if unique.len() != reg_list.len() {
        rep.add("DUP_REGISTERED", "registered_files 存在重复项", Some("registry.json".into()));
    }
    for rel in reg_list {
        if !rel.starts_with("assets/") {
            rep.add("NOT_UNDER_ASSETS", "注册路径必须位于 assets/ 下", Some(rel.clone()));
        }
    }
    if let Some(count) = registry.asset_count {
        if count != reg_list.len() as i64 {
            rep.add(
                "ASSET_COUNT_MISMATCH",
                format!("asset_count={count} 与实际 {} 不一致", reg_list.len()),
                Some("registry.json".into()),
            );
        }
    }

    // ---- 2. 枚举实际文件
    let asset_files = list_files(root, "assets");
    let meta_files = list_files(root, "metadata");

    // ---- 3. 注册资源逐项核对
    for rel in reg_list {
        let Some(asset_path) = asset_files.get(rel) else {
            rep.add("MISSING_ASSET", "注册的资源文件不存在", Some(rel.clone()));
            continue;
        };
        let meta_rel = meta_rel_for_asset(rel);
        let Some(meta_path) = meta_files.get(&meta_rel) else {
            rep.add("NO_METADATA", "缺少元数据文件", Some(rel.clone()));
            continue;
        };
        let meta = match json::read::<AssetMetadata>(meta_path) {
            Ok(m) => m,
            Err(e) => {
                rep.add("BAD_METADATA_JSON", format!("元数据 JSON 解析失败: {e}"), Some(meta_rel));
                continue;
            }
        };
        if meta.path != *rel {
            rep.add(
                "PATH_FIELD_MISMATCH",
                format!("元数据 path={:?} 与注册路径 {:?} 不一致", meta.path, rel),
                Some(meta_rel.clone()),
            );
        }
        let blob = match std::fs::read(asset_path) {
            Ok(b) => b,
            Err(e) => {
                rep.add("MISSING_ASSET", format!("资源读取失败: {e}"), Some(rel.clone()));
                continue;
            }
        };
        let real_hash = crate::sha::sha256_hex(&blob);
        if !crate::sha::is_valid_sha256(&meta.sha256) {
            rep.add("BAD_SHA256_FORMAT", "sha256 字段不是 64 位小写十六进制", Some(meta_rel.clone()));
        } else if real_hash != meta.sha256 {
            rep.add(
                "HASH_MISMATCH",
                format!("实际哈希 {}… 与元数据不符", &real_hash[..12.min(real_hash.len())]),
                Some(rel.clone()),
            );
        }
        if blob.len() as u64 != meta.size {
            rep.add(
                "SIZE_MISMATCH",
                format!("实际大小 {} 与元数据 size={} 不符", blob.len(), meta.size),
                Some(rel.clone()),
            );
        }
        if meta.is_text() && !check_text_is_utf8(asset_path) {
            rep.add("INVALID_UTF8", "文本类型资源不是合法 UTF-8", Some(rel.clone()));
        }
    }

    // ---- 4. 双向镜像
    for rel in asset_files.keys() {
        if !reg_list.contains(rel) {
            rep.add("UNREGISTERED_ASSET", "游离资源：存在但未注册", Some(rel.clone()));
        }
        let meta_rel = meta_rel_for_asset(rel);
        if !meta_files.contains_key(&meta_rel) {
            rep.add("NO_METADATA", "资源缺少对应元数据", Some(rel.clone()));
        }
    }
    for mrel in meta_files.keys() {
        match asset_rel_for_meta(mrel) {
            Some(asset_rel) if asset_files.contains_key(&asset_rel) => {}
            _ => rep.add("ORPHAN_METADATA", "孤儿元数据：无对应资源", Some(mrel.clone())),
        }
    }

    // ---- 9. 描述层（spec/ 存在时解析全部 JSON）
    let spec_files = list_files(root, "spec");
    for (rel, path) in &spec_files {
        if crate::path::ext_of(rel) == "json" {
            if let Err(e) = json::read::<serde_json::Value>(path) {
                rep.add("SPEC_JSON_INVALID", format!("描述层 JSON 解析失败: {e}"), Some(rel.clone()));
            }
        }
    }

    rep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// 造一个最小合法工程目录。
    fn make_project(root: &Path) -> (Info, Registry) {
        let asset = b"hello";
        let hash = crate::sha::sha256_hex(asset);
        write(&root.join("assets/images/a.png"), "hello");
        write(
            &root.join("metadata/images/a.png.json"),
            &format!(
                "{{\"path\":\"assets/images/a.png\",\"type\":\"image\",\"size\":5,\"sha256\":\"{hash}\"}}"
            ),
        );
        write(&root.join("spec/model.json"), "{\"nodes\":[]}");
        let mut info = Info::new("demo");
        info.created_time = 1;
        info.last_update_time = 2;
        write(&root.join("info.json"), &crate::json::to_string_pretty_tab(&info).unwrap());
        let mut reg = Registry { registered_files: vec!["assets/images/a.png".into()], ..Default::default() };
        reg.normalize();
        write(&root.join("registry.json"), &crate::json::to_string_pretty_tab(&reg).unwrap());
        (info, reg)
    }

    #[test]
    fn valid_project_passes() {
        let dir = tempfile::tempdir().unwrap();
        let _ = make_project(dir.path());
        let rep = validate_dir(dir.path()).unwrap();
        assert!(rep.ok(), "{}", rep.summary());
    }

    #[test]
    fn hash_mismatch_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let _ = make_project(dir.path());
        write(&dir.path().join("assets/images/a.png"), "HELLO!");
        let rep = validate_dir(dir.path()).unwrap();
        assert!(rep.codes().contains(&"HASH_MISMATCH"), "{}", rep.summary());
        assert!(rep.codes().contains(&"SIZE_MISMATCH"));
    }

    #[test]
    fn unregistered_asset_and_orphan_metadata_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let _ = make_project(dir.path());
        write(&dir.path().join("assets/images/extra.png"), "x");
        write(&dir.path().join("metadata/images/gone.png.json"), "{\"path\":\"assets/images/gone.png\"}");
        let rep = validate_dir(dir.path()).unwrap();
        let codes = rep.codes();
        assert!(codes.contains(&"UNREGISTERED_ASSET"));
        assert!(codes.contains(&"ORPHAN_METADATA"));
        assert!(codes.contains(&"NO_METADATA"));
    }

    #[test]
    fn format_and_name_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        let (mut info, reg) = make_project(dir.path());
        info.format = "lgdf".into();
        info.name = "Bad Name".into();
        info.min_sdk = SDK_VERSION + 1;
        let rep = validate_with(dir.path(), &info, Some(&reg));
        let codes = rep.codes();
        assert!(codes.contains(&"FORMAT_MISMATCH"));
        assert!(codes.contains(&"NAME_INVALID"));
        assert!(codes.contains(&"MIN_SDK_TOO_NEW"));
    }

    #[test]
    fn missing_registry_short_circuits() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("info.json"), "{\"name\":\"demo\"}");
        let rep = validate_dir(dir.path()).unwrap();
        assert!(rep.codes().contains(&"MISSING_REGISTRY"));
    }

    #[test]
    fn bad_spec_json_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let _ = make_project(dir.path());
        write(&dir.path().join("spec/broken.json"), "{ not json");
        let rep = validate_dir(dir.path()).unwrap();
        assert!(rep.codes().contains(&"SPEC_JSON_INVALID"));
    }

    #[test]
    fn text_asset_must_be_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let raw = [0xffu8, 0xfe, 0x00];
        std::fs::create_dir_all(dir.path().join("assets/spec")).unwrap();
        std::fs::write(dir.path().join("assets/spec/a.json"), raw).unwrap();
        let hash = crate::sha::sha256_hex(&raw);
        write(
            &dir.path().join("metadata/spec/a.json.json"),
            &format!(
                "{{\"path\":\"assets/spec/a.json\",\"type\":\"text\",\"size\":3,\"sha256\":\"{hash}\"}}"
            ),
        );
        let mut info = Info::new("demo");
        info.created_time = 1;
        info.last_update_time = 1;
        let mut reg = Registry { registered_files: vec!["assets/spec/a.json".into()], ..Default::default() };
        reg.normalize();
        let rep = validate_with(dir.path(), &info, Some(&reg));
        assert!(rep.codes().contains(&"INVALID_UTF8"), "{}", rep.summary());
    }
}
