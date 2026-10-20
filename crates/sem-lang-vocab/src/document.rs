//! Lossless, language-neutral structure for human-authored `.sem` documents.
//!
//! This parser recognizes presentation structure only.  It deliberately does
//! not assign meaning to heading or property text: selected semantic packages
//! own that resolution.  Keeping the original line and span makes a `.sem`
//! document safe to share as Markdown without making rendering another source
//! of truth.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub source: String,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub line: usize,
    pub indent: usize,
    pub raw: String,
    pub kind: BlockKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    Blank,
    Comment(String),
    Heading {
        level: u8,
        text: String,
    },
    Property {
        name: String,
        value: Option<String>,
    },
    ListItem {
        ordered: bool,
        ordinal: Option<u64>,
        text: String,
    },
    Quote(String),
    CodeFence {
        marker: String,
        info: String,
    },
    Code(String),
    TableRow(Vec<String>),
    Paragraph(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DocumentError {
    #[error("sem document contains a NUL byte")]
    Nul,
    #[error("sem document contains a heading deeper than six levels on line {0}")]
    HeadingDepth(usize),
    #[error("sem document contains an unclosed code fence starting on line {0}")]
    UnclosedCodeFence(usize),
}

impl Document {
    pub fn parse(source: &str) -> Result<Self, DocumentError> {
        if source.as_bytes().contains(&0) {
            return Err(DocumentError::Nul);
        }
        let mut blocks = Vec::new();
        let mut fence = None::<(String, usize)>;
        for (line, original) in source.lines().enumerate() {
            let raw = original.trim_end_matches('\r').to_owned();
            let indent = raw.len() - raw.trim_start_matches([' ', '\t']).len();
            let trimmed = raw.trim();
            let kind = if let Some((marker, _)) = fence.clone() {
                if trimmed.starts_with(marker.as_str()) {
                    fence = None;
                    BlockKind::CodeFence {
                        marker,
                        info: String::new(),
                    }
                } else {
                    BlockKind::Code(raw.clone())
                }
            } else if trimmed.is_empty() {
                BlockKind::Blank
            } else if let Some(value) = trimmed.strip_prefix("//") {
                BlockKind::Comment(value.trim_start().to_owned())
            } else if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                let marker = trimmed
                    .chars()
                    .take_while(|value| *value == '`' || *value == '~')
                    .collect::<String>();
                let info = trimmed[marker.len()..].trim().to_owned();
                fence = Some((marker.clone(), line));
                BlockKind::CodeFence { marker, info }
            } else if let Some((level, text)) = heading(trimmed, line)? {
                BlockKind::Heading {
                    level,
                    text: text.to_owned(),
                }
            } else if let Some((ordered, ordinal, text)) = list_item(trimmed) {
                BlockKind::ListItem {
                    ordered,
                    ordinal,
                    text: text.to_owned(),
                }
            } else if let Some(value) = trimmed.strip_prefix('>') {
                BlockKind::Quote(value.trim_start().to_owned())
            } else if let Some(cells) = table_row(trimmed) {
                BlockKind::TableRow(cells)
            } else if let Some((name, value)) = property(trimmed) {
                BlockKind::Property {
                    name: name.to_owned(),
                    value: value.map(str::to_owned),
                }
            } else {
                BlockKind::Paragraph(trimmed.to_owned())
            };
            blocks.push(Block {
                line,
                indent,
                raw,
                kind,
            });
        }
        if let Some((_, line)) = fence {
            return Err(DocumentError::UnclosedCodeFence(line + 1));
        }
        Ok(Self {
            source: source.to_owned(),
            blocks,
        })
    }

    #[must_use]
    pub fn render(&self) -> &str {
        &self.source
    }
}

fn heading(value: &str, line: usize) -> Result<Option<(u8, &str)>, DocumentError> {
    let count = value.bytes().take_while(|byte| *byte == b'#').count();
    if count == 0
        || !value
            .as_bytes()
            .get(count)
            .is_some_and(u8::is_ascii_whitespace)
    {
        return Ok(None);
    }
    if count > 6 {
        return Err(DocumentError::HeadingDepth(line + 1));
    }
    let text = value[count..].trim();
    Ok((!text.is_empty()).then_some((count as u8, text)))
}

fn list_item(value: &str) -> Option<(bool, Option<u64>, &str)> {
    if let Some(text) = value
        .strip_prefix("- ")
        .or_else(|| value.strip_prefix("* "))
        .or_else(|| value.strip_prefix("+ "))
    {
        return Some((false, None, text.trim()));
    }
    let split = value.find(". ")?;
    let ordinal = value[..split].parse().ok()?;
    Some((true, Some(ordinal), value[split + 2..].trim()))
}

fn property(value: &str) -> Option<(&str, Option<&str>)> {
    let split = value.find(':')?;
    let remainder = &value[split + 1..];
    // A property delimiter is either terminal or followed by whitespace.
    // This keeps URI schemes, clock values and other colon-bearing values as
    // ordinary semantic evidence instead of inventing property names.
    if !remainder.is_empty() && !remainder.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let name = &value[..split];
    let value = remainder;
    let name = name.trim();
    if name.is_empty() || name.contains(['[', ']', '(', ')']) {
        return None;
    }
    let value = value.trim();
    Some((name, (!value.is_empty()).then_some(value)))
}

fn table_row(value: &str) -> Option<Vec<String>> {
    if !value.starts_with('|') || !value.ends_with('|') {
        return None;
    }
    let cells = value[1..value.len() - 1]
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect::<Vec<_>>();
    (cells.len() >= 2).then_some(cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_shareable_markdown_and_extracts_language_neutral_structure() {
        let source = "# ユーザー管理を確認する\n\n目的: 権限を確認する。\n\n対象:\n  - 管理者\n  - 一般利用者\n\n手順:\n  1. 仕様を確認する。\n  2. 結果を記録する。\n\n> 利用者の確認が必要\n";
        let document = Document::parse(source).unwrap();
        assert_eq!(document.render(), source);
        assert!(matches!(
            document.blocks[0].kind,
            BlockKind::Heading { level: 1, .. }
        ));
        assert!(document.blocks.iter().any(|block| matches!(block.kind, BlockKind::Property { ref name, value: None } if name == "対象")));
        assert_eq!(
            document
                .blocks
                .iter()
                .filter(|block| matches!(block.kind, BlockKind::ListItem { ordered: false, .. }))
                .count(),
            2
        );
        assert_eq!(
            document
                .blocks
                .iter()
                .filter(|block| matches!(block.kind, BlockKind::ListItem { ordered: true, .. }))
                .count(),
            2
        );
    }

    #[test]
    fn keeps_code_content_opaque() {
        let source = "# Example\n```sem\nNot: a property\n```\n";
        let document = Document::parse(source).unwrap();
        assert!(document.blocks.iter().any(
            |block| matches!(block.kind, BlockKind::Code(ref value) if value == "Not: a property")
        ));
    }

    #[test]
    fn uri_and_clock_values_are_not_properties() {
        let source = "# Evidence\nhttps://example.test/resource\n08:00\nPurpose: explain\n";
        let document = Document::parse(source).unwrap();
        assert!(matches!(
            document.blocks[1].kind,
            BlockKind::Paragraph(ref value) if value == "https://example.test/resource"
        ));
        assert!(matches!(
            document.blocks[2].kind,
            BlockKind::Paragraph(ref value) if value == "08:00"
        ));
        assert!(matches!(
            document.blocks[3].kind,
            BlockKind::Property { ref name, value: Some(ref value) }
                if name == "Purpose" && value == "explain"
        ));
    }
}
