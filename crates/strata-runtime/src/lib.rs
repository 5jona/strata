//! Single-threaded container isolation (`stage2`, `enter`), storage lifecycle, and atomic sync.

pub mod enter;
pub mod error;
pub mod fs_atomic;
pub mod spec_pipe;
pub mod stage2;
pub mod storage;
pub mod sync;

pub use enter::{EnterSpec, run_enter, run_enter_from_fd};
pub use error::{RuntimeError, RuntimeResult};
pub use stage2::{Stage2Spec, run_stage2, run_stage2_from_fd};
