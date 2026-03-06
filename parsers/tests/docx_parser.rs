use katha_parsers::docx::Docx;
use katha_parsers::Parser;

use std::fs::File;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::FileOptions;
use zip::ZipWriter;

fn unique_docx_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    std::env::temp_dir().join(format!("katha_docx_{}_{}.docx", std::process::id(), nanos))
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
    <w:p>
      <w:pPr><w:pStyle w:val="Heading2"/></w:pPr>
      <w:r><w:t>Section 1.1</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>Nested content.</w:t></w:r>
    </w:p>
  </w:body>
</w:document>"#;

    let core_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties
  xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
  xmlns:dc="http://purl.org/dc/elements/1.1/"
  xmlns:dcterms="http://purl.org/dc/terms/"
  xmlns:dcmitype="http://purl.org/dc/dcmitype/"
  xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <dc:title>Test Book</dc:title>
  <dc:subject>Deterministic Subtitle</dc:subject>
  <dc:creator>Unit Tester</dc:creator>
  <dc:language>en</dc:language>
  <dc:description>Docx fixture description</dc:description>
</cp:coreProperties>"#;

    zip.start_file("word/document.xml", options)
        .expect("should create document xml entry");
    use std::io::Write;
    zip.write_all(document_xml.as_bytes())
        .expect("should write document xml");

    zip.start_file("docProps/core.xml", options)
        .expect("should create core xml entry");
    zip.write_all(core_xml.as_bytes())
        .expect("should write core xml");

    zip.finish().expect("should finish docx archive");
}

#[test]
fn parse_docx_builds_metadata_toc_and_content_deterministically() {
    let path = unique_docx_path();
    write_minimal_docx(&path);

    let mut parser = Docx::new();
    let doc = parser
        .parse(path.to_string_lossy().as_ref())
        .expect("docx parse should succeed");

    assert_eq!(doc.title, "Test Book");
    assert_eq!(doc.subtitle, "Deterministic Subtitle");
    assert_eq!(doc.author, "Unit Tester");
    assert_eq!(doc.language, "en");
    assert_eq!(doc.description, "Docx fixture description");
    assert_eq!(doc.cover_image, "");

    assert_eq!(doc.toc.len(), 1);
    let chapter = &doc.toc[0];
    assert_eq!(chapter.title, "Chapter 1");
    assert_eq!(chapter.content_ref, 0);
    assert_eq!(chapter.children.len(), 1);

    let section = &chapter.children[0];
    assert_eq!(section.title, "Section 1.1");
    assert_eq!(section.content_ref, 1);

    assert_eq!(doc.content.len(), 2);
    assert_eq!(
        doc.content.get(&0).expect("chapter content should exist"),
        "Chapter 1\n\nHello world."
    );
    assert_eq!(
        doc.content.get(&1).expect("section content should exist"),
        "Section 1.1\n\nNested content."
    );

    std::fs::remove_file(path).expect("should remove fixture");
}
