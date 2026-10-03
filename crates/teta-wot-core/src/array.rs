//! `ndarray` arrays on the wire.
//!
//! [`NdArray<A, D>`] wraps an [`ndarray::Array`] and is written as nested
//! lists of its elements, one level per dimension: a matrix is
//! `[[1.0, 2.0], [3.0, 4.0]]`, and a zero-dimensional array is its one
//! element. It is read back from the same form.
//!
//! **Schemas.** With a dynamic dimension (`IxDyn`, the default), it is exactly
//! that many levels of arrays of the element's schema.
//!
//! **Validation.** Nested lists whose lengths differ at some depth, or that
//! mix numbers and lists there, are refused with numpy's message, as
//! pydantic reports it: "Value error, setting an array element with a
//! sequence. The requested array has an inhomogeneous shape after 1
//! dimensions. The detected shape was (2,) + inhomogeneous part." Elements
//! must match the element type's schema.

use std::borrow::Cow;
use std::fmt;
use std::ops::{Deref, DerefMut};

use ndarray::{Array, ArrayViewD, Axis, Dimension, IxDyn};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, DeserializeOwned, Deserializer};
use serde::ser::{SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::validate::value_error_de;

/// The description for the `NDArray` schema.
const TETATHING_DESCRIPTION: &str = "A RootModel describing a list-of-lists up to 7 deep.\n\nThis is used to generate a JSONSchema description of a `numpy.ndarray`\nserialised to a list. It is used in the annotated `.NDArray` type.";

/// An n-dimensional array, carried as nested lists (see the [module
/// documentation](self)). It dereferences to the [`ndarray::Array`].
///
/// ```
/// use teta_wot_core::array::NdArray;
///
/// let matrix: NdArray = ndarray::arr2(&[[1.0, 2.0], [3.0, 4.0]]).into_dyn().into();
/// assert_eq!(serde_json::to_string(&matrix).unwrap(), "[[1.0,2.0],[3.0,4.0]]");
/// let back: NdArray = serde_json::from_str("[[1, 2], [3, 4]]").unwrap();
/// assert_eq!(back, matrix);
/// ```
#[derive(Clone, PartialEq)]
pub struct NdArray<A = f64, D: Dimension = IxDyn>(pub Array<A, D>);

impl<A, D: Dimension> NdArray<A, D> {
    /// The array.
    pub fn into_inner(self) -> Array<A, D> {
        self.0
    }
}

impl<A: fmt::Debug, D: Dimension> fmt::Debug for NdArray<A, D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

/// An empty array: no elements, with the dimension's number of axes (one
/// for `IxDyn`).
impl<A, D: Dimension> Default for NdArray<A, D> {
    fn default() -> Self {
        let dim = D::zeros(D::NDIM.unwrap_or(1));
        Self(Array::from_shape_vec(dim, Vec::new()).expect("an empty shape has no elements"))
    }
}

impl<A, D: Dimension> Deref for NdArray<A, D> {
    type Target = Array<A, D>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<A, D: Dimension> DerefMut for NdArray<A, D> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<A, D: Dimension> From<Array<A, D>> for NdArray<A, D> {
    fn from(array: Array<A, D>) -> Self {
        Self(array)
    }
}

impl<A, D: Dimension> From<NdArray<A, D>> for Array<A, D> {
    fn from(array: NdArray<A, D>) -> Self {
        array.0
    }
}

// ---- Serialisation --------------------------------------------------------

/// A view, written as nested lists.
struct Nested<'a, A>(ArrayViewD<'a, A>);

impl<A: Serialize> Serialize for Nested<'_, A> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.0.ndim() == 0 {
            return match self.0.first() {
                Some(element) => element.serialize(serializer),
                None => serializer.serialize_none(),
            };
        }
        let length = self.0.len_of(Axis(0));
        let mut seq = serializer.serialize_seq(Some(length))?;
        for index in 0..length {
            seq.serialize_element(&Nested(self.0.index_axis(Axis(0), index)))?;
        }
        seq.end()
    }
}

impl<A: Serialize, D: Dimension> Serialize for NdArray<A, D> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Nested(self.0.view().into_dyn()).serialize(serializer)
    }
}

impl<'de, A: DeserializeOwned, D: Dimension> Deserialize<'de> for NdArray<A, D> {
    fn deserialize<De: Deserializer<'de>>(deserializer: De) -> Result<Self, De::Error> {
        let value = Value::deserialize(deserializer)?;
        let mut shape = shape(&value).map_err(value_error_de)?;
        // An empty list can't show its inner dimensions: `[]` is a 0 x 0
        // matrix for a fixed dimension of 2.
        if let Some(ndim) = D::NDIM
            && shape.len() < ndim
            && shape.last() == Some(&0)
        {
            shape.resize(ndim, 0);
        }
        let mut elements = Vec::with_capacity(shape.iter().product());
        flatten(value, &mut elements);
        let elements = elements
            .into_iter()
            .map(|element| serde_json::from_value(element).map_err(de::Error::custom))
            .collect::<Result<Vec<A>, De::Error>>()?;
        let array = Array::from_shape_vec(IxDyn(&shape), elements).map_err(de::Error::custom)?;
        let ndim = array.ndim();
        array.into_dimensionality::<D>().map(Self).map_err(|_| {
            value_error_de(format!(
                "expected an array of {} dimensions, got {ndim}",
                D::NDIM.unwrap_or(ndim)
            ))
        })
    }
}

/// The shape of nested lists, or numpy's complaint when they aren't a
/// rectangular array: at each depth, every item must be a list of one
/// length, or none may be a list.
fn shape(value: &Value) -> Result<Vec<usize>, String> {
    let mut shape = Vec::new();
    let mut level = vec![value];
    loop {
        let lists: Vec<&Vec<Value>> = level.iter().filter_map(|v| v.as_array()).collect();
        if lists.is_empty() {
            return Ok(shape);
        }
        let length = lists[0].len();
        if lists.len() != level.len() || lists.iter().any(|l| l.len() != length) {
            let detected = match &shape[..] {
                [single] => format!("({single},)"),
                dims => format!(
                    "({})",
                    dims.iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            return Err(format!(
                "setting an array element with a sequence. The requested array has an inhomogeneous shape after {} dimensions. The detected shape was {detected} + inhomogeneous part.",
                shape.len()
            ));
        }
        shape.push(length);
        level = lists.into_iter().flatten().collect();
    }
}

/// The elements of rectangular nested lists, in row-major order.
fn flatten(value: Value, out: &mut Vec<Value>) {
    match value {
        Value::Array(items) => items.into_iter().for_each(|item| flatten(item, out)),
        other => out.push(other),
    }
}

// ---- Schema ----------------------------------------------------------------

impl<A: JsonSchema, D: Dimension> JsonSchema for NdArray<A, D> {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        match D::NDIM {
            None => "NestedListOfNumbersModel".into(),
            Some(n) => format!("NdArray{n}_{}", A::schema_name()).into(),
        }
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        match D::NDIM {
            None => tetathing_schema(),
            Some(n) => {
                let mut schema = generator.subschema_for::<A>().to_value();
                for _ in 0..n {
                    schema = json!({"type": "array", "items": schema});
                }
                Schema::try_from(schema).unwrap_or_else(|_| json_schema!({}))
            }
        }
    }
}

/// A number, or lists of numbers up to six deep, or
/// seven levels of lists of anything.
fn tetathing_schema() -> Schema {
    let number = json!({"anyOf": [{"type": "integer"}, {"type": "number"}]});
    let mut branches = vec![json!({"type": "integer"}), json!({"type": "number"})];
    let mut nested = number;
    for _ in 0..6 {
        nested = json!({"items": nested, "type": "array"});
        branches.push(nested.clone());
    }
    let mut anything = json!({});
    for _ in 0..7 {
        anything = json!({"items": anything, "type": "array"});
    }
    branches.push(anything);
    Schema::try_from(json!({
        "anyOf": branches,
        "description": TETATHING_DESCRIPTION,
        "title": "NestedListOfNumbersModel",
    }))
    .unwrap_or_else(|_| json_schema!({}))
}

#[cfg(test)]
mod tests {
    use ndarray::{Ix0, Ix1, Ix2, arr2};

    use super::*;

    #[test]
    fn arrays_are_nested_lists() {
        let matrix: NdArray<i64, Ix2> = arr2(&[[1, 2, 3], [4, 5, 6]]).into();
        assert_eq!(
            serde_json::to_value(&matrix).unwrap(),
            json!([[1, 2, 3], [4, 5, 6]])
        );
        let scalar: NdArray<f64, Ix0> = Array::from_elem((), 2.5).into();
        assert_eq!(serde_json::to_value(&scalar).unwrap(), json!(2.5));
        let empty: NdArray = NdArray::default();
        assert_eq!(serde_json::to_value(&empty).unwrap(), json!([]));
        assert_eq!(empty.shape(), [0]);
        assert_eq!(NdArray::<f64, Ix2>::default().shape(), [0, 0]);

        let back: NdArray<i64, Ix2> =
            serde_json::from_value(json!([[1, 2, 3], [4, 5, 6]])).unwrap();
        assert_eq!(back, matrix);
        let dynamic: NdArray = serde_json::from_value(json!([[[1.5]]])).unwrap();
        assert_eq!(dynamic.shape(), [1, 1, 1]);
        let zero: NdArray = serde_json::from_value(json!(5)).unwrap();
        assert_eq!((zero.ndim(), zero.first()), (0, Some(&5.0)));
        let empty: NdArray = serde_json::from_value(json!([[], []])).unwrap();
        assert_eq!(empty.shape(), [2, 0]);
    }

    #[test]
    fn ragged_lists_get_numpys_message() {
        for (value, after, detected) in [
            (json!([[1, 2], [3]]), 1, "(2,)"),
            (json!([1, [2]]), 1, "(2,)"),
            (json!([[[1, 2], [3]], [[4, 5], [6, 7]]]), 2, "(2, 2)"),
            (json!([[1, 2], [3, [4]]]), 2, "(2, 2)"),
            (json!([[1], [[2]]]), 2, "(2, 1)"),
        ] {
            let error = shape(&value).unwrap_err();
            assert_eq!(
                error,
                format!(
                    "setting an array element with a sequence. The requested array has an inhomogeneous shape after {after} dimensions. The detected shape was {detected} + inhomogeneous part."
                ),
                "{value}"
            );
            assert!(serde_json::from_value::<NdArray>(value).is_err());
        }
    }

    #[test]
    fn a_fixed_dimension_is_checked() {
        let error = serde_json::from_value::<NdArray<f64, Ix1>>(json!([[1.0]])).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("expected an array of 1 dimensions, got 2"),
            "{error}"
        );
    }

    #[test]
    fn schemas() {
        let dynamic = schemars::schema_for!(NdArray).to_value();
        assert_eq!(dynamic["title"], "NestedListOfNumbersModel");
        assert_eq!(dynamic["anyOf"].as_array().unwrap().len(), 9);
        assert_eq!(
            dynamic["anyOf"][8]["items"]["items"]["items"]["items"]["items"]["items"]["items"],
            json!({})
        );
        let fixed = schemars::schema_for!(NdArray<f64, Ix2>).to_value();
        assert_eq!(fixed["type"], "array");
        assert_eq!(fixed["items"]["items"]["type"], "number");
    }
}
