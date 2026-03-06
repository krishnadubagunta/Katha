# Katha
[![Rust CI](https://github.com/krishnadubagunta/Katha/actions/workflows/Workflow.yml/badge.svg)](https://github.com/krishnadubagunta/Katha/actions/workflows/Workflow.yml)

Katha is a Rust CLI for parsing file-based book formats using an internal parser library crate.

## Architecture
This repository is a Cargo workspace with two crates:

1. `katha` (root package): CLI orchestration and process-level behavior.
2. `katha-parsers` (`parsers/`): built-in parser library with isolated dependencies.

This split keeps parser dependencies separate from CLI concerns and makes parser growth easier over time.

## Current Support
- `epub`
- `docx`
- `pdf`

## Current Behavior
- Accepts one input path argument.
- Selects parser based on file extension.
- Runs parser adapter.
- Prints parsed `Document` as pretty JSON on success.
- Prints parser error message and exits with a numeric code on failure.

## Project Structure
- `Cargo.toml`: workspace root + `katha` package
- `src/main.rs`: CLI entrypoint
- `src/error_codes.rs`: CLI errors
- `parsers/Cargo.toml`: parser crate manifest
- `parsers/src/lib.rs`: parser trait + factory
- `parsers/src/error.rs`: parser error type
- `parsers/src/epub/parse.rs`: EPUB parser adapter
- `parsers/src/docx/parse.rs`: DOCX parser adapter
- `parsers/src/pdf/parse.rs`: PDF parser adapter

## Run
```bash
cargo run -- ./book.epub
cargo run -- ./book.docx
cargo run -- ./book.pdf
```

## Error Codes
- `1000`: CLI usage error
- `3006`: file does not exist
- `4005`: unreadable file
- `4006`: undefined parser type

## Add A New Parser
1. Add module under `parsers/src/<new_type>/`.
2. Implement `Parser` for the new adapter.
3. Return hierarchical `Vec<Section>` in `get_toc`.
4. Populate `Document.content` using `content_ref` keys.
5. Register it in `parsers/src/lib.rs` (`fetch_parser`).
6. Add format-specific dependencies to `parsers/Cargo.toml`.
7. Run `cargo check`.

## Development
```bash
cargo check
cargo test
```
