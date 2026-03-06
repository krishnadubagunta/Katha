//! PDF parser adapter.
//!
//! Exposes [`Pdf`], an implementation of the crate-level [`Parser`](crate::Parser)
//! trait that extracts text from PDFs and maps it into the normalized
//! [`Document`](crate::Document) model.

mod parse;

/// PDF parser implementation.
pub use parse::Pdf;
