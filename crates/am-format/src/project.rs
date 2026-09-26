//! 工程对象：创建 / 读取 / 修改 / 导出 / 导入。
//!
//! 工程**始终以目录模式存在**；`.amproj` 是目录与压缩包之间的一种形态转换。

use crate::error::{FormatError, Result};
use crate::model::{AssetMetadata, ExtraMap, Info, Registry};
use crate::path::{asset_rel, meta_rel_for_asset, normalize_rel, to_fs_path};
use crate::validate::{list_files, ValidationReport};
use crate::{json, DIST_DIR, SPEC_DIR};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// 当前 Unix 时间戳（秒）。
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 创建工程时的可选参数。
#[derive(Debug, Clone, Default)]
pub struct CreateOptions {
    pub display_name: Option<String>,
    pub description: String,
    pub author: Option<String>,
    pub license: Option<String>,
    pub tags: Vec<String>,
    /// 覆盖 `min_sdk`（默认取当前 SDK 版本）。
    pub min_sdk: Option<i64>,
}

/// 导出结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportResult {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub entries: usize,
}

/// 一个 `.amproj` 工程。
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
    info: Info,
    registry: Registry,
    /// 由压缩包解包出来的临时工程，`drop` 时删除。
    ephemeral: bool,
}

impl Drop for Project {
    fn drop(&mut self) {
        if self.ephemeral {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

impl Project {
    // ------------------------------------------------------------ 生命周期

    /// 新建工程。目标目录必须不存在或为空。
    pub fn create(name: &str, path: impl AsRef<Path>, opts: CreateOptions) -> Result<Project> {
        if !crate::path::is_valid_name(name) {
            return Err(FormatError::invalid(format!(
                "name 非法: {name:?}（应满足 {}）",
                crate::NAME_PATTERN_HINT
            )));
        }
        let root = path.as_ref().to_path_buf();
        if root.exists() {
            let mut it = std::fs::read_dir(&root)?;
            if it.next().is_some() {
                return Err(FormatError::other(format!("目标目录非空: {}", root.display())));
            }
        }
        let now = now_secs();
        let mut info = Info::new(name);
        info.display_name = opts.display_name;
        info.description = opts.description;
        info.author = opts.author;
        info.license = opts.license;
        info.tags = opts.tags;
        info.created_time = now;
        info.last_update_time = now;
        if let Some(min_sdk) = opts.min_sdk {
            info.min_sdk = min_sdk;
        }
        let mut project = Project {
            root,
            info,
            registry: Registry::default(),
            ephemeral: false,
        };
        for dir in ["assets", "metadata", SPEC_DIR] {
            std::fs::create_dir_all(project.root.join(dir))?;
        }
        project.registry.normalize();
        project.write_meta_files()?;
        Ok(project)
    }

    /// 打开目录模式工程。
    pub fn open(source: impl AsRef<Path>) -> Result<Project> {
        let root = source.as_ref().to_path_buf();
        if !root.is_dir() {
            return Err(FormatError::invalid(format!("不是目录: {}", root.display())));
        }
        let info_path = root.join("info.json");
        if !info_path.is_file() {
            return Err(FormatError::invalid(format!("缺少 info.json: {}", root.display())));
        }
        let info: Info = json::read(&info_path).map_err(|e| {
            FormatError::invalid(format!("info.json 解析失败: {e}"))
        })?;
        Self::check_info(&info)?;
        let registry_path = root.join("registry.json");
        if !registry_path.is_file() {
            return Err(FormatError::invalid(format!("缺少 registry.json: {}", root.display())));
        }
        let registry: Registry = json::read(&registry_path).map_err(|e| {
            FormatError::invalid(format!("registry.json 解析失败: {e}"))
        })?;
        Ok(Project { root, info, registry, ephemeral: false })
    }

    /// 打开目录或 `.amproj` 压缩包。压缩包会被解包到临时目录，`Project` 析构时自动清理。
    pub fn open_any(source: impl AsRef<Path>) -> Result<Project> {
        let source = source.as_ref();
        if source.is_dir() {
            return Project::open(source);
        }
        if !source.is_file() {
            return Err(FormatError::invalid(format!("路径不存在: {}", source.display())));
        }
        let tmp = std::env::temp_dir().join(format!(
            "anima-open-{}-{}",
            std::process::id(),
            now_secs()
        ));
        let mut project = Project::import_from(source, &tmp)?;
        project.ephemeral = true;
        Ok(project)
    }

    fn check_info(info: &Info) -> Result<()> {
        if info.format != crate::FORMAT_ID {
            return Err(FormatError::invalid(format!(
                "format 应为 {:?}，实际 {:?}",
                crate::FORMAT_ID,
                info.format
            )));
        }
        if info.min_sdk > crate::SDK_VERSION {
            return Err(FormatError::invalid(format!(
                "工程要求 min_sdk={}，本 SDK 版本为 {}",
                info.min_sdk,
                crate::SDK_VERSION
            )));
        }
        Ok(())
    }

    // ------------------------------------------------------------ 访问器

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    pub fn info_mut(&mut self) -> &mut Info {
        &mut self.info
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn is_ephemeral(&self) -> bool {
        self.ephemeral
    }

    pub fn fs_path(&self, rel: &str) -> PathBuf {
        to_fs_path(&self.root, rel)
    }

    // ------------------------------------------------------------ 保存

    /// 落盘 `info.json` + `registry.json`，并刷新 `last_update_time`。
    pub fn save(&mut self) -> Result<()> {
        self.info.last_update_time = now_secs();
        self.registry.normalize();
        self.write_meta_files()
    }

    fn write_meta_files(&self) -> Result<()> {
        json::write(&self.root.join("info.json"), &self.info)?;
        json::write(&self.root.join("registry.json"), &self.registry)?;
        Ok(())
    }

    /// 递增工程版本号。
    pub fn bump_version(&mut self, steps: i64) -> Result<()> {
        if steps < 1 {
            return Err(FormatError::other("steps 必须 >= 1"));
        }
        self.info.version += steps;
        self.save()
    }

    // ------------------------------------------------------------ 资源

    pub fn is_registered(&self, rel: &str) -> bool {
        asset_rel(rel)
            .map(|r| self.registry.registered_files.iter().any(|x| x == &r))
            .unwrap_or(false)
    }

    pub fn list_assets(&self) -> Result<Vec<AssetMetadata>> {
        self.registry.registered_files.iter().map(|rel| self.get_metadata(rel)).collect()
    }

    /// 读取资源字节。
    pub fn read_asset(&self, rel: &str) -> Result<Vec<u8>> {
        let rel = asset_rel(rel)?;
        let p = self.fs_path(&rel);
        if !p.is_file() {
            return Err(FormatError::AssetNotFound(rel));
        }
        Ok(std::fs::read(p)?)
    }

    /// 读取资源元数据。
    pub fn get_metadata(&self, rel: &str) -> Result<AssetMetadata> {
        let rel = asset_rel(rel)?;
        let p = self.fs_path(&meta_rel_for_asset(&rel));
        if !p.is_file() {
            return Err(FormatError::AssetNotFound(format!("元数据不存在: {rel}")));
        }
        json::read(&p)
    }

    /// 元数据是否存在（不报错版本）。
    pub fn try_get_metadata(&self, rel: &str) -> Option<AssetMetadata> {
        self.get_metadata(rel).ok()
    }

    /// 加入资源（内容由调用方给出）。重复注册会报错。
    pub fn add_asset(
        &mut self,
        rel: &str,
        bytes: &[u8],
        properties: ExtraMap,
    ) -> Result<AssetMetadata> {
        let rel = asset_rel(rel)?;
        if self.registry.registered_files.iter().any(|x| x == &rel) {
            return Err(FormatError::DuplicateAsset(rel));
        }
        let dest = self.fs_path(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, bytes)?;
        let mut meta = AssetMetadata::from_bytes(&rel, bytes, now_secs());
        for (k, v) in properties {
            meta.properties.insert(k, v);
        }
        self.write_asset_metadata(&meta)?;
        self.registry.registered_files.push(rel);
        self.save()?;
        Ok(meta)
    }

    /// 从磁盘文件加入资源。
    pub fn add_asset_file(&mut self, rel: &str, source: impl AsRef<Path>) -> Result<AssetMetadata> {
        let bytes = std::fs::read(source)?;
        self.add_asset(rel, &bytes, ExtraMap::new())
    }

    /// 替换资源内容并刷新哈希/大小。
    pub fn replace_asset(
        &mut self,
        rel: &str,
        bytes: &[u8],
        properties: Option<ExtraMap>,
    ) -> Result<AssetMetadata> {
        let rel = asset_rel(rel)?;
        if !self.registry.registered_files.iter().any(|x| x == &rel) {
            return Err(FormatError::AssetNotFound(rel));
        }
        let dest = self.fs_path(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, bytes)?;
        let mut meta = self.get_metadata(&rel)?;
        meta.size = bytes.len() as u64;
        meta.sha256 = crate::sha::sha256_hex(bytes);
        meta.last_update_time = now_secs();
        if let Some(props) = properties {
            for (k, v) in props {
                meta.properties.insert(k, v);
            }
        }
        self.write_asset_metadata(&meta)?;
        self.save()?;
        Ok(meta)
    }

    /// 移除资源、元数据与随之空掉的目录。
    pub fn remove_asset(&mut self, rel: &str) -> Result<()> {
        let rel = asset_rel(rel)?;
        if !self.registry.registered_files.iter().any(|x| x == &rel) {
            return Err(FormatError::AssetNotFound(rel));
        }
        self.registry.registered_files.retain(|x| x != &rel);
        let asset_path = self.fs_path(&rel);
        let meta_path = self.fs_path(&meta_rel_for_asset(&rel));
        let _ = std::fs::remove_file(&asset_path);
        let _ = std::fs::remove_file(&meta_path);
        prune_empty_dirs(&asset_path, &self.root.join("assets"));
        prune_empty_dirs(&meta_path, &self.root.join(crate::METADATA_DIR));
        self.save()
    }

    fn write_asset_metadata(&self, meta: &AssetMetadata) -> Result<()> {
        let rel = meta_rel_for_asset(&meta.path);
        json::write(&self.fs_path(&rel), meta)
    }

    /// 重新为全部已注册资源计算元数据（用于外部改动后修复工程）。
    pub fn rehash_all(&mut self) -> Result<usize> {
        let mut n = 0;
        let files: Vec<String> = self.registry.registered_files.clone();
        for rel in files {
            let path = self.fs_path(&rel);
            if !path.is_file() {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            let mut meta = self.get_metadata(&rel).unwrap_or_else(|_| {
                AssetMetadata::from_bytes(&rel, &bytes, now_secs())
            });
            meta.size = bytes.len() as u64;
            meta.sha256 = crate::sha::sha256_hex(&bytes);
            meta.last_update_time = now_secs();
            self.write_asset_metadata(&meta)?;
            n += 1;
        }
        self.save()?;
        Ok(n)
    }

    // ------------------------------------------------------------ 描述层

    /// 写 `spec/` 下的文本文件。
    pub fn write_spec_file(&self, rel: &str, text: &str) -> Result<()> {
        let rel = normalize_rel(rel)?;
        if rel.is_empty() {
            return Err(FormatError::InvalidPath("空描述层路径".into()));
        }
        let path = self.fs_path(&format!("{SPEC_DIR}/{rel}"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, text)?;
        Ok(())
    }

    /// 读 `spec/` 下的文本文件。
    pub fn read_spec_file(&self, rel: &str) -> Result<String> {
        let rel = normalize_rel(rel)?;
        Ok(std::fs::read_to_string(self.fs_path(&format!("{SPEC_DIR}/{rel}")))?)
    }

    /// 写 `spec/` 下的 JSON。
    pub fn write_spec_json<T: Serialize>(&self, rel: &str, value: &T) -> Result<()> {
        let rel = normalize_rel(rel)?;
        json::write(&self.fs_path(&format!("{SPEC_DIR}/{rel}")), value)
    }

    /// 读 `spec/` 下的 JSON；文件不存在返回 `None`。
    pub fn read_spec_json<T: DeserializeOwned>(&self, rel: &str) -> Result<Option<T>> {
        let rel = normalize_rel(rel)?;
        let path = self.fs_path(&format!("{SPEC_DIR}/{rel}"));
        if !path.is_file() {
            return Ok(None);
        }
        Ok(Some(json::read(&path)?))
    }

    pub fn set_overview(&self, markdown: &str) -> Result<()> {
        self.write_spec_file("overview.md", markdown)
    }

    pub fn get_config(&self) -> Result<Option<serde_json::Value>> {
        self.read_spec_json("config.json")
    }

    pub fn set_config(&self, cfg: &serde_json::Value) -> Result<()> {
        self.write_spec_json("config.json", cfg)
    }

    pub fn get_steps(&self) -> Result<Option<serde_json::Value>> {
        self.read_spec_json("steps.json")
    }

    pub fn set_steps(&self, steps: &serde_json::Value) -> Result<()> {
        self.write_spec_json("steps.json", steps)
    }

    // ------------------------------------------------------------ 校验 / 签名

    pub fn validate(&self) -> ValidationReport {
        crate::validate::validate_with(&self.root, &self.info, Some(&self.registry))
    }

    /// 校验不通过时返回错误。
    pub fn ensure_valid(&self) -> Result<()> {
        let rep = self.validate();
        if rep.ok() {
            Ok(())
        } else {
            Err(rep.into_error())
        }
    }

    // ------------------------------------------------------------ 导出 / 导入

    /// 收集需要导出的条目（相对路径，已排序）。
    pub fn export_entries(&self) -> Vec<String> {
        let mut entries = vec!["info.json".to_string(), "registry.json".to_string()];
        for sub in ["assets", "metadata", SPEC_DIR] {
            let files = list_files(&self.root, sub);
            entries.extend(files.into_keys());
        }
        entries.sort();
        entries.dedup();
        entries
    }

    /// 导出为 `.amproj` 压缩包。默认输出到 `<工程>/dist/<name>.amproj`。
    pub fn export(&self, out_path: Option<&Path>, skip_validation: bool) -> Result<ExportResult> {
        if !skip_validation {
            self.ensure_valid()?;
        }
        let out = match out_path {
            Some(p) => p.to_path_buf(),
            None => self.root.join(DIST_DIR).join(format!("{}.{}", self.info.name, crate::PACKAGE_EXT)),
        };
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let entries = self.export_entries();
        {
            let file = std::fs::File::create(&out)?;
            let mut zw = ZipWriter::new(file);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            for rel in &entries {
                let src = self.fs_path(rel);
                if !src.is_file() {
                    continue;
                }
                zw.start_file(rel.as_str(), opts)?;
                let mut f = std::fs::File::open(&src)?;
                std::io::copy(&mut f, &mut zw)?;
            }
            zw.finish()?;
        }
        let digest = crate::sha::sha256_hex_file(&out)?;
        let sidecar = PathBuf::from(format!("{}.sha256", out.display()));
        std::fs::write(&sidecar, format!("{digest}\n"))?;
        let bytes = std::fs::metadata(&out)?.len();
        Ok(ExportResult {
            path: out.to_string_lossy().to_string(),
            sha256: digest,
            bytes,
            entries: entries.len(),
        })
    }

    /// 从压缩包导入为目录模式工程。目标目录必须为空或不存在；校验失败会回滚。
    pub fn import_from(source: impl AsRef<Path>, dest: impl AsRef<Path>) -> Result<Project> {
        let source = source.as_ref().to_path_buf();
        let dest = dest.as_ref().to_path_buf();
        if dest.exists() {
            let mut it = std::fs::read_dir(&dest)?;
            if it.next().is_some() {
                return Err(FormatError::other(format!("目标目录非空: {}", dest.display())));
            }
        }
        std::fs::create_dir_all(&dest)?;

        let result = (|| -> Result<Project> {
            let file = std::fs::File::open(&source)?;
            let mut za = ZipArchive::new(file)?;
            for i in 0..za.len() {
                let mut entry = za.by_index(i)?;
                let raw_name = entry.name().to_string();
                if raw_name.ends_with('/') {
                    continue;
                }
                // zip-slip 防护：规范化后必须落在白名单内。
                let rel = match normalize_rel(&raw_name) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                if rel.is_empty() {
                    continue;
                }
                let allowed = crate::IMPORTABLE_ROOT_FILES.contains(&rel.as_str())
                    || crate::EXPORT_ENTRY_PREFIXES.iter().any(|p| rel.starts_with(p));
                if !allowed {
                    continue;
                }
                let target = to_fs_path(&dest, &rel);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut out = std::fs::File::create(&target)?;
                std::io::copy(&mut entry, &mut out)?;
            }
            let project = Project::open(&dest)?;
            let rep = project.validate();
            if !rep.ok() {
                return Err(rep.into_error());
            }
            Ok(project)
        })();

        match result {
            Ok(p) => Ok(p),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dest);
                Err(e)
            }
        }
    }

    /// 是否为 ZIP 压缩包（魔数 `PK\x03\x04`）。
    pub fn is_package(source: impl AsRef<Path>) -> bool {
        use std::io::Read;
        let mut buf = [0u8; 4];
        match std::fs::File::open(source).and_then(|mut f| f.read_exact(&mut buf)) {
            Ok(()) => buf == [0x50, 0x4B, 0x03, 0x04],
            Err(_) => false,
        }
    }
}

fn prune_empty_dirs(removed: &Path, stop: &Path) {
    let mut dir = removed.parent().map(Path::to_path_buf);
    while let Some(d) = dir {
        if d == stop || !d.starts_with(stop) {
            break;
        }
        let is_empty = match std::fs::read_dir(&d) {
            Ok(mut it) => it.next().is_none(),
            Err(_) => false,
        };
        if !is_empty || std::fs::remove_dir(&d).is_err() {
            break;
        }
        dir = d.parent().map(Path::to_path_buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_demo(dir: &Path) -> Project {
        Project::create(
            "demo",
            dir,
            CreateOptions {
                display_name: Some("示例".into()),
                author: Some("a human".into()),
                tags: vec!["test".into()],
                ..Default::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn create_writes_expected_layout() {
        let dir = tempfile::tempdir().unwrap();
        let p = create_demo(&dir.path().join("demo"));
        for f in ["info.json", "registry.json", "assets", "metadata", "spec"] {
            assert!(p.root().join(f).exists(), "缺少 {f}");
        }
        let info = p.info();
        assert_eq!(info.format, crate::FORMAT_ID);
        assert_eq!(info.name, "demo");
        assert_eq!(info.display(), "示例");
        assert!(p.validate().ok());
    }

    #[test]
    fn create_rejects_invalid_name_and_non_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Project::create("Bad Name", dir.path().join("x"), Default::default()).is_err());
        let target = dir.path().join("busy");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("f"), "x").unwrap();
        assert!(Project::create("ok", &target, Default::default()).is_err());
    }

    #[test]
    fn add_read_replace_remove_asset_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = create_demo(&dir.path().join("demo"));

        let meta = p.add_asset("images/a.png", b"AAA", ExtraMap::new()).unwrap();
        assert_eq!(meta.path, "assets/images/a.png");
        assert_eq!(meta.kind, "image");
        assert_eq!(meta.width(), None);
        assert!(p.is_registered("assets/images/a.png"));
        assert!(p.validate().ok(), "{}", p.validate().summary());

        // 重复注册
        assert!(p.add_asset("assets/images/a.png", b"AAA", ExtraMap::new()).is_err());

        assert_eq!(p.read_asset("images/a.png").unwrap(), b"AAA");

        // 替换后哈希与大小同步
        let meta = p.replace_asset("images/a.png", b"BBBBB", None).unwrap();
        assert_eq!(meta.size, 5);
        assert_eq!(meta.sha256, crate::sha::sha256_hex(b"BBBBB"));
        assert!(p.validate().ok(), "{}", p.validate().summary());

        p.remove_asset("images/a.png").unwrap();
        assert!(!p.is_registered("assets/images/a.png"));
        assert!(!p.root().join("assets/images").exists(), "空目录应被清理");
        assert!(p.validate().ok());
    }

    #[test]
    fn metadata_carries_classification_properties() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = create_demo(&dir.path().join("demo"));
        let mut props = ExtraMap::new();
        props.insert("width".into(), json!(2160));
        props.insert("height".into(), json!(2160));
        let meta = p.add_asset("images/big.png", b"x", props).unwrap();
        assert_eq!(meta.width(), Some(2160));
        let reread = p.get_metadata("images/big.png").unwrap();
        assert_eq!(reread.height(), Some(2160));
        assert!(p.validate().ok());
    }

    #[test]
    fn spec_layer_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let p = create_demo(&dir.path().join("demo"));
        p.set_overview("# 概述").unwrap();
        p.set_config(&json!({"project": {"name": "demo"}})).unwrap();
        p.write_spec_json("model.json", &json!({"nodes": [{"id": "n1"}]})).unwrap();
        assert_eq!(p.read_spec_file("overview.md").unwrap(), "# 概述");
        assert_eq!(p.get_config().unwrap().unwrap()["project"]["name"], "demo");
        let model: Option<serde_json::Value> = p.read_spec_json("model.json").unwrap();
        assert!(model.is_some());
        let missing: Option<serde_json::Value> = p.read_spec_json("nope.json").unwrap();
        assert!(missing.is_none());
        assert!(p.validate().ok());
    }

    #[test]
    fn export_and_import_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = create_demo(&dir.path().join("demo"));
        p.add_asset("images/a.png", b"AAA", ExtraMap::new()).unwrap();
        p.write_spec_json("model.json", &json!({"nodes": []})).unwrap();

        let out = dir.path().join("dist/demo.amproj");
        let res = p.export(Some(&out), false).unwrap();
        assert!(out.is_file());
        assert_eq!(res.sha256, crate::sha::sha256_hex_file(&out).unwrap());
        let sidecar = PathBuf::from(format!("{}.sha256", out.display()));
        assert_eq!(std::fs::read_to_string(sidecar).unwrap().trim(), res.sha256);
        assert!(Project::is_package(&out));

        let restored = Project::import_from(&out, dir.path().join("restored")).unwrap();
        assert_eq!(restored.info().name, "demo");
        assert_eq!(restored.read_asset("images/a.png").unwrap(), b"AAA");
        assert!(restored.validate().ok());
    }

    #[test]
    fn export_refuses_invalid_project() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = create_demo(&dir.path().join("demo"));
        p.add_asset("images/a.png", b"AAA", ExtraMap::new()).unwrap();
        // 破坏资源内容，制造哈希不匹配
        std::fs::write(p.fs_path("assets/images/a.png"), b"BBB").unwrap();
        assert!(p.export(None, false).is_err());
        // 跳过校验可以导出
        assert!(p.export(None, true).is_ok());
    }

    #[test]
    fn import_rejects_zip_slip_entries() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let evil = dir.path().join("evil.amproj");
        {
            let f = std::fs::File::create(&evil).unwrap();
            let mut zw = ZipWriter::new(f);
            let opts = SimpleFileOptions::default();
            zw.start_file("../escaped.txt", opts).unwrap();
            zw.write_all(b"pwn").unwrap();
            zw.start_file("info.json", opts).unwrap();
            zw.write_all(br#"{"format":"amproj","min_sdk":1,"name":"demo","created_time":1,"last_update_time":1,"version":1}"#).unwrap();
            zw.start_file("registry.json", opts).unwrap();
            zw.write_all(br#"{"registered_files":[],"asset_count":0}"#).unwrap();
            zw.finish().unwrap();
        }
        let dest = dir.path().join("out");
        // 非法条目被忽略；工程本身合法 → 导入成功，且没有任何文件逃出目标目录
        let p = Project::import_from(&evil, &dest).unwrap();
        assert_eq!(p.info().name, "demo");
        assert!(!dir.path().join("escaped.txt").exists());
        assert!(!dest.parent().unwrap().join("escaped.txt").exists());
    }

    #[test]
    fn import_rolls_back_on_invalid_project() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.amproj");
        {
            let f = std::fs::File::create(&bad).unwrap();
            let mut zw = ZipWriter::new(f);
            let opts = SimpleFileOptions::default();
            zw.start_file("info.json", opts).unwrap();
            zw.write_all(br#"{"format":"amproj","min_sdk":1,"name":"bad"}"#).unwrap();
            zw.start_file("registry.json", opts).unwrap();
            zw.write_all(br#"{"registered_files":["assets/missing.png"]}"#).unwrap();
            zw.finish().unwrap();
        }
        let dest = dir.path().join("out");
        assert!(Project::import_from(&bad, &dest).is_err());
        assert!(!dest.exists(), "导入失败应回滚目标目录");
    }

    #[test]
    fn open_any_handles_package_via_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = create_demo(&dir.path().join("demo"));
        p.add_asset("images/a.png", b"AAA", ExtraMap::new()).unwrap();
        let out = p.export(Some(&dir.path().join("dist/demo.amproj")), false).unwrap();
        let opened = Project::open_any(&out.path).unwrap();
        assert!(opened.is_ephemeral());
        assert_eq!(opened.read_asset("images/a.png").unwrap(), b"AAA");
        let temp_root = opened.root().to_path_buf();
        drop(opened);
        assert!(!temp_root.exists(), "临时工程应被清理");
    }

    #[test]
    fn open_rejects_wrong_format() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("demo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("info.json"),
            r#"{"format":"lgdf","min_sdk":1,"name":"demo"}"#,
        )
        .unwrap();
        std::fs::write(root.join("registry.json"), r#"{"registered_files":[]}"#).unwrap();
        let err = Project::open(&root).unwrap_err();
        assert!(format!("{err}").contains("amproj"));
    }

    #[test]
    fn rehash_all_repairs_metadata_after_external_edit() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = create_demo(&dir.path().join("demo"));
        p.add_asset("images/a.png", b"AAA", ExtraMap::new()).unwrap();
        std::fs::write(p.fs_path("assets/images/a.png"), b"ZZZ").unwrap();
        assert!(!p.validate().ok());
        p.rehash_all().unwrap();
        assert!(p.validate().ok(), "{}", p.validate().summary());
    }
}
