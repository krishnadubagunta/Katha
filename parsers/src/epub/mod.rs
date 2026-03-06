//! EPUB parser adapter.
//!
//! Exposes [`Epub`], an implementation of the crate-level [`Parser`](crate::Parser)
//! trait that reads EPUB metadata, TOC, and spine content, then normalizes it into
//! a [`Document`](crate::Document).

mod parse;

/// EPUB parser implementation.
pub use parse::Epub;
