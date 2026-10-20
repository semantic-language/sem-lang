//! Adapter from the lossless `.sem` document tree to semantic analysis lines.
//!
//! One inline property intentionally becomes two semantic events on the same
//! physical line: the package-resolved property and its value.  This keeps
//! diagnostics tied to authored text while making `Purpose: value` equivalent
//! to the indented form.

use crate::TextRange;
use sem_lang_vocab::document::{BlockKind, Document, DocumentError};

pub(crate) struct SemanticLine {
    pub physical_line: usize,
    pub raw: String,
    pub semantic: String,
}

pub(crate) fn semantic_lines(source: &str) -> Result<Vec<SemanticLine>, DocumentError> {
    let document = Document::parse(source)?;
    let mut lines = Vec::new();
    for block in document.blocks {
        let mut push = |semantic: String| {
            lines.push(SemanticLine {
                physical_line: block.line,
                raw: block.raw.clone(),
                semantic,
            });
        };
        match block.kind {
            BlockKind::Blank
            | BlockKind::Comment(_)
            | BlockKind::CodeFence { .. }
            | BlockKind::Code(_) => {}
            BlockKind::Heading { text, .. } => push(format!("# {text}")),
            BlockKind::Property { name, value } => {
                push(format!("{name}:"));
                if let Some(value) = value {
                    push(value);
                }
            }
            BlockKind::ListItem { text, .. }
            | BlockKind::Quote(text)
            | BlockKind::Paragraph(text) => push(text),
            BlockKind::TableRow(cells) => push(cells.join(" | ")),
        }
    }
    Ok(lines)
}

pub(crate) fn line_range(line: u32, raw: &str) -> TextRange {
    TextRange {
        start_line: line,
        start_character: 0,
        end_line: line,
        end_character: raw.chars().count() as u32,
    }
}

pub(crate) fn is_narrative_property(term_label: &str) -> bool {
    matches!(term_label, "DocumentPurpose" | "CompletionCriteria")
}

pub(crate) fn uncovered_fragments(line: &str, covered: &[bool]) -> Vec<(usize, usize, String)> {
    let mut fragments = Vec::new();
    let mut start = None;
    for (offset, character) in line.char_indices() {
        let end = offset + character.len_utf8();
        let is_covered = covered[offset..end].iter().all(|value| *value);
        match (start, is_covered) {
            (None, false) => start = Some(offset),
            (Some(fragment_start), true) => {
                push_uncovered_fragment(line, fragment_start, offset, &mut fragments);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(fragment_start) = start {
        push_uncovered_fragment(line, fragment_start, line.len(), &mut fragments);
    }
    fragments
}

fn push_uncovered_fragment(
    line: &str,
    start: usize,
    end: usize,
    fragments: &mut Vec<(usize, usize, String)>,
) {
    let fragment = &line[start..end];
    if !fragment.chars().any(char::is_alphanumeric) {
        return;
    }
    fragments.push((start, end, fragment.into()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_properties_and_lists_have_clean_semantic_values() {
        let lines = semantic_lines("# Request\nPurpose: explain\nItems:\n  - one\n").unwrap();
        assert_eq!(
            lines
                .iter()
                .map(|line| line.semantic.as_str())
                .collect::<Vec<_>>(),
            ["# Request", "Purpose:", "explain", "Items:", "one"]
        );
        assert_eq!(lines[1].physical_line, lines[2].physical_line);
    }
}
