#![allow(clippy::assertions_on_constants)]

use doc_code_check::{TOTAL_COMPILE_BLOCKS, TOTAL_FILES, TOTAL_ILLUSTRATIVE_BLOCKS};

#[test]
fn test_doc_snippets_compiled() {
    println!(
        "\nDoc code check summary: {} md files scanned, {} compile blocks, {} illustrative blocks",
        TOTAL_FILES, TOTAL_COMPILE_BLOCKS, TOTAL_ILLUSTRATIVE_BLOCKS
    );
    assert!(TOTAL_FILES > 0, "No markdown files scanned!");
}
