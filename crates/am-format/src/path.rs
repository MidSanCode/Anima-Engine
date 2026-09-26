//! 工程名与相对路径的规范化。
//!
//! 规范要求：内部路径一律 `/` 分隔、相对、无前导斜杠、不含 `..`。

use crate::error::{FormatError, Result};
use std::path::PathBuf;

/// 校验工程名是否符合 `^[a-z0-9_-]+$`。
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// 规范化相对路径。反斜杠会被转换为 `/`，前导 `/` 会被去掉；含 `..` 时报错。
pub fn normalize_rel(rel: &str) -> Result<String> {
    let unified = rel.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for part in unified.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err(FormatError::InvalidPath(format!("路径包含 '..': {rel:?}"))),
            p => parts.push(p),
        }
    }
    Ok(parts.join("/"))
}

/// 把相对路径转换为文件系统路径。
pub fn to_fs_path(root: &std::path::Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for seg in rel.split('/') {
        p.push(seg);
    }
    p
}

/// 把资产相对路径补全为 `assets/` 前缀。
pub fn asset_rel(rel: &str) -> Result<String> {
    let rel = normalize_rel(rel)?;
    if rel.starts_with("assets/") {
        Ok(rel)
    } else if rel.is_empty() {
        Err(FormatError::InvalidPath("空资源路径".into()))
    } else {
        Ok(format!("assets/{rel}"))
    }
}

/// `assets/x/y.png` → `metadata/x/y.png.json`
pub fn meta_rel_for_asset(asset: &str) -> String {
    let tail = asset.strip_prefix("assets/").unwrap_or(asset);
    format!("metadata/{tail}.json")
}

/// `metadata/x/y.png.json` → `assets/x/y.png`
pub fn asset_rel_for_meta(meta: &str) -> Option<String> {
    let tail = meta.strip_prefix("metadata/")?;
    let tail = tail.strip_suffix(".json")?;
    Some(format!("assets/{tail}"))
}

/// 从路径取小写扩展名（不含点）。
pub fn ext_of(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    match name.rsplit_once('.') {
        Some((_, ext)) => ext.to_ascii_lowercase(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_unifies_separators_and_strips_prefix() {
        assert_eq!(normalize_rel("/assets\\images/a.png").unwrap(), "assets/images/a.png");
        assert_eq!(normalize_rel("./a//b/").unwrap(), "a/b");
    }

    #[test]
    fn normalize_rejects_parent_traversal() {
        assert!(normalize_rel("../etc/passwd").is_err());
        assert!(normalize_rel("assets/../../x").is_err());
    }

    #[test]
    fn asset_prefix_is_added_once() {
        assert_eq!(asset_rel("images/a.png").unwrap(), "assets/images/a.png");
        assert_eq!(asset_rel("assets/images/a.png").unwrap(), "assets/images/a.png");
    }

    #[test]
    fn metadata_mirror_round_trips() {
        let m = meta_rel_for_asset("assets/images/a.png");
        assert_eq!(m, "metadata/images/a.png.json");
        assert_eq!(asset_rel_for_meta(&m).unwrap(), "assets/images/a.png");
    }

    #[test]
    fn name_pattern() {
        assert!(is_valid_name("my_model-01"));
        assert!(!is_valid_name("My Model"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("模型"));
    }

    #[test]
    fn extension_lookup_is_case_insensitive() {
        assert_eq!(ext_of("assets/images/A.PNG"), "png");
        assert_eq!(ext_of("assets/noext"), "");
    }
}
