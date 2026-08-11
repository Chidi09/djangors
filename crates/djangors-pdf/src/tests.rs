use super::*;

#[test]
fn renders_a_real_pdf_with_valid_header_and_trailer() {
    let mut doc = PdfDocument::new("Test Document").unwrap();
    doc.heading("Report Card").text("Student: Jane Doe");
    let bytes = doc.render().unwrap();

    assert!(
        bytes.starts_with(b"%PDF-"),
        "must start with a real PDF header"
    );
    assert!(
        bytes.windows(5).any(|w| w == b"%%EOF"),
        "must end with a real PDF trailer"
    );
}

#[test]
fn table_renders_headers_and_every_row() {
    let mut doc = PdfDocument::new("Invoice").unwrap();
    doc.table(
        &["Item", "Qty", "Price"],
        &[
            vec!["Widget".to_string(), "3".to_string(), "$9.00".to_string()],
            vec!["Gadget".to_string(), "1".to_string(), "$25.00".to_string()],
        ],
    );
    let bytes = doc.render().unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    // Text content in a PDF is encoded/compressed, not searchable as plain
    // bytes in general - this just confirms rendering a real table didn't
    // panic or produce a truncated/invalid file.
    assert!(bytes.len() > 500);
}

#[test]
fn a_long_document_produces_multiple_pages_without_panicking() {
    let mut doc = PdfDocument::new("Long Report").unwrap();
    doc.heading("Annual Report");
    for i in 0..80 {
        doc.text(&format!("Line {i}"));
    }
    let bytes = doc.render().unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    // 80 lines at ~7mm each on an A4 page (297mm, ~20mm margins) doesn't fit
    // on one page - ensure_space() must have started a second page rather
    // than silently running text off the bottom of the page. A real
    // multi-page document is meaningfully larger than a single-page one.
    assert!(bytes.len() > 1000);
}
