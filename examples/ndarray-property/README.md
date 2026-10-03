# ndarray-property

**Concept:** arrays as properties and as actions' inputs and outputs. `NdArray<A, D>` wraps an `ndarray` array, and is carried as nested lists.

**Technologies:** the `ndarray` feature; `NdArray<f64, Ix2>` (a fixed dimension) and `NdArray` (any dimension); `teta_wot::ndarray`.

## Run it

```
cargo run -p ndarray-property
cargo run -p ndarray-property -- -serve
```

It prints the TD's description of the arrays and the answers to a few requests, and exits with a non-zero status if one is wrong. `--serve` serves on port 5000 until Ctrl-C, for `curl`:

```
curl http://127.0.0.1:5000/camera/dark_frame
```
