use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::panic::{AssertUnwindSafe, catch_unwind};

use handlebars::{Handlebars, no_escape};
use json_formula_rs::JsonFormula;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use thiserror::Error;

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

pub fn load_profile(path: impl AsRef<Path>) -> Result<CompiledProfile, EvaluatorError> {
    let mut visiting = HashSet::new();
    load_profile_internal(path.as_ref(), &mut visiting)
}

fn load_profile_internal(path: &Path, visiting: &mut HashSet<PathBuf>) -> Result<CompiledProfile, EvaluatorError> {
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
        let map = item
            .as_object()
            .ok_or_else(|| EvaluatorError::InvalidProfile("section item must be a mapping".to_string()))?;

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
                    if let Some(profile_obj) = state.context.get_mut("profile").and_then(Value::as_object_mut) {
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
                        if let Some(profile_obj) = state.context.get_mut("profile").and_then(Value::as_object_mut) {
                            profile_obj.insert(statement.id.clone(), value.clone());
                        }
                    }

                    let include_report_text = statement.expression.is_none() || statement.report_text.is_object();
                    if include_report_text {
                        let report_text_source =
                            select_text_value(&statement.report_text, expr_value.as_ref(), &state.language);
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

pub fn evaluate_files(profile_path: impl AsRef<Path>, indicators_path: impl AsRef<Path>) -> Result<Value, EvaluatorError> {
    let profile = load_profile(profile_path)?;
    let indicators: Value = serde_json::from_str(&fs::read_to_string(indicators_path)?)?;
    evaluate(&profile, &indicators)
}

pub fn serialize_report(report: &Value, format: OutputFormat) -> Result<String, EvaluatorError> {
    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(report)?),
        OutputFormat::Yaml => Ok(serde_yaml::to_string(report)?),
    }
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
        if let Some(caps) = full_expr_re.captures(input) {
            if let Some(expr) = caps.get(1).map(|m| m.as_str()) {
                return self.eval_expression(expr);
            }
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
        obj.entry("profile").or_insert_with(|| Value::Object(Map::new()));
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
        assert_eq!(dst, json!({"a": {"x": 1, "y": 3, "z": 4}, "b": {"nested": true}}));
    }
}
