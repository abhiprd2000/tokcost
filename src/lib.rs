//! tokcost's library crate: everything except argument parsing and process
//! wiring, which stay in `main.rs`. Split out so `tests/golden.rs` (and any
//! future integration test) can reach `bpe`/`vocab` directly instead of
//! only being able to exercise the CLI as a black box.

pub mod bpe;
pub mod meter;
pub mod pricing;
pub mod render;
pub mod vocab;
