//! Validating JSON values against a TD DataSchema, as pydantic does in lax
//! mode.
//!
//! teta-wot validates request bodies with pydantic, so clients see
//! pydantic's coercions (the string `"2"` is accepted for an integer) and
//! its errors (`{"type": "int_parsing", "loc": ["body", "n"], "msg": …,
//! "input": …}`). [`SchemaValidator`] reproduces both from the DataSchema
//! that describes the value, and returns the coerced value, which then
//! deserialises into the Rust type.
//!
//! The coercions are listed in [`coercion`], which records how they were
//! measured against pydantic.

use std::collections::HashMap;
use std::fmt;

use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Number, Value};
use teta_wot_td::{ArrayItems, DataSchema, DataType};

/// One step of an error's location: an object key or an array index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum LocItem {
    /// An object key (or `"body"` for a request body).
    Key(String),
    /// An array index.
    Index(usize),
}

impl From<&str> for LocItem {
    fn from(key: &str) -> Self {
        LocItem::Key(key.to_owned())
    }
}

impl From<usize> for LocItem {
    fn from(index: usize) -> Self {
        LocItem::Index(index)
    }
}

impl fmt::Display for LocItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LocItem::Key(k) => f.write_str(k),
            LocItem::Index(i) => write!(f, "{i}"),
        }
    }
}

/// One validation error, in pydantic's shape.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValidationIssue {
    /// The error type, such as `int_parsing` or `missing`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Where the error is.
    pub loc: Vec<LocItem>,
    /// A human-readable message (pydantic's text).
    pub msg: String,
    /// The value that failed.
    pub input: Value,
    /// Extra context, such as the bound that was exceeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctx: Option<Map<String, Value>>,
}

/// A value didn't validate. Holds every error found, in order.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub struct ValidationError {
    /// The errors.
    pub issues: Vec<ValidationIssue>,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.issues.len();
        write!(f, "{n} validation error{}", if n == 1 { "" } else { "s" })?;
        for issue in &self.issues {
            let loc: Vec<_> = issue.loc.iter().map(ToString::to_string).collect();
            let at = if loc.is_empty() {
                String::new()
            } else {
                format!("{}: ", loc.join("."))
            };
            write!(f, "\n  {at}{} [type={}]", issue.msg, issue.kind)?;
        }
        Ok(())
    }
}

impl ValidationError {
    /// A single error.
    pub fn single(issue: ValidationIssue) -> Self {
        Self {
            issues: vec![issue],
        }
    }

    /// The error pydantic reports when a required body is absent.
    pub fn missing(loc: Vec<LocItem>, input: Value) -> Self {
        Self::single(issue("missing", loc, "Field required", input, None))
    }

    /// pydantic's `value_error`, raised by a validator: `Value error, …`,
    /// with an empty `ctx.error` (FastAPI's rendering of the exception).
    pub fn value_error(loc: Vec<LocItem>, message: &str, input: Value) -> Self {
        let mut ctx = Map::new();
        ctx.insert("error".into(), Value::Object(Map::new()));
        Self::single(issue(
            "value_error",
            loc,
            &format!("Value error, {message}"),
            input,
            Some(ctx),
        ))
    }

    /// Wraps an error from deserialising an already-validated value, which
    /// means the schema allowed something the Rust type doesn't.
    pub fn from_serde(error: &serde_json::Error, loc: Vec<LocItem>, input: Value) -> Self {
        Self::single(issue(
            "value_error",
            loc,
            &format!("Value error, {error}"),
            input,
            None,
        ))
    }
}

fn issue(
    kind: &str,
    loc: Vec<LocItem>,
    msg: &str,
    input: Value,
    ctx: Option<Map<String, Value>>,
) -> ValidationIssue {
    ValidationIssue {
        kind: kind.to_owned(),
        loc,
        msg: msg.to_owned(),
        input,
        ctx,
    }
}

/// A DataSchema prepared for validation (its `pattern`s compiled).
#[derive(Debug, Clone)]
pub struct SchemaValidator {
    schema: DataSchema,
    patterns: HashMap<String, Regex>,
}

impl SchemaValidator {
    /// Prepares `schema`. Fails if a `pattern` isn't a valid regular expression.
    pub fn new(schema: DataSchema) -> Result<Self, regex::Error> {
        let mut patterns = HashMap::new();
        collect_patterns(&schema, &mut patterns)?;
        Ok(Self { schema, patterns })
    }

    /// The schema.
    pub fn schema(&self) -> &DataSchema {
        &self.schema
    }

    /// Validates `value`, returning it with pydantic's coercions applied.
    /// Errors are located under `loc` (for example `["body"]`).
    pub fn validate(&self, value: &Value, loc: &[LocItem]) -> Result<Value, ValidationError> {
        let mut run = Run {
            patterns: &self.patterns,
            loc: loc.to_vec(),
            issues: Vec::new(),
        };
        match run.check(&self.schema, value) {
            Some(value) if run.issues.is_empty() => Ok(value),
            _ => Err(ValidationError { issues: run.issues }),
        }
    }
}

fn collect_patterns(
    schema: &DataSchema,
    out: &mut HashMap<String, Regex>,
) -> Result<(), regex::Error> {
    if let Some(pattern) = &schema.pattern
        && !out.contains_key(pattern)
    {
        out.insert(pattern.clone(), Regex::new(pattern)?);
    }
    for child in children(schema) {
        collect_patterns(child, out)?;
    }
    Ok(())
}

fn children(schema: &DataSchema) -> Vec<&DataSchema> {
    let mut out = Vec::new();
    if let Some(one_of) = &schema.one_of {
        out.extend(one_of);
    }
    match &schema.items {
        Some(ArrayItems::Single(item)) => out.push(item),
        Some(ArrayItems::Tuple(items)) => out.extend(items),
        None => {}
    }
    if let Some(properties) = &schema.properties {
        out.extend(properties.values());
    }
    out
}

/// The coercions pydantic applies in lax mode, measured against pydantic
/// 2. Each function returns the coerced value, or the error type and message.
pub mod coercion {
    use serde_json::{Number, Value};

    /// An error type and message.
    pub type Failure = (&'static str, &'static str);

    const INT_TYPE: Failure = ("int_type", "Input should be a valid integer");
    const INT_PARSING: Failure = (
        "int_parsing",
        "Input should be a valid integer, unable to parse string as an integer",
    );
    const INT_FROM_FLOAT: Failure = (
        "int_from_float",
        "Input should be a valid integer, got a number with a fractional part",
    );
    const FLOAT_TYPE: Failure = ("float_type", "Input should be a valid number");
    const FLOAT_PARSING: Failure = (
        "float_parsing",
        "Input should be a valid number, unable to parse string as a number",
    );
    const INT_PARSING_SIZE: Failure = (
        "int_parsing_size",
        "Unable to parse input string as an integer, exceeded maximum size",
    );
    const BOOL_TYPE: Failure = ("bool_type", "Input should be a valid boolean");
    const BOOL_PARSING: Failure = (
        "bool_parsing",
        "Input should be a valid boolean, unable to interpret input",
    );

    /// An integer: integers, whole floats, `true`/`false`, and strings of
    /// digits (surrounding whitespace, a sign, `_` between digits and a
    /// fraction of zeros such as `.00` allowed; `2.` and `1e3` are not).
    pub fn integer(value: &Value) -> Result<Value, Failure> {
        match value {
            Value::Number(n) if n.is_i64() || n.is_u64() => Ok(value.clone()),
            Value::Number(n) => float_to_int(n.as_f64().unwrap_or(f64::NAN)),
            Value::Bool(b) => Ok(Value::from(i64::from(*b))),
            Value::String(s) => parse_int(s).ok_or(INT_PARSING),
            _ => Err(INT_TYPE),
        }
    }

    fn float_to_int(f: f64) -> Result<Value, Failure> {
        if !f.is_finite() {
            Err(INT_PARSING)
        } else if f.fract() != 0.0 {
            Err(INT_FROM_FLOAT)
        } else if f.abs() < 9.223_372_036_854_775e18 {
            Ok(Value::from(f as i64))
        } else {
            Err(INT_PARSING_SIZE)
        }
    }

    fn parse_int(s: &str) -> Option<Value> {
        let t = s.trim();
        let (whole, fraction) = t.split_once('.').unwrap_or((t, "0"));
        if fraction.is_empty() || !fraction.chars().all(|c| c == '0') {
            return None;
        }
        let digits = whole.strip_prefix(['+', '-']).unwrap_or(whole);
        if !valid_digits(digits) {
            return None;
        }
        let cleaned: String = whole.chars().filter(|&c| c != '_').collect();
        cleaned
            .parse::<i64>()
            .map(Value::from)
            .or_else(|_| cleaned.parse::<u64>().map(Value::from))
            .ok()
    }

    /// Digits, possibly with single `_` separators between them.
    fn valid_digits(digits: &str) -> bool {
        !digits.is_empty()
            && digits.chars().all(|c| c.is_ascii_digit() || c == '_')
            && !digits.starts_with('_')
            && !digits.ends_with('_')
            && !digits.contains("__")
    }

    /// A number: integers and floats (always returned as a float, as
    /// pydantic's `float` does), `true`/`false`, and strings that parse as
    /// numbers (`_` between digits allowed).
    ///
    /// pydantic also accepts `"inf"` and `"nan"`, but JSON can't carry them,
    /// so here they fail with `float_parsing`.
    pub fn number(value: &Value) -> Result<Value, Failure> {
        let f = match value {
            Value::Number(n) => n.as_f64().ok_or(FLOAT_TYPE)?,
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::String(s) => parse_float(s).ok_or(FLOAT_PARSING)?,
            _ => return Err(FLOAT_TYPE),
        };
        Number::from_f64(f).map(Value::Number).ok_or(FLOAT_PARSING)
    }

    fn parse_float(s: &str) -> Option<f64> {
        let t = s.trim();
        let chars: Vec<char> = t.chars().collect();
        let separators_ok = chars.iter().enumerate().all(|(i, &c)| {
            c != '_'
                || (i > 0
                    && chars[i - 1].is_ascii_digit()
                    && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        });
        if !separators_ok {
            return None;
        }
        t.replace('_', "").parse::<f64>().ok()
    }

    /// A boolean: booleans, the integers and floats 0 and 1, and the
    /// strings pydantic accepts (case-insensitive `0`, `off`, `f`, `false`,
    /// `n`, `no`, `1`, `on`, `t`, `true`, `y`, `yes`). Other integers are
    /// `bool_parsing` errors, other floats `bool_type` errors.
    pub fn boolean(value: &Value) -> Result<Value, Failure> {
        match value {
            Value::Bool(_) => Ok(value.clone()),
            Value::Number(n) if n.is_i64() || n.is_u64() => match n.as_i64() {
                Some(0) => Ok(Value::Bool(false)),
                Some(1) => Ok(Value::Bool(true)),
                _ => Err(BOOL_PARSING),
            },
            Value::Number(n) => match n.as_f64() {
                Some(0.0) => Ok(Value::Bool(false)),
                Some(1.0) => Ok(Value::Bool(true)),
                _ => Err(BOOL_TYPE),
            },
            Value::String(s) => match s.to_ascii_lowercase().as_str() {
                "0" | "off" | "f" | "false" | "n" | "no" => Ok(Value::Bool(false)),
                "1" | "on" | "t" | "true" | "y" | "yes" => Ok(Value::Bool(true)),
                _ => Err(BOOL_PARSING),
            },
            _ => Err(BOOL_TYPE),
        }
    }

    /// A string: only strings (pydantic doesn't turn numbers into strings).
    pub fn string(value: &Value) -> Result<Value, Failure> {
        match value {
            Value::String(_) => Ok(value.clone()),
            _ => Err(("string_type", "Input should be a valid string")),
        }
    }
}

struct Run<'a> {
    patterns: &'a HashMap<String, Regex>,
    loc: Vec<LocItem>,
    issues: Vec<ValidationIssue>,
}

impl Run<'_> {
    fn fail(
        &mut self,
        kind: &str,
        msg: &str,
        input: &Value,
        ctx: Option<Map<String, Value>>,
    ) -> Option<Value> {
        self.issues
            .push(issue(kind, self.loc.clone(), msg, input.clone(), ctx));
        None
    }

    fn at<T>(&mut self, item: LocItem, f: impl FnOnce(&mut Self) -> T) -> T {
        self.loc.push(item);
        let out = f(self);
        self.loc.pop();
        out
    }

    /// Validates `value` against `schema`, recording errors and returning the
    /// coerced value if it is valid.
    fn check(&mut self, schema: &DataSchema, value: &Value) -> Option<Value> {
        if let Some(branches) = &schema.one_of {
            return self.union(branches, value);
        }
        if let Some(allowed) = &schema.enumeration {
            return self.literal(allowed, value);
        }
        if let Some(constant) = &schema.constant {
            return self.literal(std::slice::from_ref(constant), value);
        }
        match schema.data_type {
            None => Some(value.clone()),
            Some(DataType::Null) => match value {
                Value::Null => Some(Value::Null),
                _ => self.fail("none_required", "Input should be None", value, None),
            },
            Some(DataType::Boolean) => self.coerce(coercion::boolean, value),
            Some(DataType::Integer) => {
                let v = self.coerce(coercion::integer, value)?;
                self.numeric_bounds(schema, v, value)
            }
            Some(DataType::Number) => {
                let v = self.coerce(coercion::number, value)?;
                self.numeric_bounds(schema, v, value)
            }
            Some(DataType::String) => {
                let v = self.coerce(coercion::string, value)?;
                self.string_bounds(schema, v)
            }
            Some(DataType::Array) => self.array(schema, value),
            Some(DataType::Object) => self.object(schema, value),
        }
    }

    fn coerce(
        &mut self,
        f: fn(&Value) -> Result<Value, coercion::Failure>,
        value: &Value,
    ) -> Option<Value> {
        match f(value) {
            Ok(v) => Some(v),
            Err((kind, msg)) => self.fail(kind, msg, value, None),
        }
    }

    /// `oneOf`: an `Option` (one branch plus `null`) is validated against
    /// the non-null branch with no extra location, as pydantic does for a
    /// nullable field. Otherwise branches are tried first without coercion,
    /// then with; if none fits, every branch's errors are reported, each
    /// under the branch's tag.
    fn union(&mut self, branches: &[DataSchema], value: &Value) -> Option<Value> {
        let is_null = |b: &DataSchema| b.data_type == Some(DataType::Null) && b.one_of.is_none();
        if branches.len() == 2
            && let Some(other) = branches.iter().find(|b| !is_null(b))
            && branches.iter().any(is_null)
        {
            return if value.is_null() {
                Some(Value::Null)
            } else {
                self.check(other, value)
            };
        }
        if branches.iter().all(|b| b.constant.is_some()) {
            let allowed: Vec<Value> = branches.iter().filter_map(|b| b.constant.clone()).collect();
            return self.literal(&allowed, value);
        }

        let mut attempts = Vec::new();
        for strict in [true, false] {
            for (i, branch) in branches.iter().enumerate() {
                if strict && !exact_type(branch, value) {
                    continue;
                }
                let mut run = Run {
                    patterns: self.patterns,
                    loc: self.loc.clone(),
                    issues: Vec::new(),
                };
                run.loc.push(LocItem::Key(branch_tag(branch, i)));
                match run.check(branch, value) {
                    Some(v) if run.issues.is_empty() => return Some(v),
                    _ if !strict => attempts.extend(run.issues),
                    _ => {}
                }
            }
        }
        self.issues.extend(attempts);
        None
    }

    fn literal(&mut self, allowed: &[Value], value: &Value) -> Option<Value> {
        for candidate in allowed {
            if literal_matches(candidate, value) {
                return Some(candidate.clone());
            }
        }
        let expected = python_list(allowed);
        let mut ctx = Map::new();
        ctx.insert("expected".to_owned(), Value::String(expected.clone()));
        self.fail(
            "literal_error",
            &format!("Input should be {expected}"),
            value,
            Some(ctx),
        )
    }

    /// Numeric constraints on the coerced `value`. Like pydantic-core, only
    /// the first failure is reported, checked in the order `multiple_of`,
    /// `le`, `lt`, `ge`, `gt`, and the error shows the `original` input. For
    /// a `number` schema the bound in `ctx` is a float, as pydantic converts
    /// it to the field's type.
    fn numeric_bounds(
        &mut self,
        schema: &DataSchema,
        value: Value,
        original: &Value,
    ) -> Option<Value> {
        /// A bound, its error type, its `ctx` key, its wording, and the test it sets.
        type Check<'a> = (
            &'a Option<Number>,
            &'a str,
            &'a str,
            &'a str,
            fn(f64, f64) -> bool,
        );
        let x = value.as_f64()?;
        let as_float = schema.data_type == Some(DataType::Number);
        let ctx_number = |n: &Number| match n.as_f64().and_then(Number::from_f64) {
            Some(f) if as_float => f,
            _ => n.clone(),
        };
        if let Some(m) = &schema.multiple_of
            && let Some(step) = m.as_f64()
            && step > 0.0
            && !is_multiple(x, step)
        {
            let ctx = Map::from_iter([("multiple_of".to_owned(), Value::Number(ctx_number(m)))]);
            let msg = format!("Input should be a multiple of {}", display_number(m));
            return self.fail("multiple_of", &msg, original, Some(ctx));
        }
        let checks: [Check<'_>; 4] = [
            (
                &schema.maximum,
                "less_than_equal",
                "le",
                "less than or equal to",
                |x, b| x <= b,
            ),
            (
                &schema.exclusive_maximum,
                "less_than",
                "lt",
                "less than",
                |x, b| x < b,
            ),
            (
                &schema.minimum,
                "greater_than_equal",
                "ge",
                "greater than or equal to",
                |x, b| x >= b,
            ),
            (
                &schema.exclusive_minimum,
                "greater_than",
                "gt",
                "greater than",
                |x, b| x > b,
            ),
        ];
        for (bound, kind, key, words, ok) in checks {
            if let Some(bound) = bound
                && let Some(b) = bound.as_f64()
                && !ok(x, b)
            {
                let ctx = Map::from_iter([(key.to_owned(), Value::Number(ctx_number(bound)))]);
                let msg = format!("Input should be {words} {}", display_number(bound));
                return self.fail(kind, &msg, original, Some(ctx));
            }
        }
        Some(value)
    }

    /// String constraints: the first failure of `min_length`, `max_length`
    /// and `pattern`, in that order.
    fn string_bounds(&mut self, schema: &DataSchema, value: Value) -> Option<Value> {
        let s = value.as_str()?;
        let len = s.chars().count() as u64;
        if let Some(min) = schema.min_length
            && len < min
        {
            let ctx = Map::from_iter([("min_length".to_owned(), Value::from(min))]);
            let msg = format!("String should have at least {min} character{}", plural(min));
            return self.fail("string_too_short", &msg, &value, Some(ctx));
        }
        if let Some(max) = schema.max_length
            && len > max
        {
            let ctx = Map::from_iter([("max_length".to_owned(), Value::from(max))]);
            let msg = format!("String should have at most {max} character{}", plural(max));
            return self.fail("string_too_long", &msg, &value, Some(ctx));
        }
        if let Some(pattern) = &schema.pattern
            && let Some(regex) = self.patterns.get(pattern)
            && !regex.is_match(s)
        {
            let ctx = Map::from_iter([("pattern".to_owned(), Value::String(pattern.clone()))]);
            let msg = format!("String should match pattern '{pattern}'");
            return self.fail("string_pattern_mismatch", &msg, &value, Some(ctx));
        }
        Some(value)
    }

    /// Lists and tuples. As in pydantic, a list longer than its maximum
    /// reports only `too_long`; otherwise item errors come first, and
    /// `too_short` is reported only if every item is valid.
    fn array(&mut self, schema: &DataSchema, value: &Value) -> Option<Value> {
        let tuple = matches!(schema.items, Some(ArrayItems::Tuple(_)));
        let Value::Array(items) = value else {
            let (kind, msg) = if tuple {
                ("tuple_type", "Input should be a valid tuple")
            } else {
                ("list_type", "Input should be a valid list")
            };
            return self.fail(kind, msg, value, None);
        };
        let field_type = if tuple { "Tuple" } else { "List" };
        let max = match &schema.items {
            Some(ArrayItems::Tuple(positions)) => Some(positions.len() as u64),
            _ => schema.max_items,
        };
        if let Some(max) = max
            && items.len() as u64 > max
        {
            return self.length_error("too_long", field_type, max, items.len(), value);
        }

        let before = self.issues.len();
        let mut out = Vec::with_capacity(items.len());
        match &schema.items {
            Some(ArrayItems::Tuple(positions)) => {
                for (i, position) in positions.iter().enumerate() {
                    match items.get(i) {
                        Some(item) => out.push(self.at(i.into(), |r| r.check(position, item))),
                        None => {
                            self.at(i.into(), |r| {
                                r.fail("missing", "Field required", value, None)
                            });
                        }
                    }
                }
            }
            Some(ArrayItems::Single(item_schema)) => {
                for (i, item) in items.iter().enumerate() {
                    out.push(self.at(i.into(), |r| r.check(item_schema, item)));
                }
            }
            None => out.extend(items.iter().cloned().map(Some)),
        }
        if self.issues.len() > before {
            return None;
        }
        if !tuple
            && let Some(min) = schema.min_items
            && (items.len() as u64) < min
        {
            return self.length_error("too_short", field_type, min, items.len(), value);
        }
        Some(Value::Array(out.into_iter().flatten().collect()))
    }

    fn length_error(
        &mut self,
        kind: &str,
        field_type: &str,
        limit: u64,
        actual: usize,
        value: &Value,
    ) -> Option<Value> {
        let (key, words) = if kind == "too_short" {
            ("min_length", "at least")
        } else {
            ("max_length", "at most")
        };
        let msg = format!(
            "{field_type} should have {words} {limit} item{} after validation, not {actual}",
            plural(limit)
        );
        let ctx = Map::from_iter([
            ("field_type".to_owned(), Value::from(field_type)),
            (key.to_owned(), Value::from(limit)),
            ("actual_length".to_owned(), Value::from(actual)),
        ]);
        self.fail(kind, &msg, value, Some(ctx))
    }

    /// Objects: models (with `properties`) and maps. A model that isn't an
    /// object is `model_attributes_type`.
    fn object(&mut self, schema: &DataSchema, value: &Value) -> Option<Value> {
        let additional = schema.extra.get("additionalProperties");
        let Value::Object(members) = value else {
            return if schema.properties.is_some() {
                self.fail(
                    "model_attributes_type",
                    "Input should be a valid dictionary or object to extract fields from",
                    value,
                    None,
                )
            } else {
                self.fail(
                    "dict_type",
                    "Input should be a valid dictionary",
                    value,
                    None,
                )
            };
        };
        let before = self.issues.len();
        let mut out = Map::new();
        let properties = schema.properties.as_ref();
        if let Some(properties) = properties {
            let required = schema.required.as_deref().unwrap_or_default();
            for (name, property) in properties {
                match members.get(name) {
                    Some(member) => {
                        if let Some(v) =
                            self.at(name.as_str().into(), |r| r.check(property, member))
                        {
                            out.insert(name.clone(), v);
                        }
                    }
                    None if required.contains(name) => {
                        self.at(name.as_str().into(), |r| {
                            r.fail("missing", "Field required", value, None)
                        });
                    }
                    None => {}
                }
            }
        }
        for (name, member) in members {
            if properties.is_some_and(|p| p.contains_key(name)) {
                continue;
            }
            match additional {
                Some(Value::Bool(false)) => {
                    self.at(name.as_str().into(), |r| {
                        r.fail(
                            "extra_forbidden",
                            "Extra inputs are not permitted",
                            member,
                            None,
                        )
                    });
                }
                Some(Value::Object(_)) => {
                    let member_schema: Option<DataSchema> = additional
                        .cloned()
                        .and_then(|a| serde_json::from_value(a).ok());
                    if let Some(member_schema) = member_schema
                        && let Some(v) =
                            self.at(name.as_str().into(), |r| r.check(&member_schema, member))
                    {
                        out.insert(name.clone(), v);
                    }
                }
                // `true`, or no `additionalProperties` on a map-like schema: keep.
                Some(_) => {
                    out.insert(name.clone(), member.clone());
                }
                // A model that ignores unknown members, as pydantic does by default.
                None if properties.is_some() => {}
                None => {
                    out.insert(name.clone(), member.clone());
                }
            }
        }
        (self.issues.len() == before).then_some(Value::Object(out))
    }
}

/// Whether `value` already has `schema`'s JSON type (no coercion needed).
fn exact_type(schema: &DataSchema, value: &Value) -> bool {
    match (schema.data_type, value) {
        (None, _) => true,
        (Some(DataType::Null), Value::Null)
        | (Some(DataType::Boolean), Value::Bool(_))
        | (Some(DataType::String), Value::String(_))
        | (Some(DataType::Array), Value::Array(_))
        | (Some(DataType::Object), Value::Object(_)) => true,
        (Some(DataType::Integer), Value::Number(n)) => n.is_i64() || n.is_u64(),
        (Some(DataType::Number), Value::Number(_)) => true,
        _ => false,
    }
}

/// The tag pydantic puts in the location of a union branch's errors.
fn branch_tag(schema: &DataSchema, index: usize) -> String {
    match schema.data_type {
        Some(DataType::Integer) => "int".to_owned(),
        Some(DataType::Number) => "float".to_owned(),
        Some(DataType::String) => "str".to_owned(),
        Some(DataType::Boolean) => "bool".to_owned(),
        Some(DataType::Null) => "none".to_owned(),
        Some(DataType::Array) => "list".to_owned(),
        Some(DataType::Object) => schema.title.clone().unwrap_or_else(|| "dict".to_owned()),
        None => format!("branch-{index}"),
    }
}

/// Whether `value` selects the literal `candidate`, by Python's equality:
/// numbers compare by value whatever their kind, and `true`/`false` equal
/// `1`/`0`. A string literal is only matched by the same string.
fn literal_matches(candidate: &Value, value: &Value) -> bool {
    let numeric = |v: &Value| match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    };
    match (numeric(candidate), numeric(value)) {
        (Some(a), Some(b)) => a == b,
        _ => candidate == value,
    }
}

/// Values as pydantic lists expected literals: `'a', 'b' or 'c'`.
fn python_list(values: &[Value]) -> String {
    let reprs: Vec<String> = values.iter().map(python_repr).collect();
    match reprs.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
    }
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::String(s) => format!("'{s}'"),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Null => "None".to_owned(),
        Value::Number(n) => display_number(n),
        other => other.to_string(),
    }
}

/// Formats a bound as pydantic-core does (Rust's `Display`: `1`, `0.5`).
fn display_number(n: &Number) -> String {
    if let Some(i) = n.as_i64() {
        i.to_string()
    } else if let Some(u) = n.as_u64() {
        u.to_string()
    } else {
        n.as_f64().map(|f| f.to_string()).unwrap_or_default()
    }
}

fn is_multiple(x: f64, step: f64) -> bool {
    let ratio = x / step;
    (ratio - ratio.round()).abs() < 1e-9
}

fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}
