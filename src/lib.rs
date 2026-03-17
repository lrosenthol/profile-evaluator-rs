/*
Copyright 2026 Adobe. All rights reserved.
This file is licensed to you under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License. You may obtain a copy
of the License at http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software distributed under
the License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR REPRESENTATIONS
OF ANY KIND, either express or implied. See the License for the specific language
governing permissions and limitations under the License.
*/

use std::collections::{BTreeMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
use std::fs;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use handlebars::{Handlebars, no_escape};
use json_formula_rs::JsonFormula;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use thiserror::Error;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[derive(Debug, Error)]
pub enum EvaluatorError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("json parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid profile: {0}")]
    InvalidProfile(String),
}

#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Json,
    Yaml,
}

#[derive(Debug, Clone)]
pub struct CompiledProfile {
    info: Value,
    sections: Vec<Vec<ProfileItem>>,
}

#[derive(Debug, Clone)]
enum ProfileItem {
    Block(BlockEntry),
    Statement(StatementEntry),
}

#[derive(Debug, Clone, Deserialize)]
struct BlockEntry {
    name: String,
    value: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct StatementEntry {
    id: String,
    #[allow(dead_code)]
    description: Option<Value>,
    title: Option<Value>,
    expression: Option<String>,
    report_text: Value,
}

#[derive(Debug, Deserialize)]
struct BlockWrapper {
    block: BlockEntry,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_profile(path: impl AsRef<Path>) -> Result<CompiledProfile, EvaluatorError> {
    let mut visiting: HashSet<PathBuf> = HashSet::new();
    load_profile_internal(path.as_ref(), &mut visiting)
}

#[cfg(not(target_arch = "wasm32"))]
fn load_profile_internal(
    path: &Path,
    visiting: &mut HashSet<PathBuf>,
) -> Result<CompiledProfile, EvaluatorError> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visiting.insert(canonical.clone()) {
        return Err(EvaluatorError::InvalidProfile(format!(
            "include cycle detected at {}",
            path.display()
        )));
    }

    let text = fs::read_to_string(path)?;
    let docs: Vec<Value> = serde_yaml::Deserializer::from_str(&text)
        .map(Value::deserialize)
        .collect::<Result<Vec<_>, _>>()?;

    if docs.is_empty() {
        visiting.remove(&canonical);
        return Err(EvaluatorError::InvalidProfile(format!(
            "profile {} has no YAML documents",
            path.display()
        )));
    }

    let mut info = docs[0].clone();
    if !info.is_object() {
        visiting.remove(&canonical);
        return Err(EvaluatorError::InvalidProfile(
            "first profile document must be a mapping".to_string(),
        ));
    }

    let include_list = extract_include_list(&mut info)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));

    let mut included_info = Value::Object(Map::new());
    let mut included_sections = Vec::new();

    for include in include_list {
        let include_path = if Path::new(&include).is_absolute() {
            PathBuf::from(&include)
        } else {
            parent.join(&include)
        };
        let included = load_profile_internal(&include_path, visiting)?;
        deep_merge(&mut included_info, included.info);
        included_sections.extend(included.sections);
    }

    let mut current_sections = Vec::new();
    for doc in docs.into_iter().skip(1) {
        let section = parse_section(doc)?;
        current_sections.push(section);
    }

    deep_merge(&mut included_info, info);
    current_sections.extend(included_sections);

    visiting.remove(&canonical);
    Ok(CompiledProfile {
        info: included_info,
        sections: current_sections,
    })
}

fn extract_include_list(info: &mut Value) -> Result<Vec<String>, EvaluatorError> {
    let Some(map) = info.as_object_mut() else {
        return Ok(Vec::new());
    };

    let Some(include_value) = map.remove("include") else {
        return Ok(Vec::new());
    };

    let Some(items) = include_value.as_array() else {
        return Err(EvaluatorError::InvalidProfile(
            "include must be a YAML array".to_string(),
        ));
    };

    let mut paths = Vec::new();
    for item in items {
        let Some(path) = item.as_str() else {
            return Err(EvaluatorError::InvalidProfile(
                "include entries must be string file paths".to_string(),
            ));
        };
        paths.push(path.to_string());
    }

    Ok(paths)
}

fn parse_section(doc: Value) -> Result<Vec<ProfileItem>, EvaluatorError> {
    let Some(items) = doc.as_array() else {
        return Err(EvaluatorError::InvalidProfile(
            "section document must be a YAML array".to_string(),
        ));
    };

    let mut out = Vec::new();
    for item in items {
        let map = item.as_object().ok_or_else(|| {
            EvaluatorError::InvalidProfile("section item must be a mapping".to_string())
        })?;

        if map.contains_key("block") {
            let wrapper: BlockWrapper = serde_json::from_value(item.clone())?;
            out.push(ProfileItem::Block(wrapper.block));
        } else {
            let statement: StatementEntry = serde_json::from_value(item.clone())?;
            out.push(ProfileItem::Statement(statement));
        }
    }

    Ok(out)
}

pub fn evaluate(profile: &CompiledProfile, indicators: &Value) -> Result<Value, EvaluatorError> {
    let language = profile
        .info
        .get("profile_metadata")
        .and_then(|v| v.get("language"))
        .and_then(Value::as_str)
        .unwrap_or("en")
        .to_string();

    let globals = profile
        .info
        .get("variables")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));

    let expression_map = profile
        .info
        .get("expressions")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| Some((k, v.as_str()?.to_string())))
        .collect::<BTreeMap<_, _>>();

    let mut context = indicators.clone();
    deep_merge(&mut context, profile.info.clone());
    ensure_profile_object(&mut context);

    let mut hb = Handlebars::new();
    hb.set_strict_mode(true);
    hb.register_escape_fn(no_escape);

    let mut state = EvalState {
        context,
        globals,
        expressions: expression_map,
        language,
        jf: JsonFormula::new(),
        hb,
    };

    for (name, body) in &state.expressions {
        state.jf.register_expression(name, body).map_err(|e| {
            EvaluatorError::InvalidProfile(format!("invalid expression \"{}\": {}", name, e))
        })?;
    }

    let resolved_info = state.render_value(&profile.info);
    if let Some(ctx) = state.context.as_object_mut() {
        ctx.extend(resolved_info.as_object().cloned().unwrap_or_default());
    }

    let mut report = Map::new();
    let mut statements_out = Vec::new();

    for section in &profile.sections {
        let mut section_entries = Vec::new();

        for item in section {
            match item {
                ProfileItem::Block(block) => {
                    let rendered = state.render_value(&block.value);
                    report.insert(block.name.clone(), rendered.clone());
                    if let Some(profile_obj) = state
                        .context
                        .get_mut("profile")
                        .and_then(Value::as_object_mut)
                    {
                        profile_obj.insert(block.name.clone(), rendered);
                    }
                }
                ProfileItem::Statement(statement) => {
                    let mut out = Map::new();
                    out.insert("id".to_string(), Value::String(statement.id.clone()));

                    if let Some(title) = &statement.title {
                        let selected = select_text_value(title, None, &state.language);
                        let rendered = state.render_value(&selected);
                        if let Some(s) = rendered.as_str() {
                            out.insert("title".to_string(), Value::String(s.to_string()));
                        }
                    }

                    let expr_value = statement
                        .expression
                        .as_ref()
                        .map(|expr| state.eval_expression(expr));

                    if let Some(value) = &expr_value {
                        out.insert("value".to_string(), value.clone());
                        if let Some(profile_obj) = state
                            .context
                            .get_mut("profile")
                            .and_then(Value::as_object_mut)
                        {
                            profile_obj.insert(statement.id.clone(), value.clone());
                        }
                    }

                    let include_report_text =
                        statement.expression.is_none() || statement.report_text.is_object();
                    if include_report_text {
                        let report_text_source = select_text_value(
                            &statement.report_text,
                            expr_value.as_ref(),
                            &state.language,
                        );
                        let rendered_text = state.render_value(&report_text_source);
                        if let Some(s) = rendered_text.as_str() {
                            out.insert("report_text".to_string(), Value::String(s.to_string()));
                        }
                    }

                    section_entries.push(Value::Object(out));
                }
            }
        }

        if !section_entries.is_empty() {
            statements_out.push(Value::Array(section_entries));
        }
    }

    if !statements_out.is_empty() {
        report.insert("statements".to_string(), Value::Array(statements_out));
    }

    Ok(Value::Object(report))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn evaluate_files(
    profile_path: impl AsRef<Path>,
    indicators_path: impl AsRef<Path>,
) -> Result<Value, EvaluatorError> {
    let profile = load_profile(profile_path)?;
    let indicators: Value = serde_json::from_str(&fs::read_to_string(indicators_path)?)?;
    evaluate(&profile, &indicators)
}

pub fn load_profile_from_yaml_str(
    yaml_text: &str,
    include_base_dir: Option<&Path>,
) -> Result<CompiledProfile, EvaluatorError> {
    let docs: Vec<Value> = serde_yaml::Deserializer::from_str(yaml_text)
        .map(Value::deserialize)
        .collect::<Result<Vec<_>, _>>()?;

    if docs.is_empty() {
        return Err(EvaluatorError::InvalidProfile(
            "profile has no YAML documents".to_string(),
        ));
    }

    let mut info = docs[0].clone();
    if !info.is_object() {
        return Err(EvaluatorError::InvalidProfile(
            "first profile document must be a mapping".to_string(),
        ));
    }

    let include_list = extract_include_list(&mut info)?;
    #[cfg(not(target_arch = "wasm32"))]
    let mut visiting: HashSet<PathBuf> = HashSet::new();
    #[cfg(target_arch = "wasm32")]
    let mut visiting: HashSet<String> = HashSet::new();
    let mut included_info = Value::Object(Map::new());
    #[cfg(not(target_arch = "wasm32"))]
    let mut included_sections = Vec::new();
    #[cfg(target_arch = "wasm32")]
    let included_sections = Vec::new();

    for include in include_list {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = include_base_dir;
            let _ = &mut visiting;
            return Err(EvaluatorError::InvalidProfile(format!(
                "profile include is not supported in the WASM build: {include}"
            )));
        }

        #[cfg(not(target_arch = "wasm32"))]
        let include_path = if Path::new(&include).is_absolute() {
            PathBuf::from(&include)
        } else if let Some(base) = include_base_dir {
            base.join(&include)
        } else {
            PathBuf::from(&include)
        };

        #[cfg(not(target_arch = "wasm32"))]
        let included = load_profile_internal(&include_path, &mut visiting)?;
        #[cfg(not(target_arch = "wasm32"))]
        {
            deep_merge(&mut included_info, included.info);
            included_sections.extend(included.sections);
        }
    }

    let mut current_sections = Vec::new();
    for doc in docs.into_iter().skip(1) {
        let section = parse_section(doc)?;
        current_sections.push(section);
    }

    deep_merge(&mut included_info, info);
    current_sections.extend(included_sections);

    Ok(CompiledProfile {
        info: included_info,
        sections: current_sections,
    })
}

pub fn evaluate_texts(
    profile_yaml: &str,
    indicators_json: &str,
    include_base_dir: Option<&Path>,
) -> Result<Value, EvaluatorError> {
    let profile = load_profile_from_yaml_str(profile_yaml, include_base_dir)?;
    let indicators: Value = serde_json::from_str(indicators_json)?;
    evaluate(&profile, &indicators)
}

pub fn serialize_report(report: &Value, format: OutputFormat) -> Result<String, EvaluatorError> {
    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(report)?),
        OutputFormat::Yaml => Ok(serde_yaml::to_string(report)?),
    }
}

#[cfg(target_arch = "wasm32")]
fn js_error(err: impl ToString) -> JsValue {
    JsValue::from_str(&err.to_string())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn evaluate_profile_wasm(
    profile_yaml: &str,
    indicators_json: &str,
) -> Result<JsValue, JsValue> {
    let report = evaluate_texts(profile_yaml, indicators_json, None).map_err(js_error)?;
    // Return JSON string so the JS side gets a plain object via JSON.parse();
    // serde_wasm_bindgen::to_value can produce Map instances, which break result.statements in the UI.
    let json_string = serde_json::to_string(&report).map_err(js_error)?;
    Ok(JsValue::from_str(&json_string))
}

struct EvalState {
    context: Value,
    globals: Value,
    expressions: BTreeMap<String, String>,
    language: String,
    jf: JsonFormula,
    hb: Handlebars<'static>,
}

impl EvalState {
    /// Evaluates a JSON Formula expression string against the current context and globals.
    ///
    /// For no-arg custom functions (`_Name()`), chooses the evaluation context: if the
    /// function body contains `$`, the full context is used; otherwise only `profile` is
    /// used so formulas can reference profile fields directly. Panics during evaluation
    /// are caught and yield `false`; formula errors yield `Null`.
    fn eval_expression(&mut self, expr: &str) -> Value {
        let no_arg_global = Regex::new(r"^\s*(_[A-Za-z0-9_]+)\(\)\s*$").expect("valid regex");
        let eval_context = if let Some(caps) = no_arg_global.captures(expr) {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            if let Some(body) = self.expressions.get(name) {
                if body.contains('$') {
                    &self.context
                } else {
                    self.context.get("profile").unwrap_or(&self.context)
                }
            } else {
                &self.context
            }
        } else {
            &self.context
        };

        match catch_unwind(AssertUnwindSafe(|| {
            self.jf
                .search(expr, eval_context, Some(&self.globals), Some("en-US"))
        })) {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => Value::Null,
            Err(_) => Value::Bool(false),
        }
    }

    /// Recursively renders a value for report output. Strings are processed by
    /// `render_string_value` (Handlebars plus `{{ expr "..." }}` evaluation); arrays
    /// and objects are traversed so each element/field is rendered; other types are
    /// returned unchanged.
    fn render_value(&mut self, value: &Value) -> Value {
        match value {
            Value::String(s) => self.render_string_value(s),
            Value::Array(arr) => Value::Array(arr.iter().map(|v| self.render_value(v)).collect()),
            Value::Object(obj) => {
                let mut out = Map::new();
                for (k, v) in obj {
                    out.insert(k.clone(), self.render_value(v));
                }
                Value::Object(out)
            }
            _ => value.clone(),
        }
    }

    /// Renders a single string: evaluates full-string `{{ expr "..." }}` as an expression
    /// (returning the result as-is), then replaces any inline `{{ expr "..." }}` segments
    /// with their evaluated string form, and finally runs the result through Handlebars
    /// with the current context. On Handlebars failure, falls back to `fallback_render`.
    fn render_string_value(&mut self, input: &str) -> Value {
        let full_expr_re =
            Regex::new(r#"(?s)^\s*\{\{\s*expr\s+"(.+?)"\s*\}\}\s*$"#).expect("valid regex");
        if let Some(caps) = full_expr_re.captures(input)
            && let Some(expr) = caps.get(1).map(|m| m.as_str()) {
                return self.eval_expression(expr);
            }

        let mut rendered = input.to_string();
        let expr_re = Regex::new(r#"\{\{\s*expr\s+"(.+?)"\s*\}\}"#).expect("valid regex");
        rendered = expr_re
            .replace_all(&rendered, |caps: &regex::Captures| {
                let expr = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
                let value = self.eval_expression(expr);
                json_to_inline_string(&value)
            })
            .to_string();

        match self.hb.render_template(&rendered, &self.context) {
            Ok(value) => Value::String(value),
            Err(_) => Value::String(self.fallback_render(&rendered)),
        }
    }

    /// Simple template fallback when Handlebars rendering fails. Replaces `{{ path }}`
    /// tokens by looking up the dot-separated path in the context; missing keys are
    /// shown as `🔴 Missing: path()`. Leaves `{{ expr "..." }}` blocks unchanged so
    /// they can be handled elsewhere or surface as raw text.
    fn fallback_render(&mut self, input: &str) -> String {
        let token_re = Regex::new(r"\{\{\s*([^{}]+?)\s*\}\}").expect("valid regex");
        token_re
            .replace_all(input, |caps: &regex::Captures| {
                let token = caps
                    .get(1)
                    .map(|m| m.as_str().trim())
                    .unwrap_or_default()
                    .to_string();

                if token.starts_with("expr ") {
                    return caps
                        .get(0)
                        .map(|m| m.as_str())
                        .unwrap_or_default()
                        .to_string();
                }

                match lookup_path(&self.context, &token) {
                    Some(v) => json_to_inline_string(v),
                    None => format!("🔴 Missing: {token}()"),
                }
            })
            .to_string()
    }
}

fn select_text_value(value: &Value, expr_value: Option<&Value>, language: &str) -> Value {
    if let Some(obj) = value.as_object() {
        if let Some(selected) = expr_value {
            let key = match selected {
                Value::Bool(b) => b.to_string(),
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Null => "null".to_string(),
                _ => json_to_inline_string(selected),
            };

            if let Some(found) = obj.get(&key) {
                return select_language(found, language);
            }
        }

        return select_language(value, language);
    }

    value.clone()
}

fn select_language(value: &Value, language: &str) -> Value {
    let Some(obj) = value.as_object() else {
        return value.clone();
    };

    if let Some(v) = obj.get(language) {
        return v.clone();
    }
    if let Some(v) = obj.get("en") {
        return v.clone();
    }
    obj.values().next().cloned().unwrap_or(Value::Null)
}

fn lookup_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn json_to_inline_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
        }
    }
}

fn ensure_profile_object(context: &mut Value) {
    if !context.is_object() {
        *context = json!({});
    }
    if let Some(obj) = context.as_object_mut() {
        obj.entry("profile")
            .or_insert_with(|| Value::Object(Map::new()));
    }
}

fn deep_merge(dst: &mut Value, src: Value) {
    match (dst, src) {
        (Value::Object(dst_map), Value::Object(src_map)) => {
            for (k, v) in src_map {
                if let Some(existing) = dst_map.get_mut(&k) {
                    deep_merge(existing, v);
                } else {
                    dst_map.insert(k, v);
                }
            }
        }
        (dst_slot, src_val) => {
            *dst_slot = src_val;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_full_expr_template_to_non_string() {
        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        hb.register_escape_fn(no_escape);
        let mut state = EvalState {
            context: json!({"value": 2, "profile": {}}),
            globals: json!({}),
            expressions: BTreeMap::new(),
            language: "en".to_string(),
            jf: JsonFormula::new(),
            hb,
        };

        let rendered = state.render_value(&Value::String("{{ expr \"1+1\" }}".to_string()));
        assert_eq!(rendered, json!(2));
    }

    #[test]
    fn missing_template_token_has_marker() {
        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        hb.register_escape_fn(no_escape);
        let mut state = EvalState {
            context: json!({"profile": {}}),
            globals: json!({}),
            expressions: BTreeMap::new(),
            language: "en".to_string(),
            jf: JsonFormula::new(),
            hb,
        };

        let rendered = state.render_value(&Value::String("Foo {{bar}}".to_string()));
        assert_eq!(rendered, Value::String("Foo 🔴 Missing: bar()".to_string()));
    }

    #[test]
    fn deep_merge_overrides_scalars_and_merges_maps() {
        let mut dst = json!({"a": {"x": 1, "y": 2}, "b": 5});
        let src = json!({"a": {"y": 3, "z": 4}, "b": {"nested": true}});

        deep_merge(&mut dst, src);
        assert_eq!(
            dst,
            json!({"a": {"x": 1, "y": 3, "z": 4}, "b": {"nested": true}})
        );
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// Minimal valid profile (no includes): one info doc + one section with one statement.
    const MINIMAL_PROFILE: &str = r#"---
profile_metadata:
  name: WASM Test Profile
  language: en
---
- id: wasm_test_statement
  title: WASM test
  report_text: Result from WASM
"#;

    /// Minimal valid indicators JSON (object with optional content).
    const MINIMAL_INDICATORS: &str = r#"{"content":{}}"#;

    #[wasm_bindgen_test]
    fn evaluate_texts_returns_non_empty_report() {
        let report = evaluate_texts(MINIMAL_PROFILE, MINIMAL_INDICATORS, None)
            .expect("evaluate_texts should succeed");
        let statements = report
            .get("statements")
            .and_then(|v| v.as_array())
            .expect("report should have statements array");
        assert!(!statements.is_empty(), "statements should not be empty");
        assert!(
            report.get("profile_metadata").is_some() || report.get("statements").is_some(),
            "report should have profile_metadata or statements"
        );
    }

    #[wasm_bindgen_test]
    fn evaluate_texts_report_has_expected_structure() {
        let report = evaluate_texts(MINIMAL_PROFILE, MINIMAL_INDICATORS, None)
            .expect("evaluate_texts should succeed");
        let top_level: Vec<&str> = report
            .as_object()
            .map(|m| m.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert!(
            top_level.contains(&"statements"),
            "report should contain 'statements' key, got: {:?}",
            top_level
        );
        let first_section = report["statements"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v.as_array());
        assert!(first_section.is_some(), "statements should be array of sections");
        let first_stmt = first_section.and_then(|s| s.first()).and_then(|v| v.as_object());
        assert!(first_stmt.is_some(), "first section should have statement objects");
        let id = first_stmt
            .and_then(|o| o.get("id"))
            .and_then(|v| v.as_str());
        assert_eq!(id, Some("wasm_test_statement"), "first statement id should match");
    }

    /// Exercises the same path the browser uses: evaluate_profile_wasm returns JSON string -> parse.
    #[wasm_bindgen_test]
    fn evaluate_profile_wasm_returns_serializable_report() {
        let js_value = evaluate_profile_wasm(MINIMAL_PROFILE, MINIMAL_INDICATORS)
            .expect("evaluate_profile_wasm should succeed");
        let json_string = js_value
            .as_string()
            .expect("evaluate_profile_wasm returns a JSON string");
        let report: Value = serde_json::from_str(&json_string).expect("JSON string should parse as report");
        let statements = report
            .get("statements")
            .and_then(|v| v.as_array())
            .expect("report should have statements array");
        assert!(!statements.is_empty(), "statements should not be empty");
    }

    /// Real-world testfiles from testfiles/ and output/ (embedded at compile time).
    /// Excludes "includes" because WASM build does not support YAML include:.
    const REAL_WORLD_CASES: &[(&str, &str, &str, &str)] = &[
        (
            "blocks",
            include_str!("../testfiles/blocks_profile.yml"),
            include_str!("../testfiles/blocks_indicators.json"),
            include_str!("../output/blocks_indicators_report.json"),
        ),
        (
            "camera",
            include_str!("../testfiles/camera_profile.yml"),
            include_str!("../testfiles/camera_indicators.json"),
            include_str!("../output/camera_indicators_report.json"),
        ),
        (
            "expression",
            include_str!("../testfiles/expression_profile.yml"),
            include_str!("../testfiles/expression_indicators.json"),
            include_str!("../output/expression_indicators_report.json"),
        ),
        (
            "genai",
            include_str!("../testfiles/genai_profile.yml"),
            include_str!("../testfiles/genai_indicators.json"),
            include_str!("../output/genai_indicators_report.json"),
        ),
        (
            "no_manifests",
            include_str!("../testfiles/no_manifests_profile.yml"),
            include_str!("../testfiles/no_manifests_indicators.json"),
            include_str!("../output/no_manifests_indicators_report.json"),
        ),
        (
            "signature",
            include_str!("../testfiles/signature_profile.yml"),
            include_str!("../testfiles/signature_indicators.json"),
            include_str!("../output/signature_indicators_report.json"),
        ),
    ];

    #[wasm_bindgen_test]
    fn real_world_testfiles_evaluate_and_match_expected() {
        for (name, profile_yaml, indicators_json, expected_json) in REAL_WORLD_CASES {
            let report = evaluate_texts(profile_yaml, indicators_json, None)
                .unwrap_or_else(|e| panic!("case {name}: evaluate_texts failed: {e}"));
            let expected: Value = serde_json::from_str(expected_json)
                .unwrap_or_else(|e| panic!("case {name}: invalid expected JSON: {e}"));
            assert_eq!(
                report, expected,
                "case {name}: report did not match expected output"
            );
        }
    }
}
