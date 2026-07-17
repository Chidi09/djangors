pub mod collect;
pub mod error;
pub mod serve;

pub use collect::Manifest;
pub use error::StaticFilesError;
pub use serve::{StaticFiles, StaticFilesHandler};
