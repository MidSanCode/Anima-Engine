//! 扩展名 → 分类 / MIME / 格式标识 的映射（与参考实现的映射表保持一致）。

/// 默认 MIME。
pub const DEFAULT_MIME: &str = "application/octet-stream";

/// 按扩展名推断资源分类；未知返回 `"binary"`。
pub fn guess_type(rel: &str) -> &'static str {
    match crate::path::ext_of(rel).as_str() {
        "jpg" | "jpeg" | "png" | "svg" | "webp" | "gif" | "bmp" | "ico" => "image",
        "mp3" | "wav" | "ogg" | "flac" | "m4a" | "aac" | "opus" => "audio",
        "mp4" | "webm" | "mkv" | "mov" => "video",
        "obj" | "gltf" | "glb" | "fbx" | "blend" | "stl" => "model",
        "txt" | "md" | "json" | "xml" | "yaml" | "yml" | "csv" => "text",
        _ => "binary",
    }
}

/// 按扩展名推断 MIME。
pub fn guess_mime(rel: &str) -> &'static str {
    match crate::path::ext_of(rel).as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "opus" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "obj" => "text/plain",
        "gltf" => "model/gltf+json",
        "glb" => "model/gltf-binary",
        "fbx" | "blend" => "application/octet-stream",
        "stl" => "model/stl",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "json" => "application/json",
        "xml" => "application/xml",
        "yaml" | "yml" => "application/yaml",
        "csv" => "text/csv",
        _ => DEFAULT_MIME,
    }
}

/// 按扩展名推断 `format` 字段（应用别名表）；无扩展名返回 `None`。
pub fn format_of(rel: &str) -> Option<String> {
    let ext = crate::path::ext_of(rel);
    if ext.is_empty() {
        return None;
    }
    Some(match ext.as_str() {
        "jpg" => "jpeg".to_string(),
        "yml" => "yaml".to_string(),
        other => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpg_aliases_to_jpeg_format() {
        assert_eq!(guess_type("assets/images/a.jpg"), "image");
        assert_eq!(guess_mime("assets/images/a.jpg"), "image/jpeg");
        assert_eq!(format_of("assets/images/a.jpg").as_deref(), Some("jpeg"));
    }

    #[test]
    fn unknown_extension_is_binary() {
        assert_eq!(guess_type("assets/data/x.bin"), "binary");
        assert_eq!(guess_mime("assets/data/x.bin"), DEFAULT_MIME);
        assert_eq!(format_of("assets/data/x.bin").as_deref(), Some("bin"));
        assert_eq!(format_of("assets/data/x"), None);
    }

    #[test]
    fn text_types_are_utf8_checked_by_validator() {
        assert_eq!(guess_type("assets/images/b.svg"), "image");
        assert_eq!(guess_type("assets/spec/steps.json"), "text");
    }
}
