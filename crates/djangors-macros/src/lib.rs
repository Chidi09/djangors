//! Proc macros for the Djangors web framework
//!
//! Provides the `#[derive(Model)]` macro.

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

mod form;
mod model;

#[proc_macro_derive(Model, attributes(djangors))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    model::expand_derive_model(input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

#[proc_macro_derive(Form, attributes(djangors))]
pub fn derive_form(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    form::expand_derive_form(input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}
