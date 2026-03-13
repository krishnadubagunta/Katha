//! PDF parser adapter.
//!
//! Exposes [`Pdf`], an implementation of the crate-level [`Parser`](crate::Parser)
//! trait that extracts text from PDFs and maps it into the normalized
//! [`Document`](crate::Document) model.
//!
//! # Examples
//!
//! ```no_run
//! use katha_parsers::{Parser, pdf::Pdf};
//!
//! let mut parser = Pdf::new();
//! let document = parser.parse("book.pdf")?;
//! assert!(!document.content.is_empty());
//! # Ok::<(), katha_parsers::error::ParserError>(())
//! ```

mod parse;

/// PDF parser implementation.
pub use parse::Pdf;
