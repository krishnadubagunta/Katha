use base64::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use crate::html::{
    blocks_to_content, blocks_to_plain_text, boilerplate_regex, extract_clean_blocks, html_to_text,
    normalize_inline_text, TextBlock,
};
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
        boilerplate_regex().is_match(&combined)
    }

    fn canonicalize_fragment(fragment: &str) -> String {
        fragment.trim_start_matches('#').trim().to_string()
    }

    fn heading_matches_title(heading: &str, title: &str) -> bool {
        let heading = normalize_inline_text(heading).to_ascii_lowercase();
        let title = normalize_inline_text(title).to_ascii_lowercase();
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
                    normalize_inline_text(block.content.as_deref().unwrap_or_default())
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
                    normalize_inline_text(block.content.as_deref().unwrap_or_default());
                normalized.is_empty() || !repeated.contains(&normalized)
            });

            for block in blocks.iter_mut() {
                if let Some(content) = block.content.as_mut() {
                    *content = normalize_inline_text(content);
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
        let blocks = extract_clean_blocks(html);
        if blocks.is_empty() {
            return Ok(html_to_text(html));
        }
        Ok(blocks_to_plain_text(&blocks))
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
            let blocks = extract_clean_blocks(&html);
            if blocks.is_empty() {
                continue;
            }

            let full_content = blocks_to_content(&blocks);
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

                let content = blocks_to_content(&blocks[start..end]);
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
    use crate::html::{blocks_to_content, extract_clean_blocks};
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

        let blocks = extract_clean_blocks(html);
        let content = blocks_to_content(&blocks);

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
    fn extract_clean_blocks_survives_self_closing_xhtml_header() {
        // DocBook-generated EPUBs (e.g. Project Gutenberg / epubbooks Gatsby)
        // wrap each chapter in <body><header/><section class="chapter">…</section><footer/></body>.
        // html5ever parses <header/> as <header> (the `/` is ignored because header is
        // not a void element), so the entire chapter becomes a descendant of the
        // unclosed header. The old clean_dom detached header outright, wiping the chapter.
        let html = r##"
            <!DOCTYPE html>
            <html>
                <body>
                    <header/>
                    <section class="chapter">
                        <div class="titlepage"><h1>Chapter 1</h1></div>
                        <p>In my younger and more vulnerable years my father gave me some advice.</p>
                        <p>He didn't say any more but we've always been unusually communicative.</p>
                    </section>
                    <footer/>
                </body>
            </html>
        "##;

        let blocks = extract_clean_blocks(html);
        assert!(
            blocks
                .iter()
                .any(|b| b.kind == crate::ContentKind::Heading && b.content == "Chapter 1"),
            "heading should survive misparsed <header/>, got {blocks:?}"
        );
        let paragraphs = blocks
            .iter()
            .filter(|b| b.kind == crate::ContentKind::Paragraph)
            .count();
        assert!(
            paragraphs >= 2,
            "paragraphs inside the chapter should survive misparsed <header/>, got {blocks:?}"
        );
    }

    #[test]
    fn clean_dom_still_strips_true_navigational_header() {
        // A real navigational <header>/<nav> holds only anchors — it must still be stripped.
        let html = r##"
            <html>
                <body>
                    <header><nav><ul><li><a href="#a">A</a></li><li><a href="#b">B</a></li></ul></nav></header>
                    <p>Real body text here.</p>
                </body>
            </html>
        "##;

        let blocks = extract_clean_blocks(html);
        assert_eq!(blocks.len(), 1, "only the body paragraph should remain, got {blocks:?}");
        assert_eq!(blocks[0].kind, crate::ContentKind::Paragraph);
        assert_eq!(blocks[0].content, "Real body text here.");
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

        let blocks = extract_clean_blocks(html);
        let content = blocks_to_content(&blocks);

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
