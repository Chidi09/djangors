# `examples/polls` — the Djangors API spec

This is Django's tutorial app, ported to the API we want Djangors to have. It is **not meant to compile yet.** Per `PLAN.md` Part 3 ("What developer code looks like") and Part 9 ("Start-today checklist" item 4): write the aspirational target *before* writing the framework that makes it real, so every ORM/macro/admin design decision gets justified against real code instead of invented in a vacuum.

## What's real right now vs. aspirational

As of the `djangors-core` work in Phase 1 (commit `70161d7`), the **HTTP layer in this file is 100% real** — `Router`, `Handler`, `Request`, `Response`, `PathParams`, `DjangorsError`, `DjangorsSettings`, `Djangors::run()`, and the `Json`/`Query`/`Form` extractors all exist and work exactly as written in `views.rs` and `urls.rs` below (extractors are called manually via `::from_request(&req).await?` for now — see the note in `views.rs`; auto-wiring them as handler parameters is deferred, noted in `PLAN.md` Phase 1's extractors entry).

**Everything in `models.rs` and `admin.rs` is aspirational** — `#[derive(Model)]`, `ModelMeta`, `ForeignKey<T>`, the `q!()` query macro, `ModelAdmin`/`AdminSite`, and `req.db()` don't exist yet. That's Phase 2 (ORM + migrations) and Phase 5 (admin).

## How to use this file going forward

Each time a Phase 2+ crate lands a real piece of this spec (e.g. `#[derive(Model)]` actually works), come back here and:
1. Try compiling the relevant module in isolation.
2. Fix drift between what got built and what's written here — if the real API had to diverge from the spec (it will, sometimes), that divergence is a decision worth a one-line comment explaining why.
3. Once `models.rs`, `views.rs`, `urls.rs`, and `admin.rs` all compile together, add this crate to the workspace `members` list in the root `Cargo.toml` and wire it into CI (per `PLAN.md`'s "the polls example always compiling in CI" requirement) — until then it's deliberately excluded so it doesn't break `cargo build --workspace`.

## Design decisions already visible in this spec

- **No `#[handler]` macro needed.** The original `PLAN.md` sketch used one; it turned out unnecessary — `Handler`'s blanket impl over `Fn(Request, PathParams) -> Fut` means a plain `async fn` already satisfies `Handler` directly (see `djangors-core/src/handler.rs`). One less macro to build, one less thing for a Django developer to learn.
- **Handlers take owned `Request`/`PathParams`, not references.** This was a deliberate fix during Phase 1 (see commit `361568d`'s follow-up) specifically because owned inputs are what make the blanket impl above possible in stable Rust without `async-trait`.
