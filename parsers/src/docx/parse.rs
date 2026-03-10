use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::ContentBlock;
use crate::ContentKind;
use crate::error::ParserError;
use crate::{Document, Parser, Section};
use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::QName;
use zip::ZipArchive;

#[derive(Debug, Clone)]
struct Paragraph {
    text: String,
    heading_level: Option<usize>,
}

#[derive(Debug, Clone)]
struct HeadingRecord {
    para_index: usize,
    level: usize,
    title: String,
    content_ref: usize,
}

#[derive(Debug, Clone)]
struct ParsedDocx {
    paragraphs: Vec<Paragraph>,
    headings: Vec<HeadingRecord>,
    title: String,
    subtitle: String,
    author: String,
    language: String,
    description: String,
}

#[derive(Debug, Clone)]
struct SectionDraft {
    id: String,
    title: String,
    content_ref: usize,
    level: usize,
    children: Vec<SectionDraft>,
}

/// DOCX parser adapter.
///
/// This parser reads `word/document.xml` and metadata from `docProps/core.xml`,
/// derives a heading tree from paragraph styles, and returns normalized output
/// through the crate-level [`crate::Parser`] trait.
pub struct Docx {
    source: Option<String>,
    parsed: Option<ParsedDocx>,
    document: Option<Document>,
}

impl Docx {
    /// Creates a new DOCX parser instance with no bound source.
    pub fn new() -> Self {
        Self {
            source: None,
            parsed: None,
            document: None,
        }
    }

    fn read_zip_entry(archive: &mut ZipArchive<File>, name: &str) -> Option<String> {
        let mut file = archive.by_name(name).ok()?;
        let mut xml = String::new();
        file.read_to_string(&mut xml).ok()?;
        Some(xml)
    }

    fn local_name(name: QName<'_>) -> String {
        let raw = String::from_utf8_lossy(name.as_ref());
        raw.rsplit(':').next().unwrap_or_default().to_string()
    }

    fn attr_value(start: &BytesStart<'_>, wanted: &str) -> Option<String> {
        start.attributes().flatten().find_map(|attr| {
            let key = String::from_utf8_lossy(attr.key.as_ref());
            if key.ends_with(wanted) {
                Some(String::from_utf8_lossy(&attr.value).to_string())
            } else {
                None
            }
        })
    }

    fn heading_level_from_style(style: &str) -> Option<usize> {
        let style = style.to_ascii_lowercase();
        if !style.starts_with("heading") {
            return None;
        }
        let level_digits = style.trim_start_matches("heading");
        level_digits
            .parse::<usize>()
            .ok()
            .filter(|level| *level > 0)
    }

    fn parse_document_xml(xml: &str) -> Vec<Paragraph> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut paragraphs = Vec::new();
        let mut in_paragraph = false;
        let mut in_text = false;
        let mut paragraph_text = String::new();
        let mut paragraph_style: Option<String> = None;

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let name = Self::local_name(e.name());
                    match name.as_str() {
                        "p" => {
                            in_paragraph = true;
                            paragraph_text.clear();
                            paragraph_style = None;
                        }
                        "t" => {
                            in_text = true;
                        }
                        "pStyle" if in_paragraph => {
                            if let Some(style) = Self::attr_value(&e, "val") {
                                paragraph_style = Some(style);
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::Empty(e)) => {
                    let name = Self::local_name(e.name());
                    match name.as_str() {
                        "tab" if in_paragraph => paragraph_text.push('\t'),
                        "br" if in_paragraph => paragraph_text.push('\n'),
                        "pStyle" if in_paragraph => {
                            if let Some(style) = Self::attr_value(&e, "val") {
                                paragraph_style = Some(style);
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(e)) => {
                    if in_paragraph && in_text {
                        let raw = String::from_utf8_lossy(e.as_ref());
                        if let Ok(unescaped) = unescape(&raw) {
                            paragraph_text.push_str(&unescaped);
                        }
                    }
                }
                Ok(Event::End(e)) => {
                    let name = Self::local_name(e.name());
                    match name.as_str() {
                        "t" => in_text = false,
                        "p" => {
                            let text = paragraph_text.trim().to_string();
                            let heading_level = paragraph_style
                                .as_deref()
                                .and_then(Self::heading_level_from_style);

                            if !text.is_empty() || heading_level.is_some() {
                                paragraphs.push(Paragraph {
                                    text,
                                    heading_level,
                                });
                            }

                            in_paragraph = false;
                            in_text = false;
                        }
                        _ => {}
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
        }

        paragraphs
    }

    fn parse_core_xml(xml: &str) -> HashMap<String, String> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut out = HashMap::new();
        let mut current_tag = String::new();

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    current_tag = Self::local_name(e.name());
                }
                Ok(Event::Text(e)) => {
                    let raw = String::from_utf8_lossy(e.as_ref());
                    if let Ok(unescaped) = unescape(&raw) {
                        if !current_tag.is_empty() {
                            out.insert(current_tag.clone(), unescaped.into_owned());
                        }
                    }
                }
                Ok(Event::End(_)) => current_tag.clear(),
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
        }

        out
    }

    fn ensure_parsed(&mut self) -> Result<&ParsedDocx, ParserError> {
        if self.parsed.is_none() {
            let src = self.source.clone().ok_or(ParserError::UnreadableFile)?;
            let file = File::open(&src).map_err(|_| ParserError::UnreadableFile)?;
            let mut archive = ZipArchive::new(file).map_err(|_| ParserError::UnreadableFile)?;

            let document_xml = Self::read_zip_entry(&mut archive, "word/document.xml")
                .ok_or(ParserError::UnreadableFile)?;
            let core_xml = Self::read_zip_entry(&mut archive, "docProps/core.xml");

            let paragraphs = Self::parse_document_xml(&document_xml);
            let core = core_xml
                .as_deref()
                .map(Self::parse_core_xml)
                .unwrap_or_default();

            let mut headings = Vec::new();
            for (para_index, paragraph) in paragraphs.iter().enumerate() {
                if let Some(level) = paragraph.heading_level {
                    let content_ref = headings.len();
                    headings.push(HeadingRecord {
                        para_index,
                        level,
                        title: paragraph.text.clone(),
                        content_ref,
                    });
                }
            }

            let title = core.get("title").cloned().unwrap_or_default();
            let subtitle = core.get("subject").cloned().unwrap_or_default();
            let author = core.get("creator").cloned().unwrap_or_default();
            let language = core.get("language").cloned().unwrap_or_default();
            let description = core.get("description").cloned().unwrap_or_default();

            self.parsed = Some(ParsedDocx {
                paragraphs,
                headings,
                title,
                subtitle,
                author,
                language,
                description,
            });
        }

        self.parsed.as_ref().ok_or(ParserError::UnreadableFile)
    }

    fn attach_node(stack: &mut Vec<SectionDraft>, roots: &mut Vec<SectionDraft>) {
        if let Some(node) = stack.pop() {
            if let Some(parent) = stack.last_mut() {
                parent.children.push(node);
            } else {
                roots.push(node);
            }
        }
    }

    fn finalize_tree(
        mut stack: Vec<SectionDraft>,
        mut roots: Vec<SectionDraft>,
    ) -> Vec<SectionDraft> {
        while !stack.is_empty() {
            Self::attach_node(&mut stack, &mut roots);
        }
        roots
    }

    fn draft_to_section(draft: SectionDraft) -> Section {
        Section {
            id: draft.id,
            title: draft.title,
            content_ref: draft.content_ref,
            children: draft
                .children
                .into_iter()
                .map(Self::draft_to_section)
                .collect(),
        }
    }

    fn content_from_paragraphs(paragraphs: &[Paragraph]) -> Vec<ContentBlock> {
        paragraphs
            .iter()
            .filter_map(|paragraph| {
                let text = paragraph.text.trim().to_string();
                if text.is_empty() {
                    return None;
                }

                let level = paragraph
                    .heading_level
                    .and_then(|level| u8::try_from(level).ok());

                Some(ContentBlock {
                    kind: if level.is_some() {
                        ContentKind::Heading
                    } else {
                        ContentKind::Paragraph
                    },
                    content: Some(text),
                    items: Vec::new(),
                    level,
                })
            })
            .collect()
    }
}

impl Parser for Docx {
    fn parse(&mut self, src: &str) -> Result<Document, ParserError> {
        let path = Path::new(src);
        if !path.exists() {
            return Err(ParserError::FileDoesNotExist);
        }
        if !path.is_file() {
            return Err(ParserError::UnreadableFile);
        }

        self.source = Some(src.to_string());
        let toc = self.get_toc().unwrap_or_default();
        self.document = Some(Document {
            cover_image: self.get_cover_image().unwrap_or_default(),
            title: self.get_title().unwrap_or_default(),
            subtitle: self.get_subtitle().unwrap_or_default(),
            author: self.get_author().unwrap_or_default(),
            language: self.get_language().unwrap_or_default(),
            description: self.get_description().unwrap_or_default(),
            toc,
            content: HashMap::new(),
        });

        let content = self.get_content_by_chapter().unwrap_or_default();
        if let Some(document) = self.document.as_mut() {
            document.content = content;
        }

        self.document.clone().ok_or(ParserError::UnreadableFile)
    }

    fn get_cover(&mut self) -> Result<String, ParserError> {
        Ok(String::new())
    }

    fn get_title(&mut self) -> Result<String, ParserError> {
        Ok(self.ensure_parsed()?.title.clone())
    }

    fn get_subtitle(&mut self) -> Result<String, ParserError> {
        Ok(self.ensure_parsed()?.subtitle.clone())
    }

    fn get_author(&mut self) -> Result<String, ParserError> {
        Ok(self.ensure_parsed()?.author.clone())
    }

    fn get_description(&mut self) -> Result<String, ParserError> {
        Ok(self.ensure_parsed()?.description.clone())
    }

    fn get_publisher(&mut self) -> Result<String, ParserError> {
        Ok(String::new())
    }

    fn get_language(&mut self) -> Result<String, ParserError> {
        Ok(self.ensure_parsed()?.language.clone())
    }

    fn get_cover_image(&mut self) -> Result<String, ParserError> {
        self.get_cover()
    }

    fn get_toc(&mut self) -> Result<Vec<Section>, ParserError> {
        let parsed = self.ensure_parsed()?;

        if parsed.headings.is_empty() {
            return Ok(vec![Section {
                id: "sec_000001".to_string(),
                title: if parsed.title.is_empty() {
                    "Document".to_string()
                } else {
                    parsed.title.clone()
                },
                content_ref: 0,
                children: Vec::new(),
            }]);
        }

        let mut roots: Vec<SectionDraft> = Vec::new();
        let mut stack: Vec<SectionDraft> = Vec::new();

        for (i, heading) in parsed.headings.iter().enumerate() {
            let node = SectionDraft {
                id: format!("sec_{:06}", i + 1),
                title: if heading.title.is_empty() {
                    format!("Section {}", i + 1)
                } else {
                    heading.title.clone()
                },
                content_ref: heading.content_ref,
                level: heading.level,
                children: Vec::new(),
            };

            while stack
                .last()
                .map(|parent| parent.level >= node.level)
                .unwrap_or(false)
            {
                Self::attach_node(&mut stack, &mut roots);
            }

            stack.push(node);
        }

        let tree = Self::finalize_tree(stack, roots);
        Ok(tree.into_iter().map(Self::draft_to_section).collect())
    }

    fn get_content_by_chapter(&mut self) -> Result<HashMap<usize, Vec<ContentBlock>>, ParserError> {
        let parsed = self.ensure_parsed()?;
        let mut content = HashMap::new();

        if parsed.headings.is_empty() {
            content.insert(0, Self::content_from_paragraphs(&parsed.paragraphs));
            return Ok(content);
        }

        for (idx, heading) in parsed.headings.iter().enumerate() {
            let start = heading.para_index;
            let end = parsed
                .headings
                .get(idx + 1)
                .map(|next| next.para_index)
                .unwrap_or(parsed.paragraphs.len());
            let blocks = Self::content_from_paragraphs(&parsed.paragraphs[start..end]);
            content.insert(heading.content_ref, blocks);
        }

        Ok(content)
    }

    fn clean_html(html: &str) -> Result<String, ParserError>
    where
        Self: Sized,
    {
        Ok(html.to_string())
    }
}
