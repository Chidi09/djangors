use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Expr, Fields, GenericArgument, PathArguments, Type};

/// If `ty` is `Option<T>`, returns `Some(&T)`. Otherwise `None`.
fn option_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    })
}

pub fn expand_derive_settings(input: DeriveInput) -> syn::Result<TokenStream> {
    let struct_name_ident = &input.ident;

    let mut prefix = String::new();
    for attr in &input.attrs {
        if attr.path().is_ident("djangors") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("prefix") {
                    let expr: Expr = meta.value()?.parse()?;
                    prefix = parse_str_value(&expr)?;
                    Ok(())
                } else {
                    Err(meta.error("unrecognized settings option"))
                }
            })?;
        }
    }

    let data_struct = match &input.data {
        Data::Struct(s) => s,
        _ => {
            return Err(syn::Error::new_spanned(
                struct_name_ident,
                "Settings derive macro only supports structs",
            ))
        }
    };
    let named_fields = match &data_struct.fields {
        Fields::Named(f) => f,
        _ => {
            return Err(syn::Error::new_spanned(
                struct_name_ident,
                "Settings derive macro only supports structs with named fields",
            ))
        }
    };

    let mut field_inits = Vec::new();

    for field in &named_fields.named {
        let field_ident = field
            .ident
            .clone()
            .ok_or_else(|| syn::Error::new_spanned(field, "unnamed fields not supported"))?;
        let field_name_str = field_ident.to_string();
        let env_var = if prefix.is_empty() {
            field_name_str.to_uppercase()
        } else {
            format!("{}_{}", prefix.to_uppercase(), field_name_str.to_uppercase())
        };

        let mut default_expr: Option<Expr> = None;
        for attr in &field.attrs {
            if attr.path().is_ident("djangors") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("default") {
                        default_expr = Some(meta.value()?.parse()?);
                        Ok(())
                    } else {
                        Err(meta.error("unrecognized settings field option"))
                    }
                })?;
            }
        }

        let init = if let Some(inner_ty) = option_inner_type(&field.ty) {
            // Option<T>: unset env var -> None, never an error.
            quote! {
                #field_ident: match ::std::env::var(#env_var) {
                    ::std::result::Result::Ok(raw) => ::std::option::Option::Some(
                        <#inner_ty as djangors_core::settings::FromSettingsValue>::parse_settings_value(&raw)
                            .map_err(|message| djangors_core::settings::SettingsError::InvalidValue {
                                field: #field_name_str,
                                env_var: #env_var.to_string(),
                                message,
                            })?
                    ),
                    ::std::result::Result::Err(_) => ::std::option::Option::None,
                }
            }
        } else {
            let field_ty = &field.ty;
            let fallback = match &default_expr {
                Some(expr) => quote! { #expr },
                None => quote! {
                    return ::std::result::Result::Err(djangors_core::settings::SettingsError::MissingRequired {
                        field: #field_name_str,
                        env_var: #env_var.to_string(),
                    })
                },
            };
            quote! {
                #field_ident: match ::std::env::var(#env_var) {
                    ::std::result::Result::Ok(raw) => {
                        <#field_ty as djangors_core::settings::FromSettingsValue>::parse_settings_value(&raw)
                            .map_err(|message| djangors_core::settings::SettingsError::InvalidValue {
                                field: #field_name_str,
                                env_var: #env_var.to_string(),
                                message,
                            })?
                    }
                    ::std::result::Result::Err(_) => #fallback,
                }
            }
        };

        field_inits.push(init);
    }

    Ok(quote! {
        impl #struct_name_ident {
            /// Loads this settings struct from environment variables, applying any
            /// `#[djangors(default = ...)]` fallbacks and erroring on the first field
            /// that is both required and unset.
            pub fn load() -> ::std::result::Result<Self, djangors_core::settings::SettingsError> {
                ::std::result::Result::Ok(Self {
                    #(#field_inits),*
                })
            }
        }
    })
}

fn parse_str_value(expr: &Expr) -> syn::Result<String> {
    if let Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(s),
        ..
    }) = expr
    {
        Ok(s.value())
    } else {
        Err(syn::Error::new_spanned(expr, "expected a string literal"))
    }
}
