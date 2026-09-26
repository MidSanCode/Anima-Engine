//! JSON 读写：统一使用 Tab 缩进、UTF-8 无 BOM、文件末尾单个换行。

use crate::error::Result;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::ser::PrettyFormatter;
use std::path::Path;

/// 序列化为 Tab 缩进的 JSON 文本（带结尾换行）。
pub fn to_string_pretty_tab<T: Serialize>(value: &T) -> Result<String> {
    let mut buf = Vec::with_capacity(1024);
    let formatter = PrettyFormatter::with_indent(b"\t");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    value.serialize(&mut ser)?;
    buf.push(b'\n');
    Ok(String::from_utf8(buf).map_err(|e| crate::FormatError::other(e.to_string()))?)
}

/// 从文本解析。
pub fn from_str<T: DeserializeOwned>(text: &str) -> Result<T> {
    Ok(serde_json::from_str::<T>(text)?)
}

/// 写入文件（自动创建父目录）。
pub fn write<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, to_string_pretty_tab(value)?)?;
    Ok(())
}

/// 读取并解析文件。
pub fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path)?;
    Ok(from_str::<T>(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn output_uses_tabs_and_raw_utf8() {
        let v = json!({"name": "示例", "list": [1, 2]});
        let s = to_string_pretty_tab(&v).unwrap();
        assert!(s.contains("\"示例\""), "非 ASCII 不应被转义: {s}");
        assert!(s.contains("\n\t\"name\""), "应使用 Tab 缩进: {s}");
        assert!(s.ends_with("}\n"));
    }

    #[test]
    fn round_trips_through_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nested/x.json");
        let v = json!({"a": 1});
        write(&p, &v).unwrap();
        let back: serde_json::Value = read(&p).unwrap();
        assert_eq!(back, v);
    }
}
