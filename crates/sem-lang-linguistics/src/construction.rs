//! Finite, curator-declared surface constructions. No inferred grammar or action.
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SurfacePattern {
    Text(String),
    Sequence(Vec<Self>),
    Choice(Vec<Self>),
    Optional(Box<Self>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConstructionBound {
    Nodes,
    Depth,
    States,
    Work,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConstructionError {
    #[error("invalid declared surface construction")]
    Invalid,
    #[error("construction {bound:?} exceeded: {observed} > {limit}")]
    LimitExceeded {
        bound: ConstructionBound,
        limit: usize,
        observed: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceMatch {
    Complete,
    Prefix,
    Rejected,
}

fn bound(kind: ConstructionBound, observed: usize, limit: usize) -> Result<(), ConstructionError> {
    if observed > limit {
        Err(ConstructionError::LimitExceeded {
            bound: kind,
            limit,
            observed,
        })
    } else {
        Ok(())
    }
}

#[derive(Default)]
struct Matches {
    ends: BTreeSet<usize>,
    prefix: bool,
}
impl Matches {
    fn extend(&mut self, other: Self) -> Result<(), ConstructionError> {
        self.ends.extend(other.ends);
        self.prefix |= other.prefix;
        bound(ConstructionBound::States, self.ends.len(), 64)
    }
}

impl SurfacePattern {
    fn validate(&self, depth: usize, nodes: &mut usize) -> Result<(), ConstructionError> {
        *nodes += 1;
        bound(ConstructionBound::Nodes, *nodes, 256)?;
        bound(ConstructionBound::Depth, depth, 32)?;
        match self {
            Self::Text(text) if text.is_empty() || text.len() > 16384 || text.contains('\0') => {
                Err(ConstructionError::Invalid)
            }
            Self::Text(_) => Ok(()),
            Self::Optional(child) => child.validate(depth + 1, nodes),
            Self::Sequence(children) | Self::Choice(children) => {
                if children.is_empty() {
                    return Err(ConstructionError::Invalid);
                }
                for child in children {
                    child.validate(depth + 1, nodes)?;
                }
                Ok(())
            }
        }
    }

    fn advance(
        &self,
        text: &str,
        at: usize,
        work: &mut usize,
    ) -> Result<Matches, ConstructionError> {
        *work += 1;
        bound(ConstructionBound::Work, *work, 4096)?;
        let mut result = Matches::default();
        match self {
            Self::Text(expected) => {
                let remaining = &text[at..];
                if remaining.starts_with(expected) {
                    result.ends.insert(at + expected.len());
                } else if expected.starts_with(remaining) {
                    result.prefix = true;
                }
            }
            Self::Sequence(children) => {
                result.ends.insert(at);
                for child in children {
                    let mut next = Matches {
                        prefix: result.prefix,
                        ..Matches::default()
                    };
                    for end in result.ends {
                        next.extend(child.advance(text, end, work)?)?;
                    }
                    result = next;
                    if result.ends.is_empty() {
                        break;
                    }
                }
            }
            Self::Choice(children) => {
                for child in children {
                    result.extend(child.advance(text, at, work)?)?;
                }
            }
            Self::Optional(child) => {
                result.ends.insert(at);
                result.extend(child.advance(text, at, work)?)?;
            }
        }
        Ok(result)
    }

    /// Whole-input coverage is distinct from a known prefix and a contradiction.
    /// A prefix cannot authorize execution or fabricate the missing source text.
    pub fn match_surface(&self, text: &str) -> Result<SurfaceMatch, ConstructionError> {
        self.validate(0, &mut 0)?;
        let result = self.advance(text, 0, &mut 0)?;
        Ok(if result.ends.contains(&text.len()) {
            SurfaceMatch::Complete
        } else if result.prefix {
            SurfaceMatch::Prefix
        } else {
            SurfaceMatch::Rejected
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(
        pattern: SurfacePattern,
        text: &str,
        kind: ConstructionBound,
        observed: usize,
        limit: usize,
    ) {
        let result = pattern.match_surface(text);
        assert_eq!(result, pattern.match_surface(text));
        if observed > limit {
            assert_eq!(
                result,
                Err(ConstructionError::LimitExceeded {
                    bound: kind,
                    limit,
                    observed
                })
            );
        } else {
            assert_eq!(result.unwrap(), SurfaceMatch::Complete);
        }
    }
    #[test]
    fn construction_limits_and_exact_partial_rejected_multilingual_matches() {
        use SurfacePattern::*;
        for n in [255, 256, 257] {
            check(
                Sequence(vec![Text("a".into()); n - 1]),
                &"a".repeat(n - 1),
                ConstructionBound::Nodes,
                n,
                256,
            );
        }
        for n in [31, 32, 33] {
            let mut pattern = Text("a".into());
            for _ in 0..n {
                pattern = Optional(Box::new(pattern));
            }
            check(pattern, "a", ConstructionBound::Depth, n, 32);
        }
        for n in [63, 64, 65] {
            check(
                Choice((1..=n).map(|i| Text("a".repeat(i))).collect()),
                &"a".repeat(n),
                ConstructionBound::States,
                n,
                64,
            );
        }
        for wrappers in 0..3 {
            let mut items = vec![Optional(Box::new(Text("a".into()))); 62];
            items.extend(vec![Text("a".into()); 3]);
            let mut pattern = Sequence(items);
            for _ in 0..wrappers {
                pattern = Optional(Box::new(pattern));
            }
            check(
                pattern,
                &"a".repeat(63),
                ConstructionBound::Work,
                4095 + wrappers,
                4096,
            );
        }
        let pattern = Sequence(vec![
            Choice(vec![Text("予約".into()), Text("reserve".into())]),
            Text(" now".into()),
        ]);
        for (input, expected) in [
            ("予約 now", SurfaceMatch::Complete),
            ("reserve now", SurfaceMatch::Complete),
            ("予約 n", SurfaceMatch::Prefix),
            ("予約 later", SurfaceMatch::Rejected),
            ("extra reserve now", SurfaceMatch::Rejected),
        ] {
            assert_eq!(pattern.match_surface(input).unwrap(), expected);
        }
        for invalid in [Text(String::new()), Sequence(vec![]), Choice(vec![])] {
            assert_eq!(invalid.match_surface(""), Err(ConstructionError::Invalid));
        }
    }
}
