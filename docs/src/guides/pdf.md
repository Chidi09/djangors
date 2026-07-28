# PDF Generation (`djangors-pdf`)

`djangors-pdf` is the Djangors equivalent of `weasyprint`, but built differently on purpose.
Rather than rendering HTML/CSS through a headless browser engine (a real, heavy runtime
dependency that's often awkward or impossible to install in a minimal container image),
`djangors-pdf` is a typed Rust builder API over a pure-Rust PDF backend. There's no Chrome/
Chromium dependency to ship, and the documents you build are structured (headings, text, tables)
rather than arbitrary rendered markup. That's a good fit for the actual common cases: report cards,
invoices, and receipts.

## Basic usage

```rust,compile
# fn build() -> Result<Vec<u8>, djangors_pdf::PdfError> {
use djangors_pdf::PdfDocument;

let mut doc = PdfDocument::new("Report Card")?;
doc.heading("Term Report")
    .text("Student: Jane Doe")
    .text("Class: Grade 5A")
    .spacer(5.0)
    .table(
        &["Subject", "Score", "Grade"],
        &[
            vec!["Mathematics".to_string(), "92".to_string(), "A".to_string()],
            vec!["English".to_string(), "78".to_string(), "B".to_string()],
        ],
    );

let bytes: Vec<u8> = doc.render()?;
# Ok(bytes)
# }
```

`render()` consumes the builder and returns the finished PDF as `Vec<u8>`. Write it directly to a
response body, to disk, or through a `Storage` backend (see the ORM/static-files guides).

## Builder methods

- **`PdfDocument::new(title)`**: starts a new A4 document with the given title.
- **`.heading(text)`**: a large, bold heading line.
- **`.text(text)`**: a regular body-text line.
- **`.spacer(height_mm)`**: vertical whitespace, in millimeters.
- **`.table(headers, rows)`**: a simple table. A header row plus any number of data rows (each
  row is a `Vec<String>` matching the header count).

All of these return `&mut Self`, so calls chain. Content automatically flows onto a new page once
the current one fills up. You don't need to manage page breaks yourself.

## Serving a generated PDF

```rust,illustrative
use djangors_core::{Request, PathParams, Response, DjangorsError, StatusCode};
use djangors_pdf::PdfDocument;

pub async fn report_card_view(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let mut doc = PdfDocument::new("Report Card").map_err(|e| DjangorsError::Internal(e.to_string()))?;
    doc.heading("Term Report").text("Student: Jane Doe");
    let bytes = doc.render().map_err(|e| DjangorsError::Internal(e.to_string()))?;

    Ok(Response::bytes(StatusCode::OK, "application/pdf", bytes)
        .header("Content-Disposition", "inline; filename=\"report-card.pdf\""))
}
```
