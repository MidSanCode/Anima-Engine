//! 参数存储：模型参数当前取值的唯一来源。
//!
//! 求值器只读这里，任何写入（UI 拖拽、动作播放、物理、表情、自动效果）都通过本类型，
//! 从而保证「同一份参数 → 同一份画面」。

use am_model::{Expression, Id, Model, ModelSettings};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 参数当前取值表。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ParamStore {
    values: BTreeMap<Id, f32>,
}

impl ParamStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 用模型参数的默认值初始化。
    pub fn from_model(model: &Model) -> Self {
        let mut store = Self::new();
        for p in &model.parameters {
            store.values.insert(p.id.clone(), p.default);
        }
        store
    }

    /// 用模型默认值 + 模型设置中的覆盖值初始化。
    pub fn from_settings(model: &Model, settings: Option<&ModelSettings>) -> Self {
        let mut store = Self::from_model(model);
        if let Some(s) = settings {
            for (id, value) in &s.parameter_defaults {
                if store.values.contains_key(id) {
                    store.values.insert(id.clone(), *value);
                }
            }
        }
        store
    }

    /// 读取取值；不存在的参数返回 `0.0`。
    pub fn get(&self, id: &str) -> f32 {
        self.values.get(id).copied().unwrap_or(0.0)
    }

    /// 读取取值；不存在的参数返回 `None`。
    pub fn try_get(&self, id: &str) -> Option<f32> {
        self.values.get(id).copied()
    }

    /// 直接写入（不做范围限制，物理与动作可以超出范围）。
    pub fn set(&mut self, id: impl Into<Id>, value: f32) {
        self.values.insert(id.into(), if value.is_finite() { value } else { 0.0 });
    }

    /// 写入并按参数范围钳制；参数不存在时返回 `false`。
    pub fn set_clamped(&mut self, model: &Model, id: &str, value: f32) -> bool {
        match model.parameter(id) {
            Some(p) => {
                self.values.insert(p.id.clone(), p.clamp_value(value));
                true
            }
            None => false,
        }
    }

    /// 按归一化位置 `[0,1]` 写入。
    pub fn set_normalized(&mut self, model: &Model, id: &str, t: f32) -> bool {
        match model.parameter(id) {
            Some(p) => {
                let v = p.min + (p.max - p.min) * t.clamp(0.0, 1.0);
                self.values.insert(p.id.clone(), v);
                true
            }
            None => false,
        }
    }

    /// 读取归一化位置。
    pub fn get_normalized(&self, model: &Model, id: &str) -> f32 {
        match model.parameter(id) {
            Some(p) => p.normalize(self.get(id)),
            None => 0.0,
        }
    }

    /// 恢复全部参数到默认值。
    pub fn reset(&mut self, model: &Model) {
        for p in &model.parameters {
            self.values.insert(p.id.clone(), p.default);
        }
    }

    /// 批量写入（`(id, value)` 列表）。
    pub fn set_many<I, S>(&mut self, values: I)
    where
        I: IntoIterator<Item = (S, f32)>,
        S: Into<Id>,
    {
        for (id, v) in values {
            self.set(id, v);
        }
    }

    /// 按权重混合一个表情（`weight = 1` 为完全应用）。
    pub fn apply_expression(&mut self, model: &Model, expression: &Expression, weight: f32) {
        let w = weight.clamp(0.0, 1.0);
        for ep in &expression.parameters {
            let Some(param) = model.parameter(&ep.parameter) else {
                continue;
            };
            let target = param.clamp_value(ep.value);
            let current = self.get(&param.id);
            let blended = current + (target - current) * w * ep.weight;
            self.values.insert(param.id.clone(), blended);
        }
    }

    /// 只保留模型里存在的参数（丢弃已删除参数的历史值）。
    pub fn retain_model_parameters(&mut self, model: &Model) {
        let valid: std::collections::BTreeSet<&str> =
            model.parameters.iter().map(|p| p.id.as_str()).collect();
        self.values.retain(|k, _| valid.contains(k.as_str()));
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Id, &f32)> {
        self.values.iter()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn as_map(&self) -> &BTreeMap<Id, f32> {
        &self.values
    }

    /// 判断当前是否全部处于默认值。
    pub fn is_default(&self, model: &Model) -> bool {
        model.parameters.iter().all(|p| p.is_default(self.get(&p.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_model::{ExpressionParam, Parameter};

    fn model_with_params() -> Model {
        let mut m = Model::new("demo");
        m.add_parameter(Parameter::new("px", "AngleX", -30.0, 30.0, 0.0));
        m.add_parameter(Parameter::new("py", "AngleY", -20.0, 20.0, 5.0));
        m
    }

    #[test]
    fn defaults_come_from_model() {
        let m = model_with_params();
        let s = ParamStore::from_model(&m);
        assert_eq!(s.get("px"), 0.0);
        assert_eq!(s.get("py"), 5.0);
        assert_eq!(s.get("unknown"), 0.0);
        assert_eq!(s.try_get("unknown"), None);
        assert!(s.is_default(&m));
    }

    #[test]
    fn settings_override_defaults() {
        let m = model_with_params();
        let mut settings = ModelSettings::default();
        settings.parameter_defaults.insert("py".into(), -7.0);
        settings.parameter_defaults.insert("ghost".into(), 1.0);
        let s = ParamStore::from_settings(&m, Some(&settings));
        assert_eq!(s.get("py"), -7.0);
        assert!(!s.as_map().contains_key("ghost"), "不存在的参数不应被写入");
    }

    #[test]
    fn set_clamped_respects_range_and_reports_unknown() {
        let m = model_with_params();
        let mut s = ParamStore::from_model(&m);
        assert!(s.set_clamped(&m, "px", 100.0));
        assert_eq!(s.get("px"), 30.0);
        assert!(!s.set_clamped(&m, "ghost", 1.0));
    }

    #[test]
    fn normalized_get_set_round_trip() {
        let m = model_with_params();
        let mut s = ParamStore::from_model(&m);
        assert!(s.set_normalized(&m, "px", 1.0));
        assert_eq!(s.get("px"), 30.0);
        assert_eq!(s.get_normalized(&m, "px"), 1.0);
        assert!(s.set_normalized(&m, "px", 2.0));
        assert_eq!(s.get_normalized(&m, "px"), 1.0);
        assert_eq!(s.get_normalized(&m, "ghost"), 0.0);
    }

    #[test]
    fn non_finite_values_are_rejected() {
        let mut s = ParamStore::new();
        s.set("px", f32::NAN);
        assert_eq!(s.get("px"), 0.0);
    }

    #[test]
    fn expression_blends_by_weight() {
        let m = model_with_params();
        let mut s = ParamStore::from_model(&m);
        let mut e = Expression::new("e1", "smile");
        e.parameters.push(ExpressionParam::new("px", 30.0));
        s.apply_expression(&m, &e, 0.5);
        assert_eq!(s.get("px"), 15.0);
        s.apply_expression(&m, &e, 1.0);
        assert_eq!(s.get("px"), 30.0);
        // 超出参数范围的表达值会被钳制
        let mut e2 = Expression::new("e2", "extreme");
        e2.parameters.push(ExpressionParam::new("px", 999.0));
        s.apply_expression(&m, &e2, 1.0);
        assert_eq!(s.get("px"), 30.0);
    }

    #[test]
    fn expression_ignores_unknown_parameters() {
        let m = model_with_params();
        let mut s = ParamStore::from_model(&m);
        let mut e = Expression::new("e", "x");
        e.parameters.push(ExpressionParam::new("ghost", 1.0));
        s.apply_expression(&m, &e, 1.0);
        assert!(s.try_get("ghost").is_none());
    }

    #[test]
    fn reset_and_retain() {
        let m = model_with_params();
        let mut s = ParamStore::from_model(&m);
        s.set("px", 20.0);
        assert!(!s.is_default(&m));
        s.reset(&m);
        assert!(s.is_default(&m));

        s.set("ghost", 3.0);
        s.retain_model_parameters(&m);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn store_serializes_as_plain_map() {
        let mut s = ParamStore::new();
        s.set("px", 1.5);
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(text, r#"{"px":1.5}"#);
        let back: ParamStore = serde_json::from_str(&text).unwrap();
        assert_eq!(back.get("px"), 1.5);
    }
}
