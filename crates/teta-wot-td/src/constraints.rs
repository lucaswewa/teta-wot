//! Value constraints on properties, and how they appear in a DataSchema.

use serde_json::{Number, Value};

use crate::data_schema::{DataSchema, DataType};

/// A numeric bound: any integer, or a finite float.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Bound {
    /// A signed integer.
    Int(i64),
    /// An unsigned integer too large for `i64`.
    UInt(u64),
    /// A float.
    Float(f64),
}

macro_rules! bound_from {
    ($variant:ident as $target:ty: $($t:ty),*) => {$(
        impl From<$t> for Bound {
            fn from(value: $t) -> Self {
                Bound::$variant(value as $target)
            }
        }
    )*};
}
bound_from!(Int as i64: i8, i16, i32, i64, u8, u16, u32);
bound_from!(Float as f64: f32, f64);

impl From<u64> for Bound {
    fn from(value: u64) -> Self {
        i64::try_from(value).map_or(Bound::UInt(value), Bound::Int)
    }
}

impl Bound {
    fn to_number(self, name: &'static str) -> Result<Number, ConstraintError> {
        match self {
            Bound::Int(i) => Ok(i.into()),
            Bound::UInt(u) => Ok(u.into()),
            Bound::Float(f) => {
                Number::from_f64(f).ok_or(ConstraintError::NotFinite { constraint: name })
            }
        }
    }
}

/// The constraints on a property.
///
/// [`apply`](Self::apply) writes them into a DataSchema the way pydantic
/// does: `ge` → `minimum`, `gt` → `exclusiveMinimum`, `le` → `maximum`,
/// `lt` → `exclusiveMaximum`, `multiple_of` → `multipleOf`, `pattern` →
/// `pattern`, and `min_length`/`max_length` → `minLength`/`maxLength` for
/// strings, `minItems`/`maxItems` for arrays, or
/// `minProperties`/`maxProperties` for objects. `allow_inf_nan` only
/// affects validation, so it adds nothing to the schema.
///
/// ```
/// # use teta_wot_td::{Constraints, DataSchema, DataType};
/// let mut schema = DataSchema::of(DataType::Integer);
/// Constraints::new().ge(1).le(10).apply(&mut schema).unwrap();
/// assert_eq!(schema.minimum, Some(1.into()));
/// assert_eq!(schema.maximum, Some(10.into()));
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
#[must_use]
pub struct Constraints {
    /// Greater than.
    pub gt: Option<Bound>,
    /// Greater than or equal to.
    pub ge: Option<Bound>,
    /// Less than.
    pub lt: Option<Bound>,
    /// Less than or equal to.
    pub le: Option<Bound>,
    /// A multiple of this (must be positive).
    pub multiple_of: Option<Bound>,
    /// Whether infinities and NaN are allowed (numbers only).
    pub allow_inf_nan: Option<bool>,
    /// Minimum length of a string, array or object.
    pub min_length: Option<u64>,
    /// Maximum length of a string, array or object.
    pub max_length: Option<u64>,
    /// A regular expression that a string must match.
    pub pattern: Option<String>,
}

impl Constraints {
    /// No constraints.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether there are no constraints.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Sets `gt`.
    pub fn gt(mut self, value: impl Into<Bound>) -> Self {
        self.gt = Some(value.into());
        self
    }

    /// Sets `ge`.
    pub fn ge(mut self, value: impl Into<Bound>) -> Self {
        self.ge = Some(value.into());
        self
    }

    /// Sets `lt`.
    pub fn lt(mut self, value: impl Into<Bound>) -> Self {
        self.lt = Some(value.into());
        self
    }

    /// Sets `le`.
    pub fn le(mut self, value: impl Into<Bound>) -> Self {
        self.le = Some(value.into());
        self
    }

    /// Sets `multiple_of`.
    pub fn multiple_of(mut self, value: impl Into<Bound>) -> Self {
        self.multiple_of = Some(value.into());
        self
    }

    /// Sets `allow_inf_nan`.
    pub fn allow_inf_nan(mut self, allow: bool) -> Self {
        self.allow_inf_nan = Some(allow);
        self
    }

    /// Sets `min_length`.
    pub fn min_length(mut self, length: u64) -> Self {
        self.min_length = Some(length);
        self
    }

    /// Sets `max_length`.
    pub fn max_length(mut self, length: u64) -> Self {
        self.max_length = Some(length);
        self
    }

    /// Sets `pattern`.
    pub fn pattern(mut self, pattern: impl Into<String>) -> Self {
        self.pattern = Some(pattern.into());
        self
    }

    /// Writes the constraints into `schema`.
    ///
    /// If `schema` has a `type`, every constraint must suit it. If it has no
    /// `type` but a `oneOf` (for example an `Option<i64>`), each constraint
    /// goes into every branch it suits, as pydantic does for `Optional`
    /// fields, and must suit at least one. Otherwise the schema can't be
    /// constrained.
    pub fn apply(&self, schema: &mut DataSchema) -> Result<(), ConstraintError> {
        if self.is_empty() {
            return Ok(());
        }
        if let Some(Bound::Int(m)) = self.multiple_of
            && m <= 0
        {
            return Err(ConstraintError::NotPositive);
        }
        if let Some(Bound::Float(m)) = self.multiple_of
            && m <= 0.0
        {
            return Err(ConstraintError::NotPositive);
        }

        if schema.data_type.is_some() {
            for name in self.set() {
                if !suits(name, schema.data_type) {
                    return Err(ConstraintError::Unsuitable {
                        constraint: name,
                        data_type: schema.data_type,
                    });
                }
            }
            self.check_numeric_bounds()?;
            return self.write(schema);
        }

        let Some(branches) = schema.one_of.as_mut() else {
            return Err(ConstraintError::Untyped);
        };
        for name in self.set() {
            if !branches.iter().any(|b| suits(name, b.data_type)) {
                return Err(ConstraintError::Unsuitable {
                    constraint: name,
                    data_type: None,
                });
            }
        }
        self.check_numeric_bounds()?;
        for branch in branches.iter_mut() {
            let only_suitable = Constraints {
                gt: self.gt.filter(|_| suits("gt", branch.data_type)),
                ge: self.ge.filter(|_| suits("ge", branch.data_type)),
                lt: self.lt.filter(|_| suits("lt", branch.data_type)),
                le: self.le.filter(|_| suits("le", branch.data_type)),
                multiple_of: self
                    .multiple_of
                    .filter(|_| suits("multiple_of", branch.data_type)),
                allow_inf_nan: self
                    .allow_inf_nan
                    .filter(|_| suits("allow_inf_nan", branch.data_type)),
                min_length: self
                    .min_length
                    .filter(|_| suits("min_length", branch.data_type)),
                max_length: self
                    .max_length
                    .filter(|_| suits("max_length", branch.data_type)),
                pattern: self
                    .pattern
                    .clone()
                    .filter(|_| suits("pattern", branch.data_type)),
            };
            only_suitable.write(branch)?;
        }
        Ok(())
    }

    /// Checks every numeric bound before any part of the schema is changed.
    fn check_numeric_bounds(&self) -> Result<(), ConstraintError> {
        for (name, bound) in [
            ("gt", self.gt),
            ("ge", self.ge),
            ("lt", self.lt),
            ("le", self.le),
            ("multiple_of", self.multiple_of),
        ] {
            if let Some(bound) = bound {
                bound.to_number(name)?;
            }
        }
        Ok(())
    }

    /// The names of the constraints that are set.
    fn set(&self) -> impl Iterator<Item = &'static str> {
        [
            ("gt", self.gt.is_some()),
            ("ge", self.ge.is_some()),
            ("lt", self.lt.is_some()),
            ("le", self.le.is_some()),
            ("multiple_of", self.multiple_of.is_some()),
            ("allow_inf_nan", self.allow_inf_nan.is_some()),
            ("min_length", self.min_length.is_some()),
            ("max_length", self.max_length.is_some()),
            ("pattern", self.pattern.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, set)| set.then_some(name))
    }

    /// Writes the constraints into a schema they are known to suit.
    fn write(&self, schema: &mut DataSchema) -> Result<(), ConstraintError> {
        let number = |bound: Option<Bound>, name| bound.map(|b| b.to_number(name)).transpose();
        if let Some(n) = number(self.gt, "gt")? {
            schema.exclusive_minimum = Some(n);
        }
        if let Some(n) = number(self.ge, "ge")? {
            schema.minimum = Some(n);
        }
        if let Some(n) = number(self.lt, "lt")? {
            schema.exclusive_maximum = Some(n);
        }
        if let Some(n) = number(self.le, "le")? {
            schema.maximum = Some(n);
        }
        if let Some(n) = number(self.multiple_of, "multiple_of")? {
            schema.multiple_of = Some(n);
        }
        if let Some(pattern) = &self.pattern {
            schema.pattern = Some(pattern.clone());
        }
        let lengths = [(self.min_length, true), (self.max_length, false)];
        for (length, is_min) in lengths {
            let Some(length) = length else { continue };
            match (schema.data_type, is_min) {
                (Some(DataType::String), true) => schema.min_length = Some(length),
                (Some(DataType::String), false) => schema.max_length = Some(length),
                (Some(DataType::Array), true) => schema.min_items = Some(length),
                (Some(DataType::Array), false) => schema.max_items = Some(length),
                (_, true) => {
                    schema
                        .extra
                        .insert("minProperties".to_owned(), Value::from(length));
                }
                (_, false) => {
                    schema
                        .extra
                        .insert("maxProperties".to_owned(), Value::from(length));
                }
            }
        }
        Ok(())
    }
}

/// Whether the constraint `name` suits values of `data_type`.
fn suits(name: &str, data_type: Option<DataType>) -> bool {
    use DataType::*;
    match name {
        "gt" | "ge" | "lt" | "le" | "multiple_of" => matches!(data_type, Some(Integer | Number)),
        "allow_inf_nan" => data_type == Some(Number),
        "min_length" | "max_length" => matches!(data_type, Some(String | Array | Object)),
        "pattern" => data_type == Some(String),
        _ => false,
    }
}

/// Constraints can't be applied to a schema.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ConstraintError {
    /// A constraint doesn't suit the schema's type, for example `pattern` on
    /// an integer or `allow_inf_nan` on an integer.
    #[error("constraint `{constraint}` doesn't suit {}", data_type.map_or("any of the schema's types", |t| t.as_str()))]
    Unsuitable {
        /// The constraint.
        constraint: &'static str,
        /// The schema's type (`None` for a `oneOf` schema).
        data_type: Option<DataType>,
    },
    /// The schema has neither a `type` nor a `oneOf`.
    #[error("constraints need a schema with a `type` or a `oneOf`")]
    Untyped,
    /// A float bound is infinite or NaN.
    #[error("constraint `{constraint}` must be finite")]
    NotFinite {
        /// The constraint.
        constraint: &'static str,
    },
    /// `multiple_of` is zero or negative.
    #[error("`multiple_of` must be greater than zero")]
    NotPositive,
}
