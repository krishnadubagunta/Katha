use base64::prelude::*;
use kuchiki::NodeRef;
use kuchiki::traits::*;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::ContentBlock;
use crate::ContentKind;
use crate::Document;
use crate::Parser;
use crate::Section;
use crate::error::ParserError;
use epub::doc::EpubDoc;
use epub::doc::NavPoint;

#[derive(Debug, Clone)]
struct NavTarget {
    content_ref: usize,
    spine_index: usize,
    fragment: Option<String>,
    title: String,
}

#[derive(Debug, Clone)]
struct TextBlock {
    kind: ContentKind,
    content: String,
    level: Option<u8>,
    anchor_ids: Vec<String>,
    heading_text: Option<String>,
}

/// EPUB parser adapter.
///
/// This type implements [`crate::Parser`] and converts EPUB metadata and chapter
/// content into the normalized [`crate::Document`] schema.
pub struct Epub {
    source: Option<String>,
    doc: Option<EpubDoc<BufReader<File>>>,
    document: Option<Document>,
    nav_targets: Vec<NavTarget>,
}

impl Epub {
    /// Creates a new EPUB parser instance with no bound source.
    pub fn new() -> Self {
        Self {
            source: None,
            doc: None,
            document: None,
            nav_targets: Vec::new(),
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

    fn html_to_text(html: &str) -> String {
        let document = kuchiki::parse_html().one(html);
        Self::clean_dom(&document);

        let root = document
            .select_first("body")
            .ok()
            .map(|node| node.as_node().clone())
            .unwrap_or(document);

        Self::normalize_text(&root.text_contents())
    }

    fn whitespace_regex() -> &'static Regex {
        static REGEX: OnceLock<Regex> = OnceLock::new();
        REGEX.get_or_init(|| Regex::new(r"[ \t\r\n]+").expect("valid whitespace regex"))
    }

    fn blank_line_regex() -> &'static Regex {
        static REGEX: OnceLock<Regex> = OnceLock::new();
        REGEX.get_or_init(|| Regex::new(r"\n{3,}").expect("valid blank-line regex"))
    }

    fn boilerplate_regex() -> &'static Regex {
        static REGEX: OnceLock<Regex> = OnceLock::new();
        REGEX.get_or_init(|| {
            Regex::new(
                r"(?i)\b(copyright|all rights reserved|dedication|acknowledg(e)?ments?|contents?|table of contents|navigation)\b",
            )
            .expect("valid boilerplate regex")
        })
    }

    fn normalize_inline_text(text: &str) -> String {
        Self::whitespace_regex()
            .replace_all(text.trim(), " ")
            .trim()
            .to_string()
    }

    fn normalize_text(text: &str) -> String {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut lines = Vec::new();

        for line in normalized.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                lines.push(String::new());
                continue;
            }

            lines.push(Self::normalize_inline_text(trimmed));
        }

        let joined = lines.join("\n");
        Self::blank_line_regex()
            .replace_all(joined.trim(), "\n\n")
            .to_string()
    }

    fn is_epub_attr(value: &str, needle: &str) -> bool {
        value
            .split_whitespace()
            .any(|item| item.eq_ignore_ascii_case(needle))
    }

    fn has_noise_marker(node: &NodeRef) -> bool {
        let Some(element) = node.as_element() else {
            return false;
        };
        let attrs = element.attributes.borrow();

        if let Some(role) = attrs.get("role") {
            let role = role.to_ascii_lowercase();
            if role == "doc-toc" || role == "doc-pagebreak" {
                return true;
            }
        }

        if let Some(class_attr) = attrs.get("class") {
            let class_attr = class_attr.to_ascii_lowercase();
            if class_attr.contains("pagebreak")
                || class_attr.split_whitespace().any(|part| part == "page")
            {
                return true;
            }
        }

        attrs.map.iter().any(|(name, value)| {
            name.local.as_ref() == "type"
                && (Self::is_epub_attr(value.value.as_ref(), "toc")
                    || Self::is_epub_attr(value.value.as_ref(), "pagebreak")
                    || Self::is_epub_attr(value.value.as_ref(), "noteref")
                    || Self::is_epub_attr(value.value.as_ref(), "footnote"))
        })
    }

    fn remove_matching_nodes<F>(root: &NodeRef, predicate: F)
    where
        F: Fn(&NodeRef) -> bool,
    {
        let nodes: Vec<NodeRef> = root.descendants().collect();
        for node in nodes {
            if predicate(&node) {
                node.detach();
            }
        }
    }

    fn list_is_navigational(node: &NodeRef) -> bool {
        let Some(element) = node.as_element() else {
            return false;
        };
        let tag = element.name.local.as_ref();
        if tag != "ul" && tag != "ol" {
            return false;
        }

        let mut items = 0usize;
        let mut short_anchor_items = 0usize;
        for child in node.children() {
            let Some(li) = child.as_element() else {
                continue;
            };
            if li.name.local.as_ref() != "li" {
                continue;
            }

            items += 1;
            let element_children: Vec<NodeRef> = child
                .children()
                .filter(|n| n.as_element().is_some())
                .collect();
            let text = Self::normalize_inline_text(&child.text_contents());
            let has_single_anchor_child = element_children.len() == 1
                && element_children[0]
                    .as_element()
                    .map(|el| el.name.local.as_ref() == "a")
                    .unwrap_or(false);

            if has_single_anchor_child && text.split_whitespace().count() <= 8 {
                short_anchor_items += 1;
            }
        }

        items > 0 && short_anchor_items * 2 >= items
    }

    fn clean_dom(document: &NodeRef) {
        Self::remove_matching_nodes(document, |node| {
            let Some(element) = node.as_element() else {
                return false;
            };
            matches!(
                element.name.local.as_ref(),
                "nav" | "header" | "footer" | "aside"
            ) || Self::has_noise_marker(node)
        });

        Self::remove_matching_nodes(document, Self::list_is_navigational);
    }

    fn anchor_ids_for_node(node: &NodeRef) -> Vec<String> {
        let mut ids = HashSet::new();

        if let Some(element) = node.as_element() {
            let attrs = element.attributes.borrow();
            if let Some(id) = attrs.get("id") {
                ids.insert(id.to_string());
            }
            if let Some(name) = attrs.get("name") {
                ids.insert(name.to_string());
            }
        }

        for descendant in node.descendants() {
            let Some(element) = descendant.as_element() else {
                continue;
            };
            let attrs = element.attributes.borrow();
            if let Some(id) = attrs.get("id") {
                ids.insert(id.to_string());
            }
            if let Some(name) = attrs.get("name") {
                ids.insert(name.to_string());
            }
        }

        ids.into_iter().collect()
    }

    fn block_from_node(node: &NodeRef, tag: &str) -> Option<TextBlock> {
        let text = if matches!(tag, "ul" | "ol") {
            let items = node
                .children()
                .filter_map(|child| {
                    let element = child.as_element()?;
                    (element.name.local.as_ref() == "li")
                        .then(|| Self::normalize_inline_text(&child.text_contents()))
                })
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>();

            if items.is_empty() {
                Self::normalize_inline_text(&node.text_contents())
            } else {
                items.join("\n")
            }
        } else {
            Self::normalize_inline_text(&node.text_contents())
        };
        if text.is_empty() {
            return None;
        }

        let (kind, level) = match tag {
            "h1" => (ContentKind::Heading, Some(1)),
            "h2" => (ContentKind::Heading, Some(2)),
            "h3" => (ContentKind::Heading, Some(3)),
            "ul" | "ol" => (ContentKind::List, None),
            "blockquote" => (ContentKind::Quote, None),
            _ => (ContentKind::Paragraph, None),
        };

        let heading_text = matches!(tag, "h1" | "h2" | "h3").then_some(text.clone());
        let content = heading_text.clone().unwrap_or_else(|| text.clone());

        Some(TextBlock {
            kind,
            content,
            level,
            anchor_ids: Self::anchor_ids_for_node(node),
            heading_text,
        })
    }

    fn extract_clean_blocks(html: &str) -> Vec<TextBlock> {
        let document = kuchiki::parse_html().one(html);
        Self::clean_dom(&document);

        let root = document
            .select_first("body")
            .ok()
            .map(|node| node.as_node().clone())
            .unwrap_or(document);

        let mut blocks = Vec::new();
        for css_match in root
            .select("h1, h2, h3, p, blockquote, ul, ol")
            .expect("valid selector")
        {
            let node = css_match.as_node().clone();
            if node
                .ancestors()
                .any(|ancestor| Self::has_noise_marker(&ancestor))
            {
                continue;
            }

            let tag = css_match.name.local.as_ref();
            if let Some(block) = Self::block_from_node(&node, tag) {
                blocks.push(block);
            }
        }

        blocks
    }

    fn blocks_to_plain_text(blocks: &[TextBlock]) -> String {
        let joined = blocks
            .iter()
            .map(|block| block.content.as_str())
            .filter(|content| !content.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        Self::normalize_text(&joined)
    }

    fn blocks_to_content(blocks: &[TextBlock]) -> Vec<ContentBlock> {
        let content = blocks
            .iter()
            .filter_map(|block| {
                let content = if block.kind == ContentKind::List {
                    None
                } else {
                    Some(Self::normalize_inline_text(&block.content))
                };

                let items = if block.kind == ContentKind::List {
                    block
                        .content
                        .lines()
                        .map(Self::normalize_inline_text)
                        .filter(|item| !item.is_empty())
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };

                if content.as_deref().unwrap_or_default().is_empty() && items.is_empty() {
                    return None;
                }

                Some(ContentBlock {
                    kind: block.kind.clone(),
                    content,
                    items,
                    level: block.level,
                })
            })
            .collect::<Vec<_>>();

        Self::merge_short_paragraph_runs(content)
    }

    fn word_count(blocks: &[ContentBlock]) -> usize {
        blocks
            .iter()
            .map(|block| {
                block
                    .content
                    .as_deref()
                    .unwrap_or_default()
                    .split_whitespace()
                    .count()
                    + block
                        .items
                        .iter()
                        .map(|item| item.split_whitespace().count())
                        .sum::<usize>()
            })
            .sum()
    }

    fn is_probably_boilerplate(title: &str, blocks: &[ContentBlock]) -> bool {
        let combined = format!(
            "{title}\n{}",
            blocks
                .iter()
                .flat_map(|block| {
                    block
                        .content
                        .iter()
                        .map(String::as_str)
                        .chain(block.items.iter().map(String::as_str))
                })
                .collect::<Vec<_>>()
                .join("\n")
        );
        Self::boilerplate_regex().is_match(&combined)
    }

    fn canonicalize_fragment(fragment: &str) -> String {
        fragment.trim_start_matches('#').trim().to_string()
    }

    fn heading_matches_title(heading: &str, title: &str) -> bool {
        let heading = Self::normalize_inline_text(heading).to_ascii_lowercase();
        let title = Self::normalize_inline_text(title).to_ascii_lowercase();
        !heading.is_empty() && heading == title
    }

    fn resolve_navpoint_to_target(
        doc: &EpubDoc<BufReader<File>>,
        content: &Path,
    ) -> Option<(usize, Option<String>)> {
        let direct = content.to_path_buf();
        if let Some(idx) = doc.resource_uri_to_chapter(&direct) {
            return Some((idx, None));
        }

        let content_str = content.to_string_lossy();
        let mut parts = content_str.split('#');
        let without_fragment = parts.next().unwrap_or_default();
        let fragment = parts
            .next()
            .map(Self::canonicalize_fragment)
            .filter(|item| !item.is_empty());

        let normalized = PathBuf::from(without_fragment);
        if let Some(idx) = doc.resource_uri_to_chapter(&normalized) {
            return Some((idx, fragment));
        }

        for (resource_id, resource_item) in &doc.resources {
            let resource_path = resource_item.path.to_string_lossy();
            if resource_path == without_fragment
                || resource_path.ends_with(without_fragment)
                || without_fragment.ends_with(resource_path.as_ref())
            {
                if let Some(idx) = doc.resource_id_to_chapter(resource_id) {
                    return Some((idx, fragment));
                }
            }
        }

        None
    }

    fn build_sections(
        points: &[NavPoint],
        doc: &EpubDoc<BufReader<File>>,
        id_counter: &mut usize,
        content_ref_counter: &mut usize,
        nav_targets: &mut Vec<NavTarget>,
    ) -> Vec<Section> {
        let mut sections = Vec::new();

        for point in points {
            let Some((spine_index, fragment)) =
                Self::resolve_navpoint_to_target(doc, &point.content)
            else {
                continue;
            };

            *id_counter += 1;
            *content_ref_counter += 1;

            let id = format!("sec_{:06}", *id_counter);
            let mut title = point.label.trim().to_string();
            if title.is_empty() {
                title = point.content.to_string_lossy().to_string();
            }

            let content_ref = *content_ref_counter;
            nav_targets.push(NavTarget {
                content_ref,
                spine_index,
                fragment,
                title: title.clone(),
            });

            let children = if point.children.is_empty() {
                Vec::new()
            } else {
                Self::build_sections(
                    &point.children,
                    doc,
                    id_counter,
                    content_ref_counter,
                    nav_targets,
                )
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

    fn remove_repeated_lines(content_by_chapter: &mut HashMap<usize, Vec<ContentBlock>>) {
        if content_by_chapter.len() < 3 {
            return;
        }

        let mut appearances: HashMap<String, usize> = HashMap::new();
        for blocks in content_by_chapter.values() {
            let unique_lines: HashSet<String> = blocks
                .iter()
                .filter(|block| {
                    !matches!(
                        block.kind,
                        ContentKind::Heading | ContentKind::Quote | ContentKind::List
                    )
                })
                .map(|block| {
                    Self::normalize_inline_text(block.content.as_deref().unwrap_or_default())
                })
                .filter(|line| !line.is_empty())
                .filter(|line| line.split_whitespace().count() <= 12)
                .collect();

            for line in unique_lines {
                *appearances.entry(line).or_insert(0) += 1;
            }
        }

        let threshold = content_by_chapter.len().div_ceil(2);
        let repeated: HashSet<String> = appearances
            .into_iter()
            .filter_map(|(line, count)| (count >= threshold).then_some(line))
            .collect();

        if repeated.is_empty() {
            return;
        }

        for blocks in content_by_chapter.values_mut() {
            blocks.retain(|block| {
                if matches!(
                    block.kind,
                    ContentKind::Heading | ContentKind::Quote | ContentKind::List
                ) {
                    return true;
                }

                let normalized =
                    Self::normalize_inline_text(block.content.as_deref().unwrap_or_default());
                normalized.is_empty() || !repeated.contains(&normalized)
            });

            for block in blocks.iter_mut() {
                if let Some(content) = block.content.as_mut() {
                    *content = Self::normalize_inline_text(content);
                }
            }
        }
    }

    fn find_block_index(blocks: &[TextBlock], target: &NavTarget) -> Option<usize> {
        if let Some(fragment) = target.fragment.as_ref() {
            let canonical = Self::canonicalize_fragment(fragment);
            if let Some(index) = blocks.iter().position(|block| {
                block
                    .anchor_ids
                    .iter()
                    .any(|id| Self::canonicalize_fragment(id) == canonical)
            }) {
                return Some(index);
            }
        }

        blocks.iter().position(|block| {
            block
                .heading_text
                .as_ref()
                .map(|heading| Self::heading_matches_title(heading, &target.title))
                .unwrap_or(false)
        })
    }

    fn paragraph_word_count(block: &ContentBlock) -> usize {
        block
            .content
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .count()
    }

    fn ends_sentence(content: &str) -> bool {
        let trimmed = content.trim_end();
        trimmed.ends_with('.') || trimmed.ends_with('!') || trimmed.ends_with('?')
    }

    fn should_merge_paragraphs(current: &ContentBlock, next: &ContentBlock) -> bool {
        if current.kind != ContentKind::Paragraph || next.kind != ContentKind::Paragraph {
            return false;
        }

        let current_words = Self::paragraph_word_count(current);
        let next_words = Self::paragraph_word_count(next);

        (!Self::ends_sentence(current.content.as_deref().unwrap_or_default())
            && (current_words <= 12 || next_words <= 12))
            || (current_words <= 6 && next_words <= 6)
    }

    fn merge_short_paragraph_runs(blocks: Vec<ContentBlock>) -> Vec<ContentBlock> {
        let mut merged = Vec::new();

        for block in blocks {
            if let Some(previous) = merged.last_mut() {
                if Self::should_merge_paragraphs(previous, &block) {
                    let merged_text = format!(
                        "{} {}",
                        previous.content.as_deref().unwrap_or_default().trim_end(),
                        block.content.as_deref().unwrap_or_default().trim_start()
                    );
                    previous.content = Some(Self::normalize_inline_text(&merged_text));
                    continue;
                }
            }

            merged.push(block);
        }

        merged
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
        self.nav_targets.clear();

        let toc = self.get_toc().unwrap_or_default();
        self.document = Some(Document {
            title: self.get_title().unwrap_or_default(),
            cover_image: self.get_cover_image().unwrap_or_default(),
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

        self.document.clone().ok_or(ParserError::UnreadableFile)
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
            None => Ok(String::new()),
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
        self.get_cover()
    }

    fn get_toc(&mut self) -> Result<Vec<Section>, ParserError> {
        let doc = self.ensure_doc()?;
        let mut id_counter = 0usize;
        let mut content_ref_counter = 0usize;
        let mut nav_targets = Vec::new();
        let mut sections = Self::build_sections(
            &doc.toc,
            doc,
            &mut id_counter,
            &mut content_ref_counter,
            &mut nav_targets,
        );

        if sections.is_empty() {
            sections = doc
                .spine
                .iter()
                .enumerate()
                .map(|(spine_index, spine_item)| {
                    id_counter += 1;
                    content_ref_counter += 1;
                    let content_ref = content_ref_counter;
                    nav_targets.push(NavTarget {
                        content_ref,
                        spine_index,
                        fragment: None,
                        title: spine_item.idref.clone(),
                    });
                    Section {
                        id: format!("sec_{:06}", id_counter),
                        title: spine_item.idref.clone(),
                        content_ref,
                        children: Vec::new(),
                    }
                })
                .collect();
        }

        self.nav_targets = nav_targets;
        Ok(sections)
    }

    fn clean_html(html: &str) -> Result<String, ParserError> {
        let blocks = Self::extract_clean_blocks(html);
        if blocks.is_empty() {
            return Ok(Self::html_to_text(html));
        }
        Ok(Self::blocks_to_plain_text(&blocks))
    }

    fn get_content_by_chapter(&mut self) -> Result<HashMap<usize, Vec<ContentBlock>>, ParserError> {
        if self.nav_targets.is_empty() {
            let _ = self.get_toc()?;
        }

        let nav_targets = self.nav_targets.clone();
        let doc = self.ensure_doc()?;
        let total_spine_items = doc.get_num_chapters();
        let mut grouped_targets: HashMap<usize, Vec<NavTarget>> = HashMap::new();

        for target in nav_targets {
            grouped_targets
                .entry(target.spine_index)
                .or_default()
                .push(target);
        }

        if grouped_targets.is_empty() {
            for spine_index in 0..total_spine_items {
                grouped_targets.insert(
                    spine_index,
                    vec![NavTarget {
                        content_ref: spine_index,
                        spine_index,
                        fragment: None,
                        title: String::new(),
                    }],
                );
            }
        }

        let mut content_by_chapter: HashMap<usize, Vec<ContentBlock>> = HashMap::new();

        for spine_index in 0..total_spine_items {
            let Some(targets) = grouped_targets.get(&spine_index) else {
                continue;
            };

            if !doc.set_current_chapter(spine_index) {
                continue;
            }

            let Some((content, _)) = doc.get_current() else {
                continue;
            };

            let html = String::from_utf8_lossy(&content).into_owned();
            let blocks = Self::extract_clean_blocks(&html);
            if blocks.is_empty() {
                continue;
            }

            let full_content = Self::blocks_to_content(&blocks);
            if Self::word_count(&full_content) < 200
                && Self::is_probably_boilerplate(
                    &targets
                        .first()
                        .map(|target| target.title.as_str())
                        .unwrap_or_default(),
                    &full_content,
                )
            {
                continue;
            }

            let mut indexed_targets = targets
                .iter()
                .map(|target| (Self::find_block_index(&blocks, target), target))
                .collect::<Vec<_>>();

            indexed_targets.sort_by_key(|(position, _)| position.unwrap_or(usize::MAX));

            let any_resolved = indexed_targets
                .iter()
                .any(|(position, _)| position.is_some());
            if !any_resolved || indexed_targets.len() == 1 {
                if let Some(target) = targets.first() {
                    content_by_chapter.insert(target.content_ref, full_content);
                }
                continue;
            }

            for (idx, (start, target)) in indexed_targets.iter().enumerate() {
                let start = start.unwrap_or(0);
                let end = indexed_targets
                    .iter()
                    .skip(idx + 1)
                    .find_map(|(position, _)| *position)
                    .unwrap_or(blocks.len());

                if start >= end || start >= blocks.len() {
                    continue;
                }

                let content = Self::blocks_to_content(&blocks[start..end]);
                if content.is_empty() {
                    continue;
                }

                content_by_chapter.insert(target.content_ref, content);
            }
        }

        Self::remove_repeated_lines(&mut content_by_chapter);
        Ok(content_by_chapter)
    }
}

#[cfg(test)]
mod tests {
    use super::Epub;
    use crate::Parser;

    #[test]
    fn clean_html_removes_navigation_footnotes_and_pagebreaks() {
        let html = r##"
            <html>
                <body>
                    <nav>Table of Contents</nav>
                    <header>Book Header</header>
                    <h1 id="ch1">Chapter 1</h1>
                    <p>The <em>morning</em> was unusually quiet.</p>
                    <a epub:type="noteref">1</a>
                    <aside epub:type="footnote">Footnote content</aside>
                    <span epub:type="pagebreak">23</span>
                    <footer>Next Chapter</footer>
                </body>
            </html>
        "##;

        let markdown = Epub::clean_html(html).expect("html should clean");
        assert!(markdown.contains("Chapter 1"));
        assert!(markdown.contains("The morning was unusually quiet."));
        assert!(!markdown.contains("Table of Contents"));
        assert!(!markdown.contains("Footnote content"));
        assert!(!markdown.contains("Next Chapter"));
        assert!(!markdown.contains("23"));
    }

    #[test]
    fn clean_html_drops_navigational_lists() {
        let html = r##"
            <html>
                <body>
                    <ul>
                        <li><a href="#c1">Chapter 1</a></li>
                        <li><a href="#c2">Chapter 2</a></li>
                    </ul>
                    <h1 id="c1">Chapter 1</h1>
                    <p>Actual content lives here.</p>
                </body>
            </html>
        "##;

        let markdown = Epub::clean_html(html).expect("html should clean");
        assert!(markdown.contains("Actual content lives here."));
        assert!(!markdown.contains("Chapter 2"));
    }

    #[test]
    fn merges_short_paragraph_lines_into_one_paragraph() {
        let html = r##"
            <html>
                <body>
                    <p>Courage to change the things</p>
                    <p>which should be changed,</p>
                    <p>and the Wisdom to distinguish</p>
                    <p>the one from the other.</p>
                </body>
            </html>
        "##;

        let blocks = Epub::extract_clean_blocks(html);
        let content = Epub::blocks_to_content(&blocks);

        assert_eq!(content.len(), 1);
        assert_eq!(content[0].kind, crate::ContentKind::Paragraph);
        assert_eq!(
            content[0].content.as_deref(),
            Some(
                "Courage to change the things which should be changed, and the Wisdom to distinguish the one from the other."
            )
        );
    }

    #[test]
    fn extracts_lists_as_list_blocks() {
        let html = r##"
            <html>
                <body>
                    <h2>Habits</h2>
                    <ul>
                        <li>Move naturally</li>
                        <li>Eat until 80% full</li>
                    </ul>
                </body>
            </html>
        "##;

        let blocks = Epub::extract_clean_blocks(html);
        let content = Epub::blocks_to_content(&blocks);

        assert_eq!(content.len(), 2);
        assert_eq!(content[0].kind, crate::ContentKind::Heading);
        assert_eq!(content[1].kind, crate::ContentKind::List);
        assert_eq!(content[1].content, None);
        assert_eq!(
            content[1].items,
            vec!["Move naturally", "Eat until 80% full"]
        );
    }
}
