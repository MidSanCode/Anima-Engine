//! 标识符生成与校验。

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// 文档内实体标识符。
pub type Id = String;

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// 生成一个新 id：`<prefix><时间纳秒><进程内计数器>`。
///
/// 不依赖随机数库，因此在 native / wasm 上行为一致。
pub fn new_id(prefix: &str) -> Id {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}{now:x}{n:x}")
}

pub fn node_id() -> Id {
    new_id("n")
}

pub fn parameter_id() -> Id {
    new_id("p")
}

pub fn texture_id() -> Id {
    new_id("t")
}

pub fn motion_id() -> Id {
    new_id("m")
}

pub fn expression_id() -> Id {
    new_id("e")
}

pub fn physics_id() -> Id {
    new_id("x")
}

/// id 是否非空且只包含安全字符。
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// 供 UI 展示的 id 包装（保留原始字符串）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IdRef(pub Id);

impl IdRef {
    pub fn new(id: impl Into<Id>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn generated_ids_are_unique_and_prefixed() {
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let id = new_id("n");
            assert!(id.starts_with('n'));
            assert!(seen.insert(id), "重复 id");
        }
    }

    #[test]
    fn id_validation() {
        assert!(is_valid_id("n1a2b3"));
        assert!(is_valid_id("node-1_2"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("has space"));
        assert!(!is_valid_id("汉字"));
    }
}
