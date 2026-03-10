use std::collections::HashMap;
use std::path::Path;

use crate::ContentBlock;
use crate::ContentKind;
use crate::error::ParserError;
use crate::{Document, Parser, Section};
use regex::Regex;

#[derive(Debug, Clone)]
struct Heading {
    title: String,
    line_index: usize,
    content_ref: usize,
}

#[derive(Debug, Clone)]
struct ParsedPdf {
    lines: Vec<String>,
    headings: Vec<Heading>,
}

/// PDF parser adapter.
///
/// This parser extracts plain text from the source PDF, applies heading
/// heuristics to build a table of contents, and returns normalized content
/// through the crate-level [`crate::Parser`] contract.
pub struct Pdf {
    source: Option<String>,
    parsed: Option<ParsedPdf>,
    document: Option<Document>,
}

impl Pdf {
    /// Creates a new PDF parser instance with no bound source.
    pub fn new() -> Self {
        Self {
            source: None,
            parsed: None,
            document: None,
        }
    }

    fn ensure_parsed(&mut self) -> Result<&ParsedPdf, ParserError> {
        if self.parsed.is_none() {
            let src = self.source.clone().ok_or(ParserError::UnreadableFile)?;
            let text = pdf_extract::extract_text(&src).map_err(|_| ParserError::UnreadableFile)?;

            let lines: Vec<String> = text
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect();
            let headings = Self::detect_headings(&lines);

            self.parsed = Some(ParsedPdf { lines, headings });
        }

        self.parsed.as_ref().ok_or(ParserError::UnreadableFile)
    }

    fn detect_headings(lines: &[String]) -> Vec<Heading> {
        let numbered_heading = Regex::new(r"^\d+(\.\d+)*\s+\S+").ok();
        let chapter_heading = Regex::new(r"^(chapter|part|section)\b").ok();

        let mut headings = Vec::new();
        for (idx, line) in lines.iter().enumerate() {
            let compact = line.trim();
            if compact.is_empty() || compact.len() > 120 {
                continue;
            }

            let lower = compact.to_ascii_lowercase();
            let looks_chapter = chapter_heading
                .as_ref()
                .map(|re| re.is_match(&lower))
                .unwrap_or(false);
            let looks_numbered = numbered_heading
                .as_ref()
                .map(|re| re.is_match(compact))
                .unwrap_or(false);
            let looks_all_caps = compact.len() > 4
                && compact
                    .chars()
                    .all(|c| !c.is_alphabetic() || c.is_uppercase());

            if looks_chapter || looks_numbered || looks_all_caps {
                let content_ref = headings.len();
                headings.push(Heading {
                    title: compact.to_string(),
                    line_index: idx,
                    content_ref,
                });
            }
        }

        headings
    }

    fn lines_to_content(lines: &[String]) -> Vec<ContentBlock> {
        lines
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .map(|line| ContentBlock {
                kind: ContentKind::Paragraph,
                content: Some(line.to_string()),
                items: Vec::new(),
                level: None,
            })
            .collect()
    }
}

impl Parser for Pdf {
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
        let src = self.source.clone().unwrap_or_default();
        let title = Path::new(&src)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        Ok(title)
    }

    fn get_subtitle(&mut self) -> Result<String, ParserError> {
        Ok(String::new())
    }

    fn get_author(&mut self) -> Result<String, ParserError> {
        Ok(String::new())
    }

    fn get_description(&mut self) -> Result<String, ParserError> {
        Ok(String::new())
    }

    fn get_publisher(&mut self) -> Result<String, ParserError> {
        Ok(String::new())
    }

    fn get_language(&mut self) -> Result<String, ParserError> {
        Ok(String::new())
    }

    fn get_cover_image(&mut self) -> Result<String, ParserError> {
        self.get_cover()
    }

    fn get_toc(&mut self) -> Result<Vec<Section>, ParserError> {
        let parsed = self.ensure_parsed()?;
        if parsed.headings.is_empty() {
            return Ok(vec![Section {
                id: "sec_000001".to_string(),
                title: "Document".to_string(),
                content_ref: 0,
                children: Vec::new(),
            }]);
        }

        let sections = parsed
            .headings
            .iter()
            .enumerate()
            .map(|(idx, heading)| Section {
                id: format!("sec_{:06}", idx + 1),
                title: heading.title.clone(),
                content_ref: heading.content_ref,
                children: Vec::new(),
            })
            .collect();
        Ok(sections)
    }

    fn get_content_by_chapter(&mut self) -> Result<HashMap<usize, Vec<ContentBlock>>, ParserError> {
        let parsed = self.ensure_parsed()?;
        let mut content = HashMap::new();

        if parsed.headings.is_empty() {
            content.insert(0, Self::lines_to_content(&parsed.lines));
            return Ok(content);
        }

        for (idx, heading) in parsed.headings.iter().enumerate() {
            let start = heading.line_index;
            let end = parsed
                .headings
                .get(idx + 1)
                .map(|next| next.line_index)
                .unwrap_or(parsed.lines.len());
            let mut blocks = Vec::new();
            for (offset, line) in parsed.lines[start..end].iter().enumerate() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                if offset == 0 {
                    blocks.push(ContentBlock {
                        kind: ContentKind::Heading,
                        content: Some(trimmed.to_string()),
                        items: Vec::new(),
                        level: Some(1),
                    });
                } else {
                    blocks.push(ContentBlock {
                        kind: ContentKind::Paragraph,
                        content: Some(trimmed.to_string()),
                        items: Vec::new(),
                        level: None,
                    });
                }
            }

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
