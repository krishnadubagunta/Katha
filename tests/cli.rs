use serde_json::Value;
use std::fs::File;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use zip::ZipWriter;
use zip::write::FileOptions;

fn unique_path(prefix: &str, suffix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "katha_cli_{prefix}_{}_{}{suffix}",
        std::process::id(),
        nanos
    ))
}

fn run_katha(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_katha"))
        .args(args)
        .output()
        .expect("katha binary should execute")
}

fn truncated_exit_code(code: i32) -> i32 {
    code.rem_euclid(256)
}

fn write_minimal_docx(path: &PathBuf) {
    let file = File::create(path).expect("should create docx fixture");
    let mut zip = ZipWriter::new(file);
    let options = FileOptions::default();

    let document_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:pPr><w:pStyle w:val="Heading1"/></w:pPr>
      <w:r><w:t>Chapter 1</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>Hello world.</w:t></w:r>
    </w:p>
  </w:body>
</w:document>"#;

    let core_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties
  xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
  xmlns:dc="http://purl.org/dc/elements/1.1/">
  <dc:title>CLI Test Book</dc:title>
</cp:coreProperties>"#;

    use std::io::Write;
    zip.start_file("word/document.xml", options)
        .expect("should create document xml entry");
    zip.write_all(document_xml.as_bytes())
        .expect("should write document xml");

    zip.start_file("docProps/core.xml", options)
        .expect("should create core xml entry");
    zip.write_all(core_xml.as_bytes())
        .expect("should write core xml");

    zip.finish().expect("should finish docx archive");
}

#[test]
fn exits_with_usage_error_when_no_file_argument_is_provided() {
    let output = run_katha(&[]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(truncated_exit_code(1000)));
    assert!(stderr.contains("Usage:"));
}

#[test]
fn exits_with_undefined_parser_error_for_unsupported_extension() {
    let output = run_katha(&["/tmp/input.txt"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(truncated_exit_code(4006)));
    assert!(stderr.contains("Undefined parser error"));
}

#[test]
fn exits_with_file_not_found_error_for_missing_supported_file() {
    let missing = unique_path("missing", ".pdf");
    let output = run_katha(&[missing.to_string_lossy().as_ref()]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(truncated_exit_code(3006)));
    assert!(stderr.contains("File does not exist"));
}

#[test]
fn prints_document_json_for_valid_docx_file() {
    let docx_path = unique_path("valid", ".docx");
    write_minimal_docx(&docx_path);

    let output = run_katha(&[docx_path.to_string_lossy().as_ref()]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: Value = serde_json::from_str(&stdout).expect("stdout should be valid json");

    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    assert_eq!(json["title"], "CLI Test Book");
    assert!(json.get("toc").is_some());
    assert!(json.get("content").is_some());

    let chapter_content = json["content"]["0"]
        .as_array()
        .expect("chapter content should be an array of content blocks");
    assert_eq!(chapter_content.len(), 2);
    assert_eq!(chapter_content[0]["kind"], "heading");
    assert_eq!(chapter_content[0]["content"], "Chapter 1");
    assert_eq!(chapter_content[0]["level"], 1);
    assert_eq!(chapter_content[1]["kind"], "paragraph");
    assert_eq!(chapter_content[1]["content"], "Hello world.");

    std::fs::remove_file(docx_path).expect("should remove fixture");
}
