//! `am-doc`：文档与命令系统。
//!
//! 编辑器对模型的**所有**修改都必须经过这里：
//!
//! ```text
//! UI 操作 ──> Command ──> Document::apply ──> 快照入撤销栈 ──> Model 更新
//!                              │
//!                              └─ Effect 列表（编辑器据此增量刷新）
//! ```
//!
//! 这样做的好处：
//! * 撤销/重做、批量原子操作、脚本化编辑天然统一；
//! * 引擎 FFI 的 `doc.command` 直接转发到 [`Document::apply_json`]；
//! * 模型永远处于「命令可复现」的状态，便于回归测试与问题复现。

pub mod command;
pub mod document;

pub use command::{quad_mesh, Command, CommandError, Effect};
pub use document::{Document, DocumentError, DEFAULT_MAX_DEPTH};
