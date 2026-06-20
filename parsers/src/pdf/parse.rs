use std::collections::HashMap;
use std::path::Path;

use kuchiki::traits::*;
use pdf_oxide::converters::ConversionOptions;
use pdf_oxide::document::PdfDocument;
use pdf_oxide::{Destination, OutlineItem};

use crate::ContentBlock;
use crate::ContentKind;
use crate::error::ParserError;
use crate::html::{
    block_from_node, blocks_to_content, blocks_to_plain_text, clean_dom, extract_clean_blocks,
    html_to_text,
};
use crate::{Document, Parser, Section};

#[derive(Debug, Clone)]
struct PdfSection {
    title: String,
    content_ref: usize,
    blocks: Vec<ContentBlock>,
}

#[derive(Debug, Clone)]
struct ParsedPdf {
    sections: Vec<PdfSection>,
}

/// PDF parser adapter.
///
/// Uses the PDF's own outline (bookmark tree) to determine chapter boundaries.
/// Per chapter, text is extracted via the shared HTML pipeline, tables via
/// `extract_tables`, and embedded images via `extract_images`.
/// Falls back to heading-based splitting when the document has no outline.
pub struct Pdf {
    source: Option<String>,
    parsed: Option<ParsedPdf>,
    document: Option<Document>,
}

impl Pdf {
    pub fn new() -> Self {
        Self {
            source: None,
            parsed: None,
            document: None,
        }
    }

    /// Flatten a nested outline into DFS order: (title, page_0indexed).
    fn flatten_outline(items: &[OutlineItem]) -> Vec<(String, usize)> {
        let mut result = Vec::new();
        for item in items {
            if let Some(Destination::PageIndex(page)) = item.dest {
                result.push((item.title.trim().to_string(), page));
            }
            if !item.children.is_empty() {
                result.extend(Self::flatten_outline(&item.children));
            }
        }
        result
    }

    /// Parse the combined HTML (from `to_html_all`) into text blocks keyed by
    /// 0-indexed page number.
    fn text_blocks_by_page(
        html: &str,
    ) -> HashMap<usize, Vec<crate::html::TextBlock>> {
        let document = kuchiki::parse_html().one(html);
        clean_dom(&document);

        let mut map: HashMap<usize, Vec<crate::html::TextBlock>> = HashMap::new();

        let Ok(page_divs) = document.select("div.page") else {
            return map;
        };

        for page_div in page_divs {
            let page_0idx = {
                let attrs = page_div.attributes.borrow();
                match attrs.get("data-page").and_then(|s| s.parse::<usize>().ok()) {
                    Some(n) if n > 0 => n - 1,
                    _ => continue,
                }
            };

            let node = page_div.as_node();
            let Ok(elements) = node.select("h1, h2, h3, p, blockquote, ul, ol") else {
                continue;
            };

            let page_blocks: Vec<_> = elements
                .filter_map(|m| {
                    let tag = m.name.local.as_ref();
                    block_from_node(&m.as_node().clone(), tag)
                })
                .collect();

            map.insert(page_0idx, page_blocks);
        }

        map
    }

    /// Extract table blocks from a single page.
    fn table_blocks_for_page(doc: &PdfDocument, page: usize) -> Vec<ContentBlock> {
        let Ok(tables) = doc.extract_tables(page) else {
            return Vec::new();
        };

        tables
            .into_iter()
            .filter_map(|table| {
                let items: Vec<String> = table
                    .rows
                    .iter()
                    .map(|row| {
                        row.cells
                            .iter()
                            .map(|c| c.text.trim().to_string())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .filter(|row| !row.trim().is_empty())
                    .collect();

                if items.is_empty() {
                    return None;
                }

                Some(ContentBlock {
                    kind: ContentKind::Table,
                    content: None,
                    items,
                    level: None,
                })
            })
            .collect()
    }

    /// Extract image (figure) blocks from a single page.
    fn figure_blocks_for_page(doc: &PdfDocument, page: usize) -> Vec<ContentBlock> {
        let Ok(images) = doc.extract_images(page) else {
            return Vec::new();
        };

        images
            .into_iter()
            .filter_map(|img| {
                let uri = img.to_base64_data_uri().ok()?;
                Some(ContentBlock {
                    kind: ContentKind::Figure,
                    content: Some(uri),
                    items: Vec::new(),
                    level: None,
                })
            })
            .collect()
    }

    /// Assemble all content blocks for a page range: text → tables → figures,
    /// in page order.
    fn blocks_for_pages(
        doc: &PdfDocument,
        text_by_page: &HashMap<usize, Vec<crate::html::TextBlock>>,
        pages: std::ops::Range<usize>,
    ) -> Vec<ContentBlock> {
        let mut all = Vec::new();
        for page in pages {
            // Text
            if let Some(text_blocks) = text_by_page.get(&page) {
                all.extend(blocks_to_content(text_blocks));
            }
            // Tables
            all.extend(Self::table_blocks_for_page(doc, page));
            // Figures
            all.extend(Self::figure_blocks_for_page(doc, page));
        }
        all
    }

    fn build_outline_sections(
        doc: &PdfDocument,
        html: &str,
        outline_items: &[(String, usize)],
        page_count: usize,
    ) -> Vec<PdfSection> {
        let text_by_page = Self::text_blocks_by_page(html);
        let mut sections = Vec::new();

        for (idx, (title, start_page)) in outline_items.iter().enumerate() {
            let end_page = outline_items
                .get(idx + 1)
                .map(|(_, p)| *p)
                .unwrap_or(page_count);

            let blocks =
                Self::blocks_for_pages(doc, &text_by_page, *start_page..end_page);
            if blocks.is_empty() {
                continue;
            }

            sections.push(PdfSection {
                title: title.clone(),
                content_ref: sections.len(),
                blocks,
            });
        }

        sections
    }

    fn build_heading_sections(
        doc: &PdfDocument,
        html: &str,
        page_count: usize,
    ) -> Vec<PdfSection> {
        let text_by_page = Self::text_blocks_by_page(html);

        // Collect all text blocks in page order to find heading positions.
        let all_text_blocks: Vec<crate::html::TextBlock> = (0..page_count)
            .flat_map(|p| {
                text_by_page
                    .get(&p)
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                    .iter()
                    .cloned()
            })
            .collect();

        let all_blocks = blocks_to_content(&all_text_blocks);
        if all_blocks.is_empty() {
            return Vec::new();
        }

        // Split on h1 only; fall back to all headings if none.
        let h1_indices: Vec<usize> = all_blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.kind == ContentKind::Heading && b.level == Some(1))
            .map(|(i, _)| i)
            .collect();

        let heading_indices: Vec<usize> = if h1_indices.is_empty() {
            all_blocks
                .iter()
                .enumerate()
                .filter(|(_, b)| b.kind == ContentKind::Heading)
                .map(|(i, _)| i)
                .collect()
        } else {
            h1_indices
        };

        if heading_indices.is_empty() {
            // No headings at all — one section for the full document.
            let blocks = Self::blocks_for_pages(doc, &text_by_page, 0..page_count);
            return if blocks.is_empty() {
                Vec::new()
            } else {
                vec![PdfSection {
                    title: "Document".to_string(),
                    content_ref: 0,
                    blocks,
                }]
            };
        }

        // Group consecutive h1s with no body content between them into one title.
        let mut chapter_starts: Vec<Vec<usize>> = Vec::new();
        for &idx in &heading_indices {
            let prev_end = chapter_starts
                .last()
                .and_then(|g| g.last())
                .copied()
                .map(|i| i + 1)
                .unwrap_or(0);

            let has_body = all_blocks[prev_end..idx]
                .iter()
                .any(|b| b.kind != ContentKind::Heading);

            if has_body || chapter_starts.is_empty() {
                chapter_starts.push(vec![idx]);
            } else {
                chapter_starts.last_mut().unwrap().push(idx);
            }
        }

        // For heading-based splitting we don't have page boundaries per chapter,
        // so tables and figures can't be assigned to chapters without positional
        // data. Collect all tables+figures once and append after last chapter.
        // TODO: use bbox info from pdf_oxide to interleave properly.
        let mut sections = Vec::new();
        for (pos, title_indices) in chapter_starts.iter().enumerate() {
            let first = title_indices[0];
            let end = chapter_starts
                .get(pos + 1)
                .and_then(|g| g.first())
                .copied()
                .unwrap_or(all_blocks.len());

            let title = title_indices
                .iter()
                .filter_map(|&i| all_blocks[i].content.as_deref())
                .collect::<Vec<_>>()
                .join(" ");

            let blocks = all_blocks[first..end].to_vec();
            if !blocks.is_empty() {
                sections.push(PdfSection {
                    title,
                    content_ref: sections.len(),
                    blocks,
                });
            }
        }

        sections
    }

    fn ensure_parsed(&mut self) -> Result<&ParsedPdf, ParserError> {
        if self.parsed.is_none() {
            let src = self.source.clone().ok_or(ParserError::UnreadableFile)?;
            let doc =
                PdfDocument::open(&src).map_err(|_| ParserError::UnreadableFile)?;

            let outline_flat = doc
                .get_outline()
                .ok()
                .flatten()
                .map(|items| Self::flatten_outline(&items))
                .unwrap_or_default();

            let page_count = doc
                .page_count()
                .map_err(|_| ParserError::UnreadableFile)?;

            let sections = if outline_flat.len() > 5 {
                let options = ConversionOptions {
                    preserve_layout: false,
                    detect_headings: false,
                    extract_tables: false, // we call extract_tables ourselves per page
                    include_images: false,
                    ..Default::default()
                };
                let html = doc
                    .to_html_all(&options)
                    .map_err(|_| ParserError::UnreadableFile)?;
                Self::build_outline_sections(&doc, &html, &outline_flat, page_count)
            } else {
                let options = ConversionOptions {
                    preserve_layout: false,
                    detect_headings: true,
                    extract_tables: false,
                    include_images: false,
                    ..Default::default()
                };
                let html = doc
                    .to_html_all(&options)
                    .map_err(|_| ParserError::UnreadableFile)?;
                Self::build_heading_sections(&doc, &html, page_count)
            };

            self.parsed = Some(ParsedPdf { sections });
        }

        self.parsed.as_ref().ok_or(ParserError::UnreadableFile)
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

        let content = self.get_content_by_chapter()?;
        if content.is_empty() {
            return Err(ParserError::InvalidContent);
        }
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
        let sections = parsed
            .sections
            .iter()
            .enumerate()
            .map(|(idx, s)| Section {
                id: format!("sec_{:06}", idx + 1),
                title: s.title.clone(),
                content_ref: s.content_ref,
                children: Vec::new(),
            })
            .collect();
        Ok(sections)
    }

    fn get_content_by_chapter(
        &mut self,
    ) -> Result<HashMap<usize, Vec<ContentBlock>>, ParserError> {
        let parsed = self.ensure_parsed()?;
        let content = parsed
            .sections
            .iter()
            .map(|s| (s.content_ref, s.blocks.clone()))
            .collect();
        Ok(content)
    }

    fn clean_html(html: &str) -> Result<String, ParserError>
    where
        Self: Sized,
    {
        let blocks = extract_clean_blocks(html);
        if blocks.is_empty() {
            return Ok(html_to_text(html));
        }
        Ok(blocks_to_plain_text(&blocks))
    }
}
