//! A value that the TD allows to be written either on its own or as an array.

use serde::{Deserialize, Serialize};

/// A TD term that may hold one value or an array of values.
///
/// The two forms are kept apart so that a TD is written back exactly as it
/// was built or parsed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    /// A single value
    One(T),
    /// Any number of values, as an array.
    Many(Vec<T>),
}

impl<T> OneOrMany<T> {
    /// Iterators over the values
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        match self {
            OneOrMany::One(value) => std::slice::from_ref(value).iter(),
            OneOrMany::Many(values) => values.iter(),
        }
    }

    /// The number of values.
    pub fn len(&self) -> usize {
        match self {
            OneOrMany::One(_) => 1,
            OneOrMany::Many(values) => values.len(),
        }
    }

    /// Whether there are no values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether `value` is one of the values
    pub fn contains(&self, value: &T) -> bool
    where
        T: PartialEq,
    {
        self.iter().any(|v| v == value)
    }
}

impl<T> From<T> for OneOrMany<T> {
    fn from(value: T) -> Self {
        OneOrMany::One(value)
    }
}

impl<T> From<Vec<T>> for OneOrMany<T> {
    fn from(values: Vec<T>) -> Self {
        OneOrMany::Many(values)
    }
}

impl<T, const N: usize> From<[T; N]> for OneOrMany<T> {
    fn from(values: [T; N]) -> Self {
        OneOrMany::Many(values.into())
    }
}

impl From<&str> for OneOrMany<String> {
    fn from(value: &str) -> Self {
        OneOrMany::One(value.to_owned())
    }
}
