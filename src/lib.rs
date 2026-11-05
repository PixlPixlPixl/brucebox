//! Brucebox host foundation.
//!
//! The library is the host: GPUI application shell, workspace engine, and the
//! panel catalog that contributions register with. Plugin implementations are
//! not compiled into this crate — they arrive through the catalog.

pub mod app;
pub mod workspace;
