//! Proc macros for the Djangors web framework
//!
//! Provides the `#[derive(Model)]` macro.

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

mod model;

#[proc_macro_derive(Model, attributes(djangors))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    model::expand_derive_model(input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}
