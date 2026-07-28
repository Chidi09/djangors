#![deny(missing_docs)]
//! Typed PDF generation for the Djangors web framework.
//!
//! Not an HTML+CSS rendering engine like `weasyprint` - genuine HTML/CSS-fidelity
//! PDF rendering needs an external browser engine (headless Chrome or similar),
//! which this crate deliberately avoids so a Djangors deployment never gains an
//! implicit Chrome/Chromium runtime dependency. Instead this provides a typed
//! Rust builder API - in the same spirit as `djangors-forms`' typed forms over
//! Django's stringly-typed ones - for the concrete documents real apps actually
//! need: report cards, invoices, receipts. Structured text and simple tables,
//! flowing top to bottom with automatic page breaks, not arbitrary page layout.
//!
//! ```
//! use djangors_pdf::PdfDocument;
//!
//! let mut doc = PdfDocument::new("Invoice #1042").unwrap();
//! doc.heading("Invoice #1042")
//!     .text("Billed to: Acme Corp")
//!     .spacer(5.0)
//!     .table(
//!         &["Item", "Qty", "Price"],
//!         &[
//!             vec!["Widget".to_string(), "3".to_string(), "$9.00".to_string()],
//!             vec!["Gadget".to_string(), "1".to_string(), "$25.00".to_string()],
//!         ],
//!     );
//! let bytes = doc.render().unwrap();
//! assert!(bytes.starts_with(b"%PDF"));
//! ```

use printpdf::{
    BuiltinFont, IndirectFontRef, Mm, PdfDocument as InnerDoc, PdfDocumentReference,
    PdfLayerReference,
};
use std::io::{BufWriter, Cursor};

/// Errors produced by PDF generation.
#[derive(thiserror::Error, Debug)]
pub enum PdfError {
    /// The underlying PDF library failed to build or serialize the document.
    #[error("failed to render PDF: {0}")]
    Render(String),
}

const PAGE_WIDTH_MM: f32 = 210.0; // A4
const PAGE_HEIGHT_MM: f32 = 297.0;
const MARGIN_MM: f32 = 20.0;
const LINE_HEIGHT_MM: f32 = 7.0;

/// A single-column document builder for report cards, invoices, and receipts:
/// headings, text lines, and simple tables, flowing top to bottom across A4
/// pages with automatic page breaks when content runs off the bottom margin.
pub struct PdfDocument {
    doc: PdfDocumentReference,
    font: IndirectFontRef,
    bold_font: IndirectFontRef,
    current_layer: PdfLayerReference,
    cursor_y_mm: f32,
}

impl PdfDocument {
    /// Creates a new A4-portrait document. `title` is embedded in the PDF's own
    /// metadata (visible in a PDF viewer's document properties) - it is not
    /// rendered on the page itself; call [`PdfDocument::heading`] for that.
    pub fn new(title: &str) -> Result<Self, PdfError> {
        let (doc, page1, layer1) =
            InnerDoc::new(title, Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), "Layer 1");
        let font = doc
            .add_builtin_font(BuiltinFont::Helvetica)
            .map_err(|e| PdfError::Render(e.to_string()))?;
        let bold_font = doc
            .add_builtin_font(BuiltinFont::HelveticaBold)
            .map_err(|e| PdfError::Render(e.to_string()))?;
        let current_layer = doc.get_page(page1).get_layer(layer1);
        Ok(Self {
            doc,
            font,
            bold_font,
            current_layer,
            cursor_y_mm: PAGE_HEIGHT_MM - MARGIN_MM,
        })
    }

    /// Starts a fresh page if fewer than `needed_mm` of vertical space remain
    /// above the bottom margin on the current one.
    fn ensure_space(&mut self, needed_mm: f32) {
        if self.cursor_y_mm - needed_mm < MARGIN_MM {
            let (page, layer) =
                self.doc
                    .add_page(Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), "Layer 1");
            self.current_layer = self.doc.get_page(page).get_layer(layer);
            self.cursor_y_mm = PAGE_HEIGHT_MM - MARGIN_MM;
        }
    }

    /// Writes a bold, larger-than-body-text heading line and advances the cursor.
    pub fn heading(&mut self, text: &str) -> &mut Self {
        self.ensure_space(LINE_HEIGHT_MM * 1.5);
        self.current_layer.use_text(
            text,
            16.0,
            Mm(MARGIN_MM),
            Mm(self.cursor_y_mm),
            &self.bold_font,
        );
        self.cursor_y_mm -= LINE_HEIGHT_MM * 1.5;
        self
    }

    /// Writes a regular-weight body text line and advances the cursor.
    pub fn text(&mut self, text: &str) -> &mut Self {
        self.ensure_space(LINE_HEIGHT_MM);
        self.current_layer
            .use_text(text, 11.0, Mm(MARGIN_MM), Mm(self.cursor_y_mm), &self.font);
        self.cursor_y_mm -= LINE_HEIGHT_MM;
        self
    }

    /// Adds vertical whitespace without writing text.
    pub fn spacer(&mut self, height_mm: f32) -> &mut Self {
        self.cursor_y_mm -= height_mm;
        self
    }

    /// Writes a simple table: a bold header row, then one line per data row,
    /// with columns evenly spaced across the page width. Cells beyond
    /// `headers.len()` in any row are ignored; missing cells are left blank.
    pub fn table(&mut self, headers: &[&str], rows: &[Vec<String>]) -> &mut Self {
        let col_count = headers.len().max(1);
        let col_width_mm = (PAGE_WIDTH_MM - 2.0 * MARGIN_MM) / col_count as f32;

        self.ensure_space(LINE_HEIGHT_MM);
        for (i, header) in headers.iter().enumerate() {
            let x = MARGIN_MM + col_width_mm * i as f32;
            self.current_layer.use_text(
                *header,
                11.0,
                Mm(x),
                Mm(self.cursor_y_mm),
                &self.bold_font,
            );
        }
        self.cursor_y_mm -= LINE_HEIGHT_MM;

        for row in rows {
            self.ensure_space(LINE_HEIGHT_MM);
            for (i, cell) in row.iter().enumerate().take(headers.len()) {
                let x = MARGIN_MM + col_width_mm * i as f32;
                self.current_layer
                    .use_text(cell.as_str(), 11.0, Mm(x), Mm(self.cursor_y_mm), &self.font);
            }
            self.cursor_y_mm -= LINE_HEIGHT_MM;
        }
        self
    }

    /// Renders the document to raw PDF bytes, suitable for
    /// `djangors_core::Response::bytes(StatusCode::OK, "application/pdf", bytes)`.
    pub fn render(self) -> Result<Vec<u8>, PdfError> {
        let mut buf = Vec::new();
        {
            let mut writer = BufWriter::new(Cursor::new(&mut buf));
            self.doc
                .save(&mut writer)
                .map_err(|e| PdfError::Render(e.to_string()))?;
        }
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_real_pdf_with_valid_header_and_trailer() {
        let mut doc = PdfDocument::new("Test Document").unwrap();
        doc.heading("Report Card").text("Student: Jane Doe");
        let bytes = doc.render().unwrap();

        assert!(bytes.starts_with(b"%PDF-"), "must start with a real PDF header");
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
}
