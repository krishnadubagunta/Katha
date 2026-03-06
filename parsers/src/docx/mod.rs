//! DOCX parser adapter.
//!
//! Exposes [`Docx`], an implementation of the crate-level [`Parser`](crate::Parser)
//! trait that reads OOXML document content and maps it into the normalized
//! [`Document`](crate::Document) model.

mod parse;

/// DOCX parser implementation.
pub use parse::Docx;
