use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Type};

pub fn expand_derive_form(input: DeriveInput) -> syn::Result<TokenStream> {
    let struct_name_ident = &input.ident;
    let struct_name = struct_name_ident.to_string();
    let cleaned_name = Ident::new(&format!("{}Cleaned", struct_name), struct_name_ident.span());
    let struct_vis = &input.vis;

    // Parse the fields
    let data_struct = match input.data {
        Data::Struct(s) => s,
        _ => {
            return Err(syn::Error::new_spanned(
                struct_name_ident,
                "Form derive macro only supports structs",
            ))
        }
    };

    let named_fields = match data_struct.fields {
        Fields::Named(f) => f,
        _ => {
            return Err(syn::Error::new_spanned(
                struct_name_ident,
                "Form derive macro only supports structs with named fields",
            ))
        }
    };

    struct FormFieldInfo {
        ident: Ident,
        name_str: String,
        cleaned_ty: TokenStream,
        validator_expr: TokenStream,
    }

    let mut fields_info = Vec::new();

    for field in named_fields.named {
        let field_ident = field
            .ident
            .clone()
            .ok_or_else(|| syn::Error::new_spanned(&field, "unnamed fields not supported"))?;
        let field_name_str = field_ident.to_string();

        let mut max_length = None;
        let mut required = true;
        let mut email = false;
        let mut min = None;
        let mut max = None;

        for attr in &field.attrs {
            if attr.path().is_ident("djangors") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("max_length") {
                        let expr: syn::Expr = meta.value()?.parse()?;
                        let val = parse_int_value(&expr)?;
                        if val < 0 {
                            return Err(syn::Error::new_spanned(
                                &expr,
                                "max_length must be non-negative",
                            ));
                        }
                        max_length = Some(val as usize);
                    } else if meta.path.is_ident("required") {
                        let expr: syn::Expr = meta.value()?.parse()?;
                        required = parse_bool_value(&expr)?;
                    } else if meta.path.is_ident("email") {
                        email = true;
                    } else if meta.path.is_ident("min") {
                        let expr: syn::Expr = meta.value()?.parse()?;
                        min = Some(parse_int_value(&expr)?);
                    } else if meta.path.is_ident("max") {
                        let expr: syn::Expr = meta.value()?.parse()?;
                        max = Some(parse_int_value(&expr)?);
                    } else {
                        return Err(meta.error("unrecognized form option"));
                    }
                    Ok(())
                })?;
            }
        }

        // Determine field type
        let last_ident = get_last_path_segment_ident(&field.ty);
        let last_ident_str = last_ident.map(|id| id.to_string());

        let validator_expr: TokenStream;
        let cleaned_ty: TokenStream;

        match last_ident_str.as_deref() {
            Some("String") => {
                if min.is_some() || max.is_some() {
                    return Err(syn::Error::new_spanned(
                        &field.ty,
                        "min/max are only valid on integer fields",
                    ));
                }

                if email {
                    if max_length.is_some() {
                        return Err(syn::Error::new_spanned(
                            &field.ty,
                            "max_length is not valid on email fields",
                        ));
                    }
                    validator_expr = quote! {
                        djangors_forms::EmailField {
                            required: #required,
                        }
                    };
                } else {
                    let max_len_expanded = match max_length {
                        Some(ml) => quote! { Some(#ml) },
                        None => quote! { None },
                    };
                    validator_expr = quote! {
                        djangors_forms::CharField {
                            max_length: #max_len_expanded,
                            required: #required,
                        }
                    };
                }
                cleaned_ty = quote! { String };
            }
            Some("i64") | Some("i32") => {
                if max_length.is_some() {
                    return Err(syn::Error::new_spanned(
                        &field.ty,
                        "max_length is only valid on String fields",
                    ));
                }
                if email {
                    return Err(syn::Error::new_spanned(
                        &field.ty,
                        "email is only valid on String fields",
                    ));
                }

                let min_expanded = match min {
                    Some(m) => quote! { Some(#m) },
                    None => quote! { None },
                };
                let max_expanded = match max {
                    Some(m) => quote! { Some(#m) },
                    None => quote! { None },
                };
                validator_expr = quote! {
                    djangors_forms::IntegerField {
                        min: #min_expanded,
                        max: #max_expanded,
                        required: #required,
                    }
                };
                cleaned_ty = quote! { Option<i64> };
            }
            Some("bool") => {
                if max_length.is_some() {
                    return Err(syn::Error::new_spanned(
                        &field.ty,
                        "max_length is only valid on String fields",
                    ));
                }
                if email {
                    return Err(syn::Error::new_spanned(
                        &field.ty,
                        "email is only valid on String fields",
                    ));
                }
                if min.is_some() || max.is_some() {
                    return Err(syn::Error::new_spanned(
                        &field.ty,
                        "min/max are only valid on integer fields",
                    ));
                }

                validator_expr = quote! {
                    djangors_forms::BooleanField {
                        required: #required,
                    }
                };
                cleaned_ty = quote! { bool };
            }
            Some(other) => {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    format!("Unsupported field type for Form: {}. Supported types are String, i64, i32, and bool.", other),
                ));
            }
            None => {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "Unsupported field type for Form.",
                ));
            }
        }

        fields_info.push(FormFieldInfo {
            ident: field_ident,
            name_str: field_name_str,
            cleaned_ty,
            validator_expr,
        });
    }

    let field_idents: Vec<&Ident> = fields_info.iter().map(|f| &f.ident).collect();
    let cleaned_tys: Vec<&TokenStream> = fields_info.iter().map(|f| &f.cleaned_ty).collect();
    let val_idents: Vec<Ident> = fields_info
        .iter()
        .map(|f| Ident::new(&format!("{}_val", f.ident), f.ident.span()))
        .collect();
    let raw_idents: Vec<Ident> = fields_info
        .iter()
        .map(|f| Ident::new(&format!("{}_raw", f.ident), f.ident.span()))
        .collect();
    let name_strs: Vec<&String> = fields_info.iter().map(|f| &f.name_str).collect();
    let validator_exprs: Vec<&TokenStream> =
        fields_info.iter().map(|f| &f.validator_expr).collect();

    Ok(quote! {
        #[derive(Debug)]
        #struct_vis struct #cleaned_name {
            #(pub #field_idents: #cleaned_tys),*
        }

        impl #struct_name_ident {
            pub fn clean(data: &std::collections::HashMap<String, String>) -> Result<#cleaned_name, djangors_forms::FormErrors> {
                use djangors_forms::FormField;
                let mut errors = djangors_forms::FormErrors::new();

                #(
                    let #raw_idents = data.get(#name_strs).map(|s| s.as_str());
                    let #val_idents = match (#validator_exprs).clean(#raw_idents) {
                        Ok(val) => Some(val),
                        Err(err) => {
                            for msg in err.0 {
                                errors.add_field_error(#name_strs, msg);
                            }
                            None
                        }
                    };
                )*

                if !errors.is_empty() {
                    return Err(errors);
                }

                Ok(#cleaned_name {
                    #(#field_idents: #val_idents.unwrap()),*
                })
            }
        }
    })
}

fn get_last_path_segment_ident(ty: &Type) -> Option<&Ident> {
    if let Type::Path(type_path) = ty {
        type_path.path.segments.last().map(|seg| &seg.ident)
    } else {
        None
    }
}

fn parse_int_value(expr: &syn::Expr) -> syn::Result<i64> {
    match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Int(lit_int),
            ..
        }) => lit_int.base10_parse(),
        syn::Expr::Unary(syn::ExprUnary {
            op: syn::UnOp::Neg(_),
            expr: inner_expr,
            ..
        }) => {
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Int(lit_int),
                ..
            }) = &**inner_expr
            {
                let val: i64 = lit_int.base10_parse()?;
                Ok(-val)
            } else {
                Err(syn::Error::new_spanned(expr, "expected integer literal"))
            }
        }
        _ => Err(syn::Error::new_spanned(expr, "expected integer literal")),
    }
}

fn parse_bool_value(expr: &syn::Expr) -> syn::Result<bool> {
    match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Bool(lit_bool),
            ..
        }) => Ok(lit_bool.value),
        _ => Err(syn::Error::new_spanned(expr, "expected boolean literal")),
    }
}
