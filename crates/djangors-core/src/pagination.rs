//! Pagination utility shared by admin and user code.

use base64::Engine;

/// Errors returned while decoding an opaque cursor.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CursorError {
    /// The cursor was not valid base64.
    #[error("invalid cursor encoding")]
    InvalidEncoding,
    /// The decoded cursor did not contain exactly a primary key and ordering value.
    #[error("invalid cursor format")]
    InvalidFormat,
    /// The primary key portion was not an integer.
    #[error("invalid cursor primary key")]
    InvalidPrimaryKey,
}

/// Encodes a cursor as base64 of the UTF-8 string `pk|ordering-value`.
pub fn encode_cursor(pk: i64, order_value: Option<&str>) -> String {
    let raw = format!("{}|{}", pk, order_value.unwrap_or(""));
    base64::engine::general_purpose::STANDARD.encode(raw)
}

/// Decodes a cursor produced by [`encode_cursor`].
pub fn decode_cursor(cursor: &str) -> Result<(i64, Option<String>), CursorError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(cursor)
        .map_err(|_| CursorError::InvalidEncoding)?;
    let raw = String::from_utf8(bytes).map_err(|_| CursorError::InvalidFormat)?;
    let (pk, value) = raw.split_once('|').ok_or(CursorError::InvalidFormat)?;
    let pk = pk.parse().map_err(|_| CursorError::InvalidPrimaryKey)?;
    Ok((pk, (!value.is_empty()).then(|| value.to_string())))
}

/// A page of results addressed by opaque cursors.
#[derive(Debug, Clone, PartialEq)]
pub struct CursorPage<T> {
    /// Items in this page.
    pub items: Vec<T>,
    /// Cursor for the next page, if one exists.
    pub next_cursor: Option<String>,
    /// Cursor for the previous page, if one exists.
    pub previous_cursor: Option<String>,
}

/// Pagination math engine for item sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paginator {
    /// Total count of items across all pages.
    pub total_items: i64,
    /// Maximum number of items per page.
    pub page_size: i64,
}

impl Paginator {
    /// Creates a new `Paginator`.
    /// `page_size` must be greater than 0. `total_items` is clamped to at least 0.
    pub fn new(total_items: i64, page_size: i64) -> Self {
        assert!(page_size > 0, "page_size must be positive");
        Self {
            total_items: total_items.max(0),
            page_size,
        }
    }

    /// Computes total number of pages.
    /// 0 total items yields 1 page per Djangors admin convention.
    pub fn total_pages(&self) -> i64 {
        if self.total_items == 0 {
            1
        } else {
            (self.total_items + self.page_size - 1) / self.page_size
        }
    }

    /// Returns the 0-indexed SQL query offset for the given 1-indexed page number,
    /// after clamping the page number into `[1, total_pages()]`.
    pub fn offset(&self, page: i64) -> i64 {
        let clamped = page.clamp(1, self.total_pages());
        (clamped - 1) * self.page_size
    }

    /// Returns `true` if a previous page exists before `page`.
    /// Per Djangors admin convention: `has_previous` is `page > 1`.
    pub fn has_previous(&self, page: i64) -> bool {
        page > 1
    }

    /// Returns `true` if a next page exists after `page`.
    /// Per Djangors admin convention: `has_next` is `page * page_size < total_items`.
    pub fn has_next(&self, page: i64) -> bool {
        page * self.page_size < self.total_items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paginator_zero_items() {
        let p = Paginator::new(0, 100);
        assert_eq!(p.total_pages(), 1);
        assert_eq!(p.offset(1), 0);
        assert!(!p.has_previous(1));
        assert!(!p.has_next(1));
    }

    #[test]
    fn test_cursor_ordering_value_may_contain_pipe() {
        let cursor = encode_cursor(42, Some("title|with|pipes"));
        assert_eq!(
            decode_cursor(&cursor),
            Ok((42, Some("title|with|pipes".to_string())))
        );
    }

    #[test]
    fn test_paginator_exact_one_page() {
        let p = Paginator::new(50, 100);
        assert_eq!(p.total_pages(), 1);
        assert_eq!(p.offset(1), 0);
        assert!(!p.has_previous(1));
        assert!(!p.has_next(1));

        let p_exact = Paginator::new(100, 100);
        assert_eq!(p_exact.total_pages(), 1);
        assert_eq!(p_exact.offset(1), 0);
        assert!(!p_exact.has_previous(1));
        assert!(!p_exact.has_next(1));
    }

    #[test]
    fn test_paginator_multiple_pages() {
        let p = Paginator::new(250, 100);
        assert_eq!(p.total_pages(), 3);

        // Page 1
        assert_eq!(p.offset(1), 0);
        assert!(!p.has_previous(1));
        assert!(p.has_next(1));

        // Page 2
        assert_eq!(p.offset(2), 100);
        assert!(p.has_previous(2));
        assert!(p.has_next(2));

        // Page 3
        assert_eq!(p.offset(3), 200);
        assert!(p.has_previous(3));
        assert!(!p.has_next(3));
    }

    #[test]
    fn test_paginator_past_end_clamped() {
        let p = Paginator::new(250, 100);
        // total_pages is 3
        assert_eq!(p.offset(5), 200);
        assert_eq!(p.offset(100), 200);
    }

    #[test]
    fn test_paginator_zero_or_negative_page_clamped() {
        let p = Paginator::new(250, 100);
        assert_eq!(p.offset(0), 0);
        assert_eq!(p.offset(-5), 0);
    }
}
