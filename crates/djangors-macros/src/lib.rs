#![deny(missing_docs)]
//! Proc macros for the Djangors web framework
//!
//! Provides procedural derive macros `#[derive(Model)]` and `#[derive(Form)]`,
//! as well as attribute macros like `#[task]`.

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

mod form;
mod model;
mod task;

/// Derive macro for `Model` structs in the Djangors ORM.
///
/// Automatically generates table metadata, column mappings, and `QuerySet` helpers
/// for a struct representing a database table.
///
/// # Requirements and Constraints
/// Every `Model` struct must have an explicit primary key designated with `#[djangors(primary_key)]`,
/// or contain a field named `id` of an integer type.
#[proc_macro_derive(Model, attributes(djangors))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    model::expand_derive_model(input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// Derive macro for `Form` structs in Djangors.
///
/// Generates field-level validation and form cleaning methods for HTTP input mapping.
#[proc_macro_derive(Form, attributes(djangors))]
pub fn derive_form(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    form::expand_derive_form(input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// Attribute macro for registering background tasks.
///
/// Uses `inventory` for distributed task handler registration at link time,
/// allowing tasks to be invoked asynchronously across worker processes.
#[proc_macro_attribute]
pub fn task(attr: TokenStream, item: TokenStream) -> TokenStream {
    task::expand_task(attr.into(), item.into())
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}
