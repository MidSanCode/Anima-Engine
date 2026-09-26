//! `am-eval`：参数求值内核（引擎的心脏）。
//!
//! 输入「模型文档 + 参数取值」，输出「可直接渲染的场景」：
//!
//! ```text
//! ParamStore ──► 关键形插值与合成 ──► 节点变换链 ──► Scene（画布空间顶点/不透明度/遮罩）
//!    (params)        (keyform)          (scene)
//! ```
//!
//! 三个子模块各司其职：
//!
//! - [`params`]：参数取值表，所有写入的唯一入口；
//! - [`keyform`]：单参数插值 + 多参数合成（含四种混合类型）；
//! - [`warp`]：自由变形的双线性网格映射；
//! - [`scene`]：变换链构建与场景组装（含编辑器手柄所需的画布坐标）。
//!
//! 本 crate 不含任何 I/O、不依赖渲染后端，因此 native 与 wasm 行为完全一致，
//! 可以直接作为「权威求值结果」用于黄金图对比测试。

pub mod keyform;
pub mod params;
pub mod scene;
pub mod warp;

pub use keyform::{blend_keyforms, evaluate_node, sample_keyforms, Deform, RestValues};
pub use params::ParamStore;
pub use scene::{
    apply_ops, evaluate, fold_affine, DrawableInstance, Evaluated, NodeView, PointOp, Scene,
};
pub use warp::WarpMap;
