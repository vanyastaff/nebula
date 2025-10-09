//! Source span tracking for error reporting
//!
//! This module provides types for tracking source code positions.

/// A span in the source code
///
/// Uses u32 for positions to reduce memory footprint (supports files up to 4GB).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// Start byte offset in the source
    pub start: u32,
    /// End byte offset in the source (exclusive)
    pub end: u32,
}

impl Span {
    /// Create a new span
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start: start as u32,
            end: end as u32,
        }
    }

    /// Create a span for a single character
    pub fn single(pos: usize) -> Self {
        let pos = pos as u32;
        Self {
            start: pos,
            end: pos + 1,
        }
    }

    /// Get the length of this span
    pub fn len(&self) -> usize {
        (self.end.saturating_sub(self.start)) as usize
    }

    /// Check if this span is empty
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// Merge two spans into a single span covering both
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Extract the text for this span from the source
    pub fn slice<'a>(&self, source: &'a str) -> &'a str {
        source
            .get(self.start as usize..self.end as usize)
            .unwrap_or("")
    }

    /// Get line and column information for this span
    pub fn line_col(&self, source: &str) -> (usize, usize) {
        let mut line = 1;
        let mut col = 1;
        let start = self.start as usize;

        for (i, ch) in source.char_indices() {
            if i >= start {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }

        (line, col)
    }

    /// Get line and column range for multi-line spans
    pub fn line_col_range(&self, source: &str) -> ((usize, usize), (usize, usize)) {
        let start_lc = self.line_col(source);

        // Calculate end position
        let mut line = start_lc.0;
        let mut col = start_lc.1;
        let start = self.start as usize;
        let end = (self.end as usize).min(source.len());

        for ch in source[start..end].chars() {
            if ch == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }

        (start_lc, (line, col))
    }
}

impl Default for Span {
    fn default() -> Self {
        Self { start: 0, end: 0 }
    }
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_span_creation() {
        let span = Span::new(5, 10);
        assert_eq!(span.start, 5);
        assert_eq!(span.end, 10);
        assert_eq!(span.len(), 5);
        assert!(!span.is_empty());
    }

    #[test]
    fn test_span_single() {
        let span = Span::single(5);
        assert_eq!(span.start, 5);
        assert_eq!(span.end, 6);
        assert_eq!(span.len(), 1);
    }

    #[test]
    fn test_span_merge() {
        let span1 = Span::new(5, 10);
        let span2 = Span::new(8, 15);
        let merged = span1.merge(span2);
        assert_eq!(merged.start, 5);
        assert_eq!(merged.end, 15);
    }

    #[test]
    fn test_span_slice() {
        let source = "hello world";
        let span = Span::new(0, 5);
        assert_eq!(span.slice(source), "hello");

        let span = Span::new(6, 11);
        assert_eq!(span.slice(source), "world");
    }

    #[test]
    fn test_line_col() {
        let source = "line1\nline2\nline3";

        // First line, first char
        let span = Span::new(0, 1);
        assert_eq!(span.line_col(source), (1, 1));

        // Second line, first char
        let span = Span::new(6, 7);
        assert_eq!(span.line_col(source), (2, 1));

        // Third line, first char
        let span = Span::new(12, 13);
        assert_eq!(span.line_col(source), (3, 1));
    }

    #[test]
    fn test_line_col_range() {
        let source = "hello\nworld";
        let span = Span::new(0, 11);
        let ((start_line, start_col), (end_line, end_col)) = span.line_col_range(source);

        assert_eq!(start_line, 1);
        assert_eq!(start_col, 1);
        assert_eq!(end_line, 2);
        assert_eq!(end_col, 6);
    }
}
