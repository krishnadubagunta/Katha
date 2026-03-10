use katha_parsers::Parser;
use katha_parsers::docx::Docx;
use katha_parsers::{ContentBlock, ContentKind};

use std::fs::File;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use zip::ZipWriter;
use zip::write::FileOptions;

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

fn assert_block_matches_canonical_contract(block: &ContentBlock) {
    match block.kind {
        ContentKind::Heading => {
            assert!(
                block
                    .content
                    .as_deref()
                    .is_some_and(|value| !value.is_empty()),
                "heading blocks must carry text content"
            );
            assert!(
                block.level.is_some(),
                "heading blocks must carry a heading level"
            );
            assert!(
                block.items.is_empty(),
                "heading blocks must not use list items"
            );
        }
        ContentKind::Paragraph | ContentKind::Quote => {
            assert!(
                block
                    .content
                    .as_deref()
                    .is_some_and(|value| !value.is_empty()),
                "text blocks must carry text content"
            );
            assert!(
                block.level.is_none(),
                "non-heading text blocks must not carry a heading level"
            );
            assert!(
                block.items.is_empty(),
                "non-list text blocks must not use list items"
            );
        }
        ContentKind::List => {
            assert!(
                block.content.is_none(),
                "list blocks should store entries in items rather than content"
            );
            assert!(block.level.is_none(), "list blocks must not carry a level");
            assert!(
                !block.items.is_empty(),
                "list blocks must carry at least one list item"
            );
        }
    }
}

fn assert_section_refs_resolve(
    section: &katha_parsers::Section,
    content: &std::collections::HashMap<usize, Vec<ContentBlock>>,
) {
    assert!(
        content.contains_key(&section.content_ref),
        "section '{}' must resolve its content_ref {}",
        section.title,
        section.content_ref
    );

    for child in &section.children {
        assert_section_refs_resolve(child, content);
    }
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
        &vec![
            ContentBlock {
                kind: ContentKind::Heading,
                content: Some("Chapter 1".to_string()),
                items: Vec::new(),
                level: Some(1),
            },
            ContentBlock {
                kind: ContentKind::Paragraph,
                content: Some("Hello world.".to_string()),
                items: Vec::new(),
                level: None,
            }
        ]
    );
    assert_eq!(
        doc.content.get(&1).expect("section content should exist"),
        &vec![
            ContentBlock {
                kind: ContentKind::Heading,
                content: Some("Section 1.1".to_string()),
                items: Vec::new(),
                level: Some(2),
            },
            ContentBlock {
                kind: ContentKind::Paragraph,
                content: Some("Nested content.".to_string()),
                items: Vec::new(),
                level: None,
            }
        ]
    );

    std::fs::remove_file(path).expect("should remove fixture");
}

#[test]
fn parse_docx_output_matches_canonical_document_contract() {
    let path = unique_docx_path();
    write_minimal_docx(&path);

    let mut parser = Docx::new();
    let doc = parser
        .parse(path.to_string_lossy().as_ref())
        .expect("docx parse should succeed");

    assert!(
        !doc.toc.is_empty(),
        "parsed document should produce at least one toc section"
    );
    assert!(
        !doc.content.is_empty(),
        "parsed document should produce content keyed by content_ref"
    );

    for section in &doc.toc {
        assert_section_refs_resolve(section, &doc.content);
    }

    for (content_ref, blocks) in &doc.content {
        assert!(
            !blocks.is_empty(),
            "content_ref {content_ref} should contain at least one content block"
        );
        for block in blocks {
            assert_block_matches_canonical_contract(block);
        }
    }

    std::fs::remove_file(path).expect("should remove fixture");
}
