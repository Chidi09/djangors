#![deny(missing_docs)]
//! `dj deploy` - multi-provider deployment for the Djangors web framework.
//!
//! A [`DeployProvider`] trait covering the shape every real deployment target
//! needs (`provision`/`deploy`/`status`/`logs`/`destroy`), so `dj deploy` can
//! support Render, Railway, a raw VPS over SSH, GCP, and AWS behind one CLI
//! surface. Ships with a real, live-verified [`render::RenderProvider`] first
//! (this framework's own example apps deploy there); other providers follow
//! the same trait and can be added incrementally.

mod provider;
/// [`DeployProvider`] implementation for Render.
pub mod render;
/// [`DeployProvider`] implementation for a raw VPS over SSH.
pub mod ssh;

pub use provider::{DeployError, DeployProvider, DeploySpec, DeployStatus, DeploymentInfo};
