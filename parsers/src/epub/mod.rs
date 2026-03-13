//! EPUB parser adapter.
//!
//! Exposes [`Epub`], an implementation of the crate-level [`Parser`](crate::Parser)
//! trait that reads EPUB metadata, TOC, and spine content, then normalizes it into
//! a [`Document`](crate::Document).
//!
//! # Examples
//!
//! ```no_run
//! use katha_parsers::{Parser, epub::Epub};
//!
//! let mut parser = Epub::new();
//! let document = parser.parse("book.epub")?;
//! assert!(!document.content.is_empty());
//! # Ok::<(), katha_parsers::error::ParserError>(())
//! ```

mod parse;

/// EPUB parser implementation.
pub use parse::Epub;
