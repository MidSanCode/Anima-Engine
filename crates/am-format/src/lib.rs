//! `am-format`：`.amproj` 工程格式的读写、校验与打包。
//!
//! 本 crate 是格式规范的**唯一权威实现**，派生自 LGDF v2.0 标准（见仓库 `docs/format.md`）。
//! 所有读写路径都必须经过这里，编辑器与查看器不得自行解析工程文件。

pub mod error;
pub mod json;
pub mod mime;
pub mod model;
pub mod path;
pub mod project;
pub mod sha;
pub mod validate;

pub use error::{FormatError, Result};
pub use model::{AssetMetadata, AuthorSign, Info, Registry, TimestampSign};
pub use project::{CreateOptions, ExportResult, Project};
pub use validate::{ValidationIssue, ValidationReport};

/// 格式标识，写入 `info.json` 的 `format` 字段。
pub const FORMAT_ID: &str = "amproj";

/// 当前 SDK 版本。
pub const SDK_VERSION: i64 = 1;

/// 导出包默认扩展名。
pub const PACKAGE_EXT: &str = "amproj";

/// 工程名合法字符集提示。
pub const NAME_PATTERN_HINT: &str = "^[a-z0-9_-]+$";

/// 导出包中允许出现在根目录的文件。
pub const IMPORTABLE_ROOT_FILES: [&str; 2] = ["info.json", "registry.json"];

/// 导出包中允许的顶层目录前缀。
pub const EXPORT_ENTRY_PREFIXES: [&str; 3] = ["assets/", "metadata/", "spec/"];

pub const METADATA_DIR: &str = "metadata";
pub const ASSETS_DIR: &str = "assets";
pub const SPEC_DIR: &str = "spec";
pub const WORK_DIR: &str = "work";
pub const DIST_DIR: &str = "dist";
