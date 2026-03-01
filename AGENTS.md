# AGENTS.md

## Project Identity
- Name: `katha`
- Language: Rust (`edition = 2024`)
- Type: Workspace with:
- CLI app crate: root package `katha`
- Internal parser library crate: `parsers` (`katha-parsers`)
- Purpose: Parse document files through adapter-style parser implementations.
- Current parser support: `epub`, `docx`, `pdf`.
- Output format: JSON serialized `Document`.

## Workspace Layout
- `Cargo.toml`: workspace root + CLI package manifest.
- `src/main.rs`: CLI entrypoint.
- `src/error_codes.rs`: CLI-level error codes (currently usage only).
- `parsers/Cargo.toml`: parser library manifest (separate dependency boundary).
- `parsers/src/lib.rs`: parser trait + registry/factory.
- `parsers/src/error.rs`: parser error model and exit-code mapping.
- `parsers/src/epub/`: EPUB parser implementation.
- `parsers/src/docx/`: DOCX parser implementation.
- `parsers/src/pdf/`: PDF parser implementation.

## Runtime Flow
1. CLI reads a single input path argument.
2. CLI derives file extension.
3. CLI asks `katha_parsers::fetch_parser(extension)` for adapter.
4. Parser validates/parses file via `Parser::parse`.
5. CLI prints parsed `Document` as pretty JSON.
6. Errors are surfaced as `ParserError`, then mapped to message + exit code.

## Parser Library Contract
- Trait: `Parser`
- Method: `fn parse(&mut self, src: &str) -> Result<Document, ParserError>`
- Factory: `fetch_parser(parser_type: &str) -> Result<Box<dyn Parser>, ParserError>`
- Supported types registry: `SUPPORTED_PARSERS`

### Canonical Output Model
- `Document`:
- `cover_image: String` (base64)
- `title: String`
- `subtitle: String`
- `author: String`
- `language: String`
- `description: String` (markdown)
- `toc: Vec<Section>` (hierarchical logical sections)
- `content: HashMap<usize, String>` (markdown, keyed by `content_ref`)

- `Section`:
- `id: String` (unique section reference)
- `title: String`
- `content_ref: usize` (resolves into `Document.content`)
- `children: Vec<Section>`

## Error Model
- CLI crate:
- `1000` `CLI_ERROR` (missing/invalid CLI usage)
- Parser crate (`ParserError`):
- `3006` file does not exist
- `4005` unreadable file
- `4006` undefined parser

Use `ParserError::message()` for human-readable text and `ParserError::code()` for process exit code.

## How To Add A New Parser
1. Create `parsers/src/<type>/`.
2. Implement a parser type that implements `Parser`.
3. Export module in `parsers/src/lib.rs`.
4. Register parser in `fetch_parser`.
5. Return hierarchical `Vec<Section>` in `get_toc` and map section/subsection content via `content_ref`.
6. Populate `Document.content` using the same `content_ref` keys.
7. Add/update parser-specific error behavior in `ParserError` if needed.
8. Add parser crate dependencies only in `parsers/Cargo.toml`.
9. Run `cargo check` from workspace root.

## AI Agent Guardrails
- Keep parser logic inside the `parsers` crate.
- Keep CLI orchestration and argument UX in root `katha` crate.
- Do not add parser dependencies to root `Cargo.toml` unless they are also needed by CLI itself.
- Avoid panics in parser resolution/execution flow.
- Preserve numeric error code semantics for backward compatibility.
- Keep parser outputs adapter-consistent: all parser types must emit the same `Document` + `Section` schema.
- Ignore `target/` artifacts during edits.

## Known Gaps
- DOCX/PDF heading detection is heuristic and may need document-specific tuning.
- No automated tests yet.
- Extension detection is basic and may need normalization rules.

## Commands
- `cargo check`
- `cargo run -- <file-path>`
- `cargo test`
