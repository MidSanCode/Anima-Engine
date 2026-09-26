//! `am-wasm`：浏览器绑定。
//!
//! 与 C ABI 完全同构：宿主拿到的仍然是「方法名 + JSON 参数 → JSON 信封」，
//! 因此 Web 端的调用代码和桌面端可以共用同一套 TypeScript 封装。
//!
//! 构建（需要 `wasm32-unknown-unknown` 目标）：
//!
//! ```bash
//! rustup target add wasm32-unknown-unknown
//! cargo build -p am-wasm --target wasm32-unknown-unknown --release --no-default-features
//! wasm-bindgen --target web --out-dir www/pkg \
//!     target/wasm32-unknown-unknown/release/anima_wasm.wasm
//! ```
//!
//! 渲染：Web 端默认**不**编译 GPU 后端（`--no-default-features`），
//! 场景数据由 `runtime.scene` 交给页面里的 WebGL/Canvas 自行绘制；
//! 若需要引擎自绘，用 `--features gpu` 打开（体积会明显变大）。

use am_core::Session;
use wasm_bindgen::prelude::*;

/// 浏览器侧引擎句柄。
#[wasm_bindgen]
pub struct Engine {
    session: Session,
}

#[wasm_bindgen]
impl Engine {
    /// 创建空引擎。
    #[wasm_bindgen(constructor)]
    pub fn new() -> Engine {
        Engine { session: Session::empty() }
    }

    /// 引擎版本号。
    #[wasm_bindgen(js_name = version)]
    pub fn version() -> String {
        am_core::ENGINE_VERSION.to_string()
    }

    /// 调用一个方法，返回 JSON 信封字符串。
    #[wasm_bindgen(js_name = call)]
    pub fn call(&mut self, method: &str, params_json: &str) -> String {
        self.session.dispatch_json(method, params_json)
    }

    /// 便捷方法：推进一帧。
    #[wasm_bindgen(js_name = advance)]
    pub fn advance(&mut self, dt: f32) {
        self.session.advance(dt);
    }

    /// 便捷方法：设置参数（自动钳制）。
    #[wasm_bindgen(js_name = setParam)]
    pub fn set_param(&mut self, id: &str, value: f32) -> bool {
        self.session.document_mut().set_param(id, value)
    }

    /// 便捷方法：当前参数快照（JSON 对象字符串）。
    #[wasm_bindgen(js_name = params)]
    pub fn params(&self) -> String {
        serde_json::to_string(self.session.document().params().as_map())
            .unwrap_or_else(|_| "{}".to_string())
    }

    /// 便捷方法：当前场景（JSON 对象字符串），供页面自行绘制。
    #[wasm_bindgen(js_name = scene)]
    pub fn scene(&self) -> String {
        match serde_json::to_string(&self.session.evaluate().scene) {
            Ok(text) => text,
            Err(e) => format!(r#"{{"error":"场景序列化失败: {e}"}}"#),
        }
    }

    /// 最近一帧像素在 wasm 内存中的起始地址（0 表示没有帧）。
    #[wasm_bindgen(js_name = framePtr)]
    pub fn frame_ptr(&self) -> *const u8 {
        let pixels = self.frame_pixels();
        if pixels.is_empty() {
            return std::ptr::null();
        }
        pixels.as_ptr()
    }

    /// 最近一帧像素长度（字节）。
    #[wasm_bindgen(js_name = frameLen)]
    pub fn frame_len(&self) -> usize {
        self.frame_pixels().len()
    }
}

impl Engine {
    #[cfg(feature = "gpu")]
    fn frame_pixels(&self) -> &[u8] {
        self.session.frame_pixels()
    }

    #[cfg(not(feature = "gpu"))]
    fn frame_pixels(&self) -> &[u8] {
        &[]
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_round_trips_through_the_envelope() {
        let mut engine = Engine::new();
        let raw = engine.call("system.ping", "");
        assert!(raw.contains("\"ok\":true"), "got {raw}");
        assert!(raw.contains("\"pong\":true"));
    }

    #[test]
    fn errors_are_structured() {
        let mut engine = Engine::new();
        let raw = engine.call("nope", "{}");
        assert!(raw.contains("-32601"), "got {raw}");
    }

    #[test]
    fn scene_is_valid_json() {
        let engine = Engine::new();
        let scene: serde_json::Value = serde_json::from_str(&engine.scene()).unwrap();
        assert!(scene["drawables"].is_array());
    }

    #[test]
    fn version_is_reported() {
        assert_eq!(Engine::version(), am_core::ENGINE_VERSION);
    }

    #[test]
    fn without_gpu_there_are_no_frame_pixels() {
        let engine = Engine::new();
        if !cfg!(feature = "gpu") {
            assert_eq!(engine.frame_len(), 0);
            assert!(engine.frame_ptr().is_null());
        }
    }
}
