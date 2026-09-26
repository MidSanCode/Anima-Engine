//! 错误类型。

/// `am-format` 的错误。
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),

    #[error("压缩包错误: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("非法工程: {0}")]
    InvalidProject(String),

    #[error("非法路径: {0}")]
    InvalidPath(String),

    #[error("资源不存在: {0}")]
    AssetNotFound(String),

    #[error("资源已注册: {0}")]
    DuplicateAsset(String),

    #[error("工程校验未通过:\n{0}")]
    ValidationFailed(String),

    #[error("{0}")]
    Other(String),
}

impl FormatError {
    pub fn other(msg: impl Into<String>) -> Self {
        FormatError::Other(msg.into())
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        FormatError::InvalidProject(msg.into())
    }
}

/// 便捷结果别名。
pub type Result<T> = std::result::Result<T, FormatError>;
