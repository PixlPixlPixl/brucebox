//! Brucebox host foundation.
//!
//! The library is the host: GPUI application shell, the Cordis composition
//! runtime, the workspace engine, and the panel catalog that contributions
//! register with. Plugin implementations are not compiled into this crate —
//! they arrive through the catalog, as declarations the runtime carries.

pub mod app;
pub mod runtime;
pub mod workspace;
