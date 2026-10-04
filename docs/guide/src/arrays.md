# Arrays

`teta-wot` has `NdArray`, with the `ndarray` feature: an [`ndarray`](https://docs.rs/ndarray) array, written on the wire as nested lists.

```rust,ignore
use teta_wot::ndarray::{Ix1, Ix2, array};
use teta_wot::prelude::*;

/// A small camera, with arrays for images.
#[derive(Thing)]
pub struct Camera {
    /// The dark frame: what the sensor reads with the shutter closed.
    #[property(default = NdArray(array![[1.0, 2.0, 1.0], [2.0, 3.0, 2.0]]))]
    dark_frame: Prop<NdArray<f64, Ix2>>,
}

#[thing_impl]
impl Camera {
    /// Subtract the dark frame from an image of the same shape.
    #[action]
    async fn correct(&self, image: NdArray<f64, Ix2>) -> NdArray<f64, Ix2> {
        NdArray(image.0 - &self.dark_frame.get().0)
    }
}
```

`NdArray<A, D>` wraps an `ndarray::Array<A, D>`, and dereferences to it. The `ndarray` crate is re-exported as `teta_wot::ndarray`.

## On the wire

- A matrix is `[[1.0, 2.0, 1.0], [2.0, 3.0, 2.0]]`: one level of lists per dimension.
- A zero-dimensional array is its one element.
- An `f64` array reads back as floats (`1.0`), whatever the client sent. numpy keeps the type the client sent.

## In the TD

- **A fixed dimension (`Ix1`, `Ix2`, …)** is described exactly. `NdArray<f64, Ix2>` is an array of arrays of numbers, and `NdArray<i64, Ix2>` of integers.
- **Any dimension (`NdArray`, which is `NdArray<f64, IxDyn>`)** is described as a number, or lists of numbers up to seven deep.

## Validation

- **Arrays must be rectangular.** Ragged lists get numpy's message, as LabThings sends it: "Value error, setting an array element with a sequence. The requested array has an inhomogeneous shape after 1 dimensions. The detected shape was (2,) + inhomogeneous part."
- **Elements must match the element type.** Strings, booleans and `null`s are refused. LabThings hands them to numpy, and they fail later.
- **A fixed dimension** refuses lists of another depth. An empty list is an empty array of that dimension.

Nested lists are a slow way to move large arrays, as LabThings warns too: use a [Blob](blobs.md) for images and large data. See the [`ndarray-property`](https://github.com/lucaswewa/teta-wot/tree/main/examples/ndarray-property) example, and the autofocus's focus curve in the simulated microscope.
