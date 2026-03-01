use base64::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crate::Document;
use crate::Parser;
use crate::Section;
use crate::error::ParserError;
use epub::doc::EpubDoc;
use epub::doc::NavPoint;
use html2md::parse_html;

pub struct Epub {
    source: Option<String>,
    doc: Option<EpubDoc<BufReader<File>>>,
    document: Option<Document>,
}

impl Epub {
    pub fn new() -> Self {
        Self {
            source: None,
            doc: None,
            document: None,
        }
    }

    fn ensure_doc(&mut self) -> Result<&mut EpubDoc<BufReader<File>>, ParserError> {
        if self.doc.is_none() {
            let source = self.source.clone().ok_or(ParserError::UnreadableFile)?;
            let doc = EpubDoc::new(source).map_err(|_| ParserError::UnreadableFile)?;
            self.doc = Some(doc);
        }

        self.doc.as_mut().ok_or(ParserError::UnreadableFile)
    }

    fn metadata(doc: &EpubDoc<BufReader<File>>, property: &str) -> Option<String> {
        doc.mdata(property).map(|item| item.value.clone())
    }

    fn html_to_markdown(html: &str) -> String {
        parse_html(html)
    }

    fn resolve_navpoint_to_spine(
        doc: &EpubDoc<BufReader<File>>,
        content: &std::path::Path,
    ) -> Option<usize> {
        let direct = content.to_path_buf();
        if let Some(idx) = doc.resource_uri_to_chapter(&direct) {
            return Some(idx);
        }

        let content_str = content.to_string_lossy();
        let without_fragment = content_str.split('#').next().unwrap_or_default();
        let normalized = std::path::PathBuf::from(without_fragment);
        if let Some(idx) = doc.resource_uri_to_chapter(&normalized) {
            return Some(idx);
        }

        for (resource_id, resource_item) in &doc.resources {
            let resource_path = resource_item.path.to_string_lossy();
            if resource_path == without_fragment
                || resource_path.ends_with(without_fragment)
                || without_fragment.ends_with(resource_path.as_ref())
            {
                if let Some(idx) = doc.resource_id_to_chapter(resource_id) {
                    return Some(idx);
                }
            }
        }

        None
    }

    fn build_sections(
        points: &[NavPoint],
        doc: &EpubDoc<BufReader<File>>,
        id_counter: &mut usize,
    ) -> Vec<Section> {
        let mut sections = Vec::new();

        for point in points {
            let Some(content_ref) = Self::resolve_navpoint_to_spine(doc, &point.content) else {
                continue;
            };

            *id_counter += 1;
            let id = format!("sec_{:06}", *id_counter);
            let mut title = point.label.trim().to_string();
            if title.is_empty() {
                title = point.content.to_string_lossy().to_string();
            }

            let children = if point.children.is_empty() {
                Vec::new()
            } else {
                Self::build_sections(&point.children, doc, id_counter)
            };

            sections.push(Section {
                id,
                title,
                content_ref,
                children,
            });
        }

        sections
    }

    fn collect_content_refs(sections: &[Section], refs: &mut Vec<usize>) {
        for section in sections {
            refs.push(section.content_ref);
            if !section.children.is_empty() {
                Self::collect_content_refs(&section.children, refs);
            }
        }
    }
}

impl Parser for Epub {
    fn parse(&mut self, src: &str) -> Result<Document, ParserError> {
        let path = Path::new(src);

        if !path.exists() {
            return Err(ParserError::FileDoesNotExist);
        }

        if !path.is_file() {
            return Err(ParserError::UnreadableFile);
        }

        let doc = EpubDoc::new(src).map_err(|_| ParserError::UnreadableFile)?;
        self.source = Some(src.to_string());
        self.doc = Some(doc);
        let toc = self.get_toc().unwrap_or_default();
        self.document = Some(Document {
            title: self.get_title().unwrap_or_default(),
            cover_image: self.get_cover_image().unwrap(),
            subtitle: self.get_subtitle().unwrap_or_default(),
            author: self.get_author().unwrap_or_default(),
            description: self.get_description().unwrap_or_default(),
            content: HashMap::new(),
            language: self.get_language().unwrap_or_default(),
            toc,
        });
        let content = self.get_content_by_chapter().unwrap_or_default();
        if let Some(document) = self.document.as_mut() {
            document.content = content;
        }

        match self.document.clone() {
            Some(d) => Ok(d),
            None => Err(ParserError::UnreadableFile),
        }
    }

    fn get_cover(&mut self) -> Result<String, ParserError> {
        let doc = self.ensure_doc()?;
        let (image_data, _) = doc.get_cover().unwrap_or_default();
        let encoded = BASE64_STANDARD.encode(image_data);
        Ok(encoded)
    }

    fn get_subtitle(&mut self) -> Result<String, ParserError> {
        let doc = self.ensure_doc()?;
        let subtitle = doc.mdata("subtitle");
        match subtitle {
            Some(subt) => Ok(subt.value.to_string()),
            None => Ok("".to_string()),
        }
    }

    fn get_title(&mut self) -> Result<String, ParserError> {
        let doc = self.ensure_doc()?;
        Ok(doc
            .get_title()
            .or_else(|| Self::metadata(doc, "title"))
            .unwrap_or_default())
    }

    fn get_author(&mut self) -> Result<String, ParserError> {
        let doc = self.ensure_doc()?;
        Ok(Self::metadata(doc, "creator").unwrap_or_default())
    }

    fn get_description(&mut self) -> Result<String, ParserError> {
        let doc = self.ensure_doc()?;
        let description_html = Self::metadata(doc, "description").unwrap_or_default();

        let markdown = Self::clean_html(&description_html)?;
        Ok(markdown)
    }

    fn get_publisher(&mut self) -> Result<String, ParserError> {
        let doc = self.ensure_doc()?;
        Ok(Self::metadata(doc, "publisher").unwrap_or_default())
    }

    fn get_language(&mut self) -> Result<String, ParserError> {
        let doc = self.ensure_doc()?;
        Ok(Self::metadata(doc, "language").unwrap_or_default())
    }

    fn get_cover_image(&mut self) -> Result<String, ParserError> {
        let encoded = self.get_cover()?;
        Ok(encoded)
    }

    fn get_toc(&mut self) -> Result<Vec<Section>, ParserError> {
        let doc = self.ensure_doc()?;
        let mut id_counter = 0usize;
        let mut sections = Self::build_sections(&doc.toc, doc, &mut id_counter);

        if sections.is_empty() {
            sections = doc
                .spine
                .iter()
                .enumerate()
                .map(|(content_ref, spine_item)| {
                    id_counter += 1;
                    Section {
                        id: format!("sec_{:06}", id_counter),
                        title: spine_item.idref.clone(),
                        content_ref,
                        children: Vec::new(),
                    }
                })
                .collect();
        }

        Ok(sections)
    }

    fn clean_html(html: &str) -> Result<String, ParserError> {
        Ok(Self::html_to_markdown(html))
    }

    fn get_content_by_chapter(&mut self) -> Result<HashMap<usize, String>, ParserError> {
        let mut content_refs = Vec::new();
        let toc = &self
            .document
            .as_ref()
            .ok_or(ParserError::InvalidContent)?
            .toc;
        Self::collect_content_refs(toc, &mut content_refs);
        content_refs.sort_unstable();
        content_refs.dedup();

        let doc = self.ensure_doc()?;
        let total_spine_items = doc.get_num_chapters();

        if content_refs.is_empty() {
            content_refs = (0..total_spine_items).collect();
        } else if content_refs.first().copied().unwrap_or(0) != 0 {
            content_refs.insert(0, 0);
        }

        let mut content_by_chapter = HashMap::new();

        for (pos, chapter_start) in content_refs.iter().copied().enumerate() {
            let chapter_end = content_refs
                .get(pos + 1)
                .copied()
                .unwrap_or(total_spine_items);

            let mut markdown_parts = Vec::new();
            for spine_index in chapter_start..chapter_end {
                if !doc.set_current_chapter(spine_index) {
                    continue;
                }

                if let Some((content, _)) = doc.get_current() {
                    let html = String::from_utf8_lossy(&content).into_owned();
                    let markdown = Self::clean_html(&html)?;
                    if !markdown.trim().is_empty() {
                        markdown_parts.push(markdown);
                    }
                }
            }

            if !markdown_parts.is_empty() {
                content_by_chapter.insert(chapter_start, markdown_parts.join("\n\n"));
            }
        }

        Ok(content_by_chapter)
    }
}
