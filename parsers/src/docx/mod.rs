//! DOCX parser adapter.
//!
//! Exposes [`Docx`], an implementation of the crate-level [`Parser`](crate::Parser)
//! trait that reads OOXML document content and maps it into the normalized
//! [`Document`](crate::Document) model.
//!
//! # Examples
//!
//! ```no_run
//! use katha_parsers::{Parser, docx::Docx};
//!
//! let mut parser = Docx::new();
//! let document = parser.parse("book.docx")?;
//! assert!(!document.title.is_empty() || !document.toc.is_empty());
//! # Ok::<(), katha_parsers::error::ParserError>(())
//! ```

mod parse;

/// DOCX parser implementation.
pub use parse::Docx;
