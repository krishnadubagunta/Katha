# katha-parsers

Internal parser library for `katha`.

This crate exposes adapter-style parsers that normalize different document formats into a single `Document` model used by the CLI.

## Scope

- Crate name: `katha-parsers` (library path: `katha_parsers`)
- Supported parser types: `epub`, `docx`, `pdf`
- Public factory: `fetch_parser(parser_type: &str) -> Result<Box<dyn Parser>, ParserError>`
- Output: `Document` serialized by the root CLI crate

## Public API

### Data model

- `Document`
  - `cover_image: String` (base64, when available)
  - `title: String`
  - `subtitle: String`
  - `author: String`
  - `language: String`
  - `description: String` (markdown/plain text by parser)
  - `toc: Vec<Section>`
  - `content: HashMap<usize, String>` (`content_ref` -> markdown/plain text)
- `Section`
  - `id: String`
  - `title: String`
  - `content_ref: usize`
  - `children: Vec<Section>`

### Parser trait

All adapters implement:

```rust
pub trait Parser {
    fn parse(&mut self, src: &str) -> Result<Document, ParserError>;
    fn get_cover(&mut self) -> Result<String, ParserError>;
    fn get_title(&mut self) -> Result<String, ParserError>;
    fn get_subtitle(&mut self) -> Result<String, ParserError>;
    fn get_author(&mut self) -> Result<String, ParserError>;
    fn get_description(&mut self) -> Result<String, ParserError>;
    fn get_publisher(&mut self) -> Result<String, ParserError>;
    fn get_language(&mut self) -> Result<String, ParserError>;
    fn get_cover_image(&mut self) -> Result<String, ParserError>;
    fn get_toc(&mut self) -> Result<Vec<Section>, ParserError>;
    fn get_content_by_chapter(&mut self) -> Result<HashMap<usize, String>, ParserError>;
    fn clean_html(html: &str) -> Result<String, ParserError>
    where
        Self: Sized;
}
```

## Parser registry

- `SUPPORTED_PARSERS`: `["epub", "docx", "pdf"]`
- `fetch_parser` dispatches by parser key and returns `ParserError::UndefinedParser` for unknown types.

## Error model

`ParserError` is mapped to stable exit-friendly numeric codes:

- `FileDoesNotExist` -> `3006`
- `UnreadableFile` -> `4005`
- `UndefinedParser` -> `4006`
- `InvalidContent` -> `4007`

Human-readable text is provided by `ParserError::message()`.

## Runtime behavior

Expected integration flow with the CLI crate:

1. CLI determines extension/parser key.
2. CLI calls `fetch_parser`.
3. Parser `parse` validates input path and extracts document data.
4. Parser returns normalized `Document`.
5. CLI serializes `Document` to JSON and prints.

## Implementation notes by parser

- `epub`
  - Uses EPUB metadata and TOC/navigation points when present.
  - Converts HTML content to markdown.
- `docx`
  - Extracts `word/document.xml` and `docProps/core.xml` from ZIP container.
  - Infers TOC from heading styles (`Heading1`, `Heading2`, ...).
- `pdf`
  - Extracts raw text, detects headings heuristically, and creates flat sections.
  - Falls back to single-section document when no headings are detected.

## Add a new parser

1. Create module folder: `parsers/src/<type>/`.
2. Implement a parser struct that implements `Parser`.
3. Export module from `parsers/src/lib.rs`.
4. Register it in `fetch_parser`.
5. Ensure `toc[*].content_ref` keys map to `Document.content`.
6. Keep behavior non-panicking and return `ParserError` variants.
7. Add parser-specific dependencies only to `parsers/Cargo.toml`.

## Development

From workspace root:

- `cargo check`
- `cargo test -p katha-parsers`
- `cargo test -p katha-parsers --doc`
