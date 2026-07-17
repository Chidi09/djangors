//! Mixed. Settings loading, router construction, and `Djangors::run()` are
//! REAL today. The app-registry (`.app(PollsApp)`, multi-app composition)
//! and CLI-subcommand dispatch through `run()` (migrate/createsuperuser/etc.
//! — that's `dj`'s job per djangors-cli, not this binary's) are
//! ASPIRATIONAL — Phase 2's app-registry design and Phase 6's CLI.

mod admin; // aspirational (Phase 5)
mod models; // aspirational (Phase 2)
mod urls;
mod views;

use djangors_core::{DjangorsError, DjangorsSettings, Djangors};

#[tokio::main]
async fn main() -> Result<(), DjangorsError> {
    djangors_core::logging::init_dev_logging(); // REAL — Phase 1

    let (settings, warnings) = DjangorsSettings::load()?; // REAL — Phase 1
    for w in warnings {
        eprintln!("settings warning: {w}");
    }

    let router = urls::urls(); // REAL — Phase 1

    // ASPIRATIONAL from here: today `Djangors::new` takes exactly one
    // Router; there's no app registry yet to fold `admin::register` or a
    // multi-app project's several routers/migrations/models together. Once
    // Phase 2's `AppConfig`/app-registry design lands, this becomes:
    //
    //   Djangors::new(settings)
    //       .app(PollsApp)
    //       .run()
    //       .await
    //
    // matching PLAN.md Part 3's target shape.
    Djangors::new(settings, router).run().await // REAL — Phase 1
}
