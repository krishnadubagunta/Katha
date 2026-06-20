use std::collections::HashSet;
use std::sync::OnceLock;

use kuchiki::NodeRef;
use kuchiki::traits::*;
use regex::Regex;

use crate::{ContentBlock, ContentKind};

#[derive(Debug, Clone)]
pub(crate) struct TextBlock {
    pub(crate) kind: ContentKind,
    pub(crate) content: String,
    pub(crate) level: Option<u8>,
    pub(crate) anchor_ids: Vec<String>,
    pub(crate) heading_text: Option<String>,
}

pub(crate) fn whitespace_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"[ \t\r\n]+").expect("valid whitespace regex"))
}

pub(crate) fn blank_line_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\n{3,}").expect("valid blank-line regex"))
}

pub(crate) fn boilerplate_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)\b(copyright|all rights reserved|dedication|acknowledg(e)?ments?|contents?|table of contents|navigation)\b",
        )
        .expect("valid boilerplate regex")
    })
}

pub(crate) fn normalize_inline_text(text: &str) -> String {
    whitespace_regex()
        .replace_all(text.trim(), " ")
        .trim()
        .to_string()
}

pub(crate) fn normalize_text(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = Vec::new();

    for line in normalized.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            lines.push(String::new());
            continue;
        }

        lines.push(normalize_inline_text(trimmed));
    }

    let joined = lines.join("\n");
    blank_line_regex()
        .replace_all(joined.trim(), "\n\n")
        .to_string()
}

pub(crate) fn is_epub_attr(value: &str, needle: &str) -> bool {
    value
        .split_whitespace()
        .any(|item| item.eq_ignore_ascii_case(needle))
}

pub(crate) fn has_noise_marker(node: &NodeRef) -> bool {
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
        if class_attr.contains("pagebreak") {
            return true;
        }
        // `class="page"` is an EPUB pagebreak marker only when the element has no
        // substantive content (e.g. <span class="page">23</span>).  pdf_oxide uses
        // `class="page"` on block containers that hold real chapter content, so we
        // must not treat those as noise.
        if class_attr.split_whitespace().any(|part| part == "page")
            && !has_substantive_content(node)
        {
            return true;
        }
    }

    attrs.map.iter().any(|(name, value)| {
        name.local.as_ref() == "type"
            && (is_epub_attr(value.value.as_ref(), "toc")
                || is_epub_attr(value.value.as_ref(), "pagebreak")
                || is_epub_attr(value.value.as_ref(), "noteref")
                || is_epub_attr(value.value.as_ref(), "footnote"))
    })
}

pub(crate) fn remove_matching_nodes<F>(root: &NodeRef, predicate: F)
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

pub(crate) fn list_is_navigational(node: &NodeRef) -> bool {
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
        let text = normalize_inline_text(&child.text_contents());
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

pub(crate) fn has_substantive_content(node: &NodeRef) -> bool {
    node.descendants().any(|d| {
        let Some(el) = d.as_element() else {
            return false;
        };
        matches!(
            el.name.local.as_ref(),
            "p" | "h1" | "h2" | "h3" | "blockquote" | "section" | "article" | "main"
        )
    })
}

pub(crate) fn unwrap_node(node: &NodeRef) {
    let children: Vec<NodeRef> = node.children().collect();
    for child in children {
        node.insert_before(child);
    }
    node.detach();
}

pub(crate) fn clean_dom(document: &NodeRef) {
    // First pass: explicit noise markers (doc-toc, pagebreaks, footnotes, etc.)
    remove_matching_nodes(document, has_noise_marker);

    // Second pass: navigational landmarks. Detach if they hold only navigation;
    // unwrap (promote children) if they hold real content — this handles the
    // html5ever misparse of self-closing <header/> / <footer/> in XHTML EPUBs,
    // where the entire chapter becomes a descendant of the unclosed landmark.
    let landmarks: Vec<NodeRef> = document
        .descendants()
        .filter(|n| {
            n.as_element()
                .map(|el| {
                    matches!(
                        el.name.local.as_ref(),
                        "nav" | "header" | "footer" | "aside"
                    )
                })
                .unwrap_or(false)
        })
        .collect();
    for node in landmarks {
        if has_substantive_content(&node) {
            unwrap_node(&node);
        } else {
            node.detach();
        }
    }

    // Third pass: navigational lists (table-of-contents style)
    remove_matching_nodes(document, list_is_navigational);
}

pub(crate) fn anchor_ids_for_node(node: &NodeRef) -> Vec<String> {
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

pub(crate) fn block_from_node(node: &NodeRef, tag: &str) -> Option<TextBlock> {
    let text = if matches!(tag, "ul" | "ol") {
        let items = node
            .children()
            .filter_map(|child| {
                let element = child.as_element()?;
                (element.name.local.as_ref() == "li")
                    .then(|| normalize_inline_text(&child.text_contents()))
            })
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>();

        if items.is_empty() {
            normalize_inline_text(&node.text_contents())
        } else {
            items.join("\n")
        }
    } else {
        normalize_inline_text(&node.text_contents())
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
        anchor_ids: anchor_ids_for_node(node),
        heading_text,
    })
}

pub(crate) fn extract_clean_blocks(html: &str) -> Vec<TextBlock> {
    let document = kuchiki::parse_html().one(html);
    clean_dom(&document);

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
            .any(|ancestor| has_noise_marker(&ancestor))
        {
            continue;
        }

        let tag = css_match.name.local.as_ref();
        if let Some(block) = block_from_node(&node, tag) {
            blocks.push(block);
        }
    }

    blocks
}

pub(crate) fn html_to_text(html: &str) -> String {
    let document = kuchiki::parse_html().one(html);
    clean_dom(&document);

    let root = document
        .select_first("body")
        .ok()
        .map(|node| node.as_node().clone())
        .unwrap_or(document);

    normalize_text(&root.text_contents())
}

pub(crate) fn blocks_to_plain_text(blocks: &[TextBlock]) -> String {
    let joined = blocks
        .iter()
        .map(|block| block.content.as_str())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    normalize_text(&joined)
}

pub(crate) fn paragraph_word_count(block: &ContentBlock) -> usize {
    block
        .content
        .as_deref()
        .unwrap_or_default()
        .split_whitespace()
        .count()
}

pub(crate) fn ends_sentence(content: &str) -> bool {
    let trimmed = content.trim_end();
    trimmed.ends_with('.') || trimmed.ends_with('!') || trimmed.ends_with('?')
}

pub(crate) fn should_merge_paragraphs(current: &ContentBlock, next: &ContentBlock) -> bool {
    if current.kind != ContentKind::Paragraph || next.kind != ContentKind::Paragraph {
        return false;
    }

    let current_words = paragraph_word_count(current);
    let next_words = paragraph_word_count(next);

    (!ends_sentence(current.content.as_deref().unwrap_or_default())
        && (current_words <= 12 || next_words <= 12))
        || (current_words <= 6 && next_words <= 6)
}

pub(crate) fn merge_short_paragraph_runs(blocks: Vec<ContentBlock>) -> Vec<ContentBlock> {
    let mut merged = Vec::new();

    for block in blocks {
        if let Some(previous) = merged.last_mut() {
            if should_merge_paragraphs(previous, &block) {
                let merged_text = format!(
                    "{} {}",
                    previous.content.as_deref().unwrap_or_default().trim_end(),
                    block.content.as_deref().unwrap_or_default().trim_start()
                );
                previous.content = Some(normalize_inline_text(&merged_text));
                continue;
            }
        }

        merged.push(block);
    }

    merged
}

pub(crate) fn blocks_to_content(blocks: &[TextBlock]) -> Vec<ContentBlock> {
    let content = blocks
        .iter()
        .filter_map(|block| {
            let content = if block.kind == ContentKind::List {
                None
            } else {
                Some(normalize_inline_text(&block.content))
            };

            let items = if block.kind == ContentKind::List {
                block
                    .content
                    .lines()
                    .map(normalize_inline_text)
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

    merge_short_paragraph_runs(content)
}
