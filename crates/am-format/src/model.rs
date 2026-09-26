//! 工程数据模型：`info.json` / `registry.json` / `metadata/*.json`。
//!
//! 未知字段一律收纳进 `extra`（Info/Registry）或 `properties`（AssetMetadata），
//! 这样读取旧版本工程或第三方扩展字段时不会丢数据。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 扩展字段容器。
pub type ExtraMap = Map<String, Value>;

/// 作者签名（RSA / ECC）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthorSign {
    /// `RSA-SHA256`、`RSA-SHA512`、`ECDSA-P256-SHA256`、`ECDSA-P384-SHA384`。
    pub algorithm: String,
    /// PEM 格式公钥。
    pub public_key: String,
    /// Base64 签名值。
    pub signature: String,
    /// 签名时间（Unix 秒）。
    #[serde(default)]
    pub signed_at: i64,
}

/// 公共 TSA 时间戳签名（RFC 3161）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimestampSign {
    pub tsa_url: String,
    /// Base64 编码的时间戳令牌。
    pub token: String,
    #[serde(default)]
    pub signed_at: i64,
}

fn default_format() -> String {
    crate::FORMAT_ID.to_string()
}

fn default_min_sdk() -> i64 {
    crate::SDK_VERSION
}

fn default_version() -> i64 {
    1
}

fn default_type() -> String {
    "other".to_string()
}

fn default_hash_algorithm() -> String {
    "sha256".to_string()
}

/// `info.json`：工程身份证。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Info {
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default = "default_min_sdk")]
    pub min_sdk: i64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_sign: Option<AuthorSign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_time: i64,
    #[serde(default)]
    pub last_update_time: i64,
    #[serde(default = "default_version")]
    pub version: i64,
    #[serde(flatten, default)]
    pub extra: ExtraMap,
}

impl Info {
    /// 新建一个未落盘的 `Info`（时间戳由调用方或 `Project::create` 填充）。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            format: default_format(),
            min_sdk: crate::SDK_VERSION,
            name: name.into(),
            display_name: None,
            description: String::new(),
            author: None,
            author_sign: None,
            license: None,
            tags: Vec::new(),
            created_time: 0,
            last_update_time: 0,
            version: default_version(),
            extra: ExtraMap::new(),
        }
    }

    pub fn display(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }

    /// 参与作者签名的规范化载荷（键排序、无空白）。
    pub fn sign_payload(&self, registered_files: &[String]) -> String {
        let mut files = registered_files.to_vec();
        files.sort();
        let obj = serde_json::json!({
            "format": self.format,
            "min_sdk": self.min_sdk,
            "name": self.name,
            "registered_files": files,
            "version": self.version,
        });
        // serde_json 的 Value 使用 BTreeMap（未启用 preserve_order）时天然有序；
        // 这里显式排序保证与参考实现一致。
        canonical_json(&obj)
    }
}

/// 生成"键按字母序、无空白"的 JSON 文本。
pub fn canonical_json(value: &Value) -> String {
    fn write(v: &Value, out: &mut String) {
        match v {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push('{');
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(k).unwrap_or_default());
                    out.push(':');
                    write(&map[*k], out);
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
        }
    }
    let mut s = String::new();
    write(value, &mut s);
    s
}

/// `registry.json`：注册资源清单。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub registered_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_sign: Option<TimestampSign>,
    #[serde(flatten, default)]
    pub extra: ExtraMap,
}

impl Registry {
    /// 去重 + 排序，并同步 `asset_count`。
    pub fn normalize(&mut self) {
        self.registered_files.sort();
        self.registered_files.dedup();
        self.asset_count = Some(self.registered_files.len() as i64);
    }
}

/// `metadata/<相对路径>.json`：单个资源的档案。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetMetadata {
    pub path: String,
    #[serde(rename = "type", default = "default_type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub sha256: String,
    #[serde(default = "default_hash_algorithm")]
    pub hash_algorithm: String,
    #[serde(default)]
    pub created_time: i64,
    #[serde(default)]
    pub last_update_time: i64,
    /// 分类可选字段（`width`/`height`/`duration_seconds`/`vertices`…）与未知字段。
    #[serde(flatten, default)]
    pub properties: ExtraMap,
    /// 显式扩展容器。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<ExtraMap>,
}

impl Default for AssetMetadata {
    fn default() -> Self {
        Self {
            path: String::new(),
            kind: default_type(),
            mime: None,
            format: None,
            size: 0,
            sha256: String::new(),
            hash_algorithm: default_hash_algorithm(),
            created_time: 0,
            last_update_time: 0,
            properties: ExtraMap::new(),
            extra: None,
        }
    }
}

impl AssetMetadata {
    /// 依据路径与内容构建元数据（分类 / MIME / 格式自动推断）。
    pub fn from_bytes(path: &str, bytes: &[u8], now: i64) -> Self {
        Self {
            path: path.to_string(),
            kind: crate::mime::guess_type(path).to_string(),
            mime: Some(crate::mime::guess_mime(path).to_string()),
            format: crate::mime::format_of(path),
            size: bytes.len() as u64,
            sha256: crate::sha::sha256_hex(bytes),
            hash_algorithm: default_hash_algorithm(),
            created_time: now,
            last_update_time: now,
            properties: ExtraMap::new(),
            extra: None,
        }
    }

    pub fn width(&self) -> Option<i64> {
        self.properties.get("width").and_then(Value::as_i64)
    }

    pub fn height(&self) -> Option<i64> {
        self.properties.get("height").and_then(Value::as_i64)
    }

    pub fn duration_seconds(&self) -> Option<f64> {
        self.properties.get("duration_seconds").and_then(Value::as_f64)
    }

    pub fn is_text(&self) -> bool {
        self.kind == "text"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_keeps_unknown_fields_in_extra() {
        let text = r#"{"name":"demo","vendor":{"x":1},"version":3}"#;
        let info: Info = crate::json::from_str(text).unwrap();
        assert_eq!(info.name, "demo");
        assert_eq!(info.version, 3);
        assert!(info.extra.contains_key("vendor"));
    }

    #[test]
    fn info_serialization_omits_absent_optionals() {
        let mut info = Info::new("demo");
        info.created_time = 10;
        info.last_update_time = 10;
        let s = crate::json::to_string_pretty_tab(&info).unwrap();
        assert!(!s.contains("display_name"));
        assert!(!s.contains("author"));
        assert!(!s.contains("author_sign"));
        assert!(s.contains("\"format\": \"amproj\""));
        assert!(s.contains("\"tags\": []"));
    }

    #[test]
    fn metadata_folds_unknown_keys_into_properties() {
        let text = r#"{"path":"assets/images/a.png","type":"image","size":1,"sha256":"aa","width":64,"height":64,"custom":true}"#;
        let meta: AssetMetadata = crate::json::from_str(text).unwrap();
        assert_eq!(meta.width(), Some(64));
        assert_eq!(meta.height(), Some(64));
        assert_eq!(meta.properties.get("custom").and_then(Value::as_bool), Some(true));
        assert_eq!(meta.hash_algorithm, "sha256");
    }

    #[test]
    fn metadata_from_bytes_infers_type_and_hash() {
        let meta = AssetMetadata::from_bytes("assets/images/a.png", b"hello", 7);
        assert_eq!(meta.kind, "image");
        assert_eq!(meta.mime.as_deref(), Some("image/png"));
        assert_eq!(meta.size, 5);
        assert_eq!(meta.sha256, crate::sha::sha256_hex(b"hello"));
    }

    #[test]
    fn registry_normalize_sorts_and_sets_count() {
        let mut r = Registry {
            registered_files: vec!["assets/b".into(), "assets/a".into(), "assets/a".into()],
            ..Default::default()
        };
        r.normalize();
        assert_eq!(r.registered_files, vec!["assets/a", "assets/b"]);
        assert_eq!(r.asset_count, Some(2));
    }

    #[test]
    fn sign_payload_is_canonical() {
        let mut info = Info::new("demo");
        info.version = 2;
        let payload = info.sign_payload(&["assets/b".into(), "assets/a".into()]);
        assert_eq!(
            payload,
            r#"{"format":"amproj","min_sdk":1,"name":"demo","registered_files":["assets/a","assets/b"],"version":2}"#
        );
    }
}
