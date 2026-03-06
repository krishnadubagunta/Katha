use katha_parsers::error::ParserError;
use katha_parsers::fetch_parser;

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_path(prefix: &str, suffix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "katha_{prefix}_{}_{}{suffix}",
        std::process::id(),
        nanos
    ))
}

#[test]
fn fetch_parser_rejects_unknown_parser_type() {
    let result = fetch_parser("txt");
    assert!(matches!(result, Err(ParserError::UndefinedParser)));
}

#[test]
fn all_parsers_return_file_does_not_exist_for_missing_file() {
    let missing = unique_path("missing", ".bin");
    let missing_str = missing.to_string_lossy().to_string();

    for parser_type in ["epub", "docx", "pdf"] {
        let mut parser = fetch_parser(parser_type).expect("supported parser should resolve");
        let result = parser.parse(&missing_str);
        assert!(
            matches!(result, Err(ParserError::FileDoesNotExist)),
            "parser {parser_type} should report missing file"
        );
    }
}

#[test]
fn all_parsers_return_unreadable_file_for_directory_input() {
    let dir = unique_path("dir_input", "");
    fs::create_dir_all(&dir).expect("should create temp directory");
    let dir_str = dir.to_string_lossy().to_string();

    for parser_type in ["epub", "docx", "pdf"] {
        let mut parser = fetch_parser(parser_type).expect("supported parser should resolve");
        let result = parser.parse(&dir_str);
        assert!(
            matches!(result, Err(ParserError::UnreadableFile)),
            "parser {parser_type} should reject directory input"
        );
    }

    fs::remove_dir_all(dir).expect("should remove temp directory");
}

#[test]
fn parser_error_codes_and_messages_are_stable() {
    assert_eq!(ParserError::FileDoesNotExist.code(), 3006);
    assert_eq!(
        ParserError::FileDoesNotExist.message(),
        "File does not exist"
    );

    assert_eq!(ParserError::UnreadableFile.code(), 4005);
    assert_eq!(ParserError::UnreadableFile.message(), "Unreadable file");

    assert_eq!(ParserError::UndefinedParser.code(), 4006);
    assert_eq!(
        ParserError::UndefinedParser.message(),
        "Undefined parser error"
    );

    assert_eq!(ParserError::InvalidContent.code(), 4007);
    assert_eq!(ParserError::InvalidContent.message(), "Invalid content");
}
