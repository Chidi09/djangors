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
