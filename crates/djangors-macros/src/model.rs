use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, Ident, LitInt, LitStr, Type};

pub fn expand_derive_model(input: DeriveInput) -> syn::Result<TokenStream> {
    // 1. Parse struct-level attributes
    let mut app = None;
    let mut ordering = None;
    let mut table_name = None;
    let mut unique_together = None;

    let struct_name_ident = &input.ident;
    let struct_name = struct_name_ident.to_string();

    for attr in &input.attrs {
        if attr.path().is_ident("djangors") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("app") {
                    let lit: LitStr = meta.value()?.parse()?;
                    app = Some(lit.value());
                } else if meta.path.is_ident("ordering") {
                    let expr: syn::Expr = meta.value()?.parse()?;
                    match expr {
                        syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) => {
                            let fields: Vec<String> = lit_str.value()
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                            ordering = Some(fields);
                        }
                        syn::Expr::Array(expr_array) => {
                            let mut fields = Vec::new();
                            for el in expr_array.elems {
                                if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) = el {
                                    fields.push(lit_str.value());
                                } else {
                                    return Err(syn::Error::new_spanned(el, "ordering elements must be string literals"));
                                }
                            }
                            ordering = Some(fields);
                        }
                        _ => {
                            return Err(syn::Error::new_spanned(expr, "ordering must be a string literal or an array of string literals"));
                        }
                    }
                } else if meta.path.is_ident("table_name") {
                    let lit: LitStr = meta.value()?.parse()?;
                    table_name = Some(lit.value());
                } else if meta.path.is_ident("unique_together") {
                    let expr: syn::Expr = meta.value()?.parse()?;
                    let mut ut_outer = Vec::new();
                    match expr {
                        syn::Expr::Array(expr_array) => {
                            for el in expr_array.elems {
                                let mut ut_inner = Vec::new();
                                match el {
                                    syn::Expr::Array(inner_array) => {
                                        for inner_el in inner_array.elems {
                                            if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) = inner_el {
                                                ut_inner.push(lit_str.value());
                                            } else {
                                                return Err(syn::Error::new_spanned(inner_el, "unique_together elements must be string literals"));
                                            }
                                        }
                                    }
                                    syn::Expr::Tuple(inner_tuple) => {
                                        for inner_el in inner_tuple.elems {
                                            if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) = inner_el {
                                                ut_inner.push(lit_str.value());
                                            } else {
                                                return Err(syn::Error::new_spanned(inner_el, "unique_together elements must be string literals"));
                                            }
                                        }
                                    }
                                    _ => {
                                        return Err(syn::Error::new_spanned(el, "unique_together must contain arrays or tuples of string literals"));
                                    }
                                }
                                ut_outer.push(ut_inner);
                            }
                        }
                        _ => {
                            return Err(syn::Error::new_spanned(expr, "unique_together must be an array of arrays/tuples of string literals"));
                        }
                    }
                    unique_together = Some(ut_outer);
                }
                Ok(())
            })?;
        }
    }

    let app_label = match app {
        Some(a) => a,
        None => {
            return Err(syn::Error::new_spanned(
                struct_name_ident,
                "models must specify #[djangors(app = \"...\")]",
            ));
        }
    };

    let table_name =
        table_name.unwrap_or_else(|| format!("{}_{}", app_label, snake_case(&struct_name)));

    // 2. Parse struct fields
    let data_struct = match input.data {
        Data::Struct(s) => s,
        _ => {
            return Err(syn::Error::new_spanned(
                struct_name_ident,
                "Model derive macro only supports structs",
            ))
        }
    };

    let named_fields = match data_struct.fields {
        Fields::Named(f) => f,
        _ => {
            return Err(syn::Error::new_spanned(
                struct_name_ident,
                "Model derive macro only supports structs with named fields",
            ))
        }
    };

    struct ParsedField {
        ident: Ident,
        primary_key: bool,
        field_meta_tokens: TokenStream,
    }

    struct ParsedRelation {
        relation_meta_tokens: TokenStream,
    }

    let mut parsed_fields = Vec::new();
    let mut parsed_relations = Vec::new();
    let mut column_names = std::collections::HashMap::new();

    for field in named_fields.named {
        let field_ident = field
            .ident
            .clone()
            .ok_or_else(|| syn::Error::new_spanned(&field, "unnamed fields not supported"))?;
        let field_name_str = field_ident.to_string();

        // 2a. Parse field attributes
        let mut primary_key = false;
        let mut auto = false;
        let mut max_length = None;
        let mut default = None;
        let mut unique = false;
        let mut db_index = false;
        let mut verbose_name = None;
        let mut help_text = None;
        let mut column = None;
        let mut max_digits = None;
        let mut decimal_places = None;
        let mut on_delete = None;
        let mut related_name = None;

        for attr in &field.attrs {
            if attr.path().is_ident("djangors") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("primary_key") {
                        primary_key = true;
                    } else if meta.path.is_ident("auto") {
                        auto = true;
                    } else if meta.path.is_ident("max_length") {
                        let lit: LitInt = meta.value()?.parse()?;
                        max_length = Some(lit.base10_parse::<u32>()?);
                    } else if meta.path.is_ident("default") {
                        let expr: syn::Expr = meta.value()?.parse()?;
                        default = Some(expr);
                    } else if meta.path.is_ident("unique") {
                        unique = true;
                    } else if meta.path.is_ident("db_index") {
                        db_index = true;
                    } else if meta.path.is_ident("verbose_name") {
                        let lit: LitStr = meta.value()?.parse()?;
                        verbose_name = Some(lit.value());
                    } else if meta.path.is_ident("help_text") {
                        let lit: LitStr = meta.value()?.parse()?;
                        help_text = Some(lit.value());
                    } else if meta.path.is_ident("column") {
                        let lit: LitStr = meta.value()?.parse()?;
                        column = Some(lit.value());
                    } else if meta.path.is_ident("max_digits") {
                        let lit: LitInt = meta.value()?.parse()?;
                        max_digits = Some(lit.base10_parse::<u32>()?);
                    } else if meta.path.is_ident("decimal_places") {
                        let lit: LitInt = meta.value()?.parse()?;
                        decimal_places = Some(lit.base10_parse::<u32>()?);
                    } else if meta.path.is_ident("foreign_key") {
                        meta.parse_nested_meta(|nested| {
                            if nested.path.is_ident("on_delete") {
                                let lit: LitStr = nested.value()?.parse()?;
                                on_delete = Some(lit.value());
                            } else if nested.path.is_ident("related_name") {
                                let lit: LitStr = nested.value()?.parse()?;
                                related_name = Some(lit.value());
                            } else if nested.path.is_ident("to") {
                                // ignore/consume
                                let _: syn::Expr = nested.value()?.parse()?;
                            } else {
                                return Err(nested.error("unrecognized foreign_key option"));
                            }
                            Ok(())
                        })?;
                    }
                    Ok(())
                })?;
            }
        }

        let final_column = column.clone().unwrap_or_else(|| field_name_str.clone());
        if column_names
            .insert(final_column.clone(), field.span())
            .is_some()
        {
            return Err(syn::Error::new_spanned(
                &field_ident,
                format!("duplicate column name '{}'", final_column),
            ));
        }

        // 2b. Check if ForeignKey relation
        if let Some(target_ty) = get_foreign_key_target(&field.ty) {
            // It is a relation!
            let on_delete_enum = match on_delete.as_deref().map(|s| s.to_lowercase()).as_deref() {
                Some("cascade") => quote! { djangors_orm::OnDelete::Cascade },
                Some("protect") => quote! { djangors_orm::OnDelete::Protect },
                Some("set_null") => quote! { djangors_orm::OnDelete::SetNull },
                Some("restrict") => quote! { djangors_orm::OnDelete::Restrict },
                Some("do_nothing") => quote! { djangors_orm::OnDelete::DoNothing },
                Some(other) => {
                    return Err(syn::Error::new_spanned(
                        &field_ident,
                        format!("invalid on_delete value '{}'. Valid options are: cascade, protect, set_null, restrict, do_nothing", other)
                    ));
                }
                None => quote! { djangors_orm::OnDelete::Cascade },
            };

            let related_name_tok = match related_name {
                Some(rn) => quote! { Some(#rn) },
                None => quote! { None },
            };

            // target resolves to `TargetType::meta`
            // Note: drops the redundant `to = Question` attribute since the field's own type ForeignKey<Question> already says it.
            let relation_meta_tokens = quote! {
                djangors_orm::RelationMeta {
                    field_name: #field_name_str,
                    kind: djangors_orm::RelationKind::ForeignKey,
                    target: <#target_ty as djangors_orm::Model>::meta,
                    on_delete: #on_delete_enum,
                    related_name: #related_name_tok,
                }
            };
            parsed_relations.push(ParsedRelation {
                relation_meta_tokens,
            });
        } else {
            // It is a regular field!
            let (inner_ty, nullable) = resolve_option_type(&field.ty);
            let last_ident = get_last_path_segment_ident(inner_ty);
            let is_string = last_ident.map(|id| id == "String").unwrap_or(false);

            if max_length.is_some() && !is_string {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "max_length is only valid on String fields",
                ));
            }

            if last_ident.map(|id| id == "Decimal").unwrap_or(false) {
                if max_digits.is_none() && decimal_places.is_none() {
                    return Err(syn::Error::new_spanned(
                        &field_ident,
                        "Decimal field is missing max_digits and decimal_places",
                    ));
                } else if max_digits.is_none() {
                    return Err(syn::Error::new_spanned(
                        &field_ident,
                        "Decimal field is missing max_digits",
                    ));
                } else if decimal_places.is_none() {
                    return Err(syn::Error::new_spanned(
                        &field_ident,
                        "Decimal field is missing decimal_places",
                    ));
                }
            }

            let kind_token = match last_ident.map(|id| id.to_string()).as_deref() {
                Some("String") => {
                    if max_length.is_some() {
                        quote! { djangors_orm::FieldKind::Char }
                    } else {
                        quote! { djangors_orm::FieldKind::Text }
                    }
                }
                Some("i32") => quote! { djangors_orm::FieldKind::Integer },
                Some("i64") => quote! { djangors_orm::FieldKind::BigInt },
                Some("f32") | Some("f64") => quote! { djangors_orm::FieldKind::Float },
                Some("bool") => quote! { djangors_orm::FieldKind::Boolean },
                Some("DateTime") => quote! { djangors_orm::FieldKind::DateTime },
                Some("NaiveDate") => quote! { djangors_orm::FieldKind::Date },
                Some("NaiveTime") => quote! { djangors_orm::FieldKind::Time },
                Some("Duration") => quote! { djangors_orm::FieldKind::Duration },
                Some("Uuid") => quote! { djangors_orm::FieldKind::Uuid },
                Some("Decimal") => {
                    let md = max_digits.unwrap_or(0);
                    let dp = decimal_places.unwrap_or(0);
                    quote! { djangors_orm::FieldKind::Decimal { max_digits: #md, decimal_places: #dp } }
                }
                Some(other) => {
                    return Err(syn::Error::new_spanned(
                        inner_ty,
                        format!("unsupported model field type '{}'", other),
                    ));
                }
                None => {
                    return Err(syn::Error::new_spanned(
                        inner_ty,
                        "unsupported model field type",
                    ));
                }
            };

            let default_tok = match default {
                Some(expr) => parse_default_value(&expr)?,
                None => quote! { djangors_orm::DefaultValue::None },
            };

            let max_length_tok = match max_length {
                Some(ml) => quote! { Some(#ml) },
                None => quote! { None },
            };

            let verbose_name_tok = match verbose_name {
                Some(vn) => quote! { Some(#vn) },
                None => quote! { None },
            };

            let help_text_tok = match help_text {
                Some(ht) => quote! { Some(#ht) },
                None => quote! { None },
            };

            // primary_key implies unique and db_index
            let unique_val = unique || primary_key;
            let db_index_val = db_index || primary_key;

            let field_meta_tokens = quote! {
                djangors_orm::FieldMeta {
                    name: #field_name_str,
                    column_name: #final_column,
                    kind: #kind_token,
                    nullable: #nullable,
                    primary_key: #primary_key,
                    auto: #auto,
                    unique: #unique_val,
                    db_index: #db_index_val,
                    default: #default_tok,
                    max_length: #max_length_tok,
                    verbose_name: #verbose_name_tok,
                    help_text: #help_text_tok,
                    choices: &[],
                }
            };

            parsed_fields.push(ParsedField {
                ident: field_ident,
                primary_key,
                field_meta_tokens,
            });
        }
    }

    // 3. Validation: exactly one primary key field
    let pk_fields: Vec<&Ident> = parsed_fields
        .iter()
        .filter(|f| f.primary_key)
        .map(|f| &f.ident)
        .collect();
    if pk_fields.is_empty() {
        return Err(syn::Error::new_spanned(
            struct_name_ident,
            "model must have exactly one #[djangors(primary_key)] field",
        ));
    } else if pk_fields.len() > 1 {
        let names: Vec<String> = pk_fields.iter().map(|id| id.to_string()).collect();
        return Err(syn::Error::new_spanned(
            struct_name_ident,
            format!(
                "model has multiple primary key fields: {}",
                names.join(", ")
            ),
        ));
    }

    // 4. Generate metadata lists
    let fields_expanded = parsed_fields.iter().map(|f| &f.field_meta_tokens);
    let relations_expanded = parsed_relations.iter().map(|r| &r.relation_meta_tokens);

    let ordering_tok = match ordering {
        Some(ord) => {
            let items = ord.iter().map(|o| quote! { #o });
            quote! { &[ #(#items),* ] }
        }
        None => quote! { &[] },
    };

    let unique_together_tok = match unique_together {
        Some(ut) => {
            let inner_lists = ut.iter().map(|list| {
                let items = list.iter().map(|item| quote! { #item });
                quote! { &[ #(#items),* ] }
            });
            quote! { &[ #(#inner_lists),* ] }
        }
        None => quote! { &[] },
    };

    Ok(quote! {
        impl #struct_name_ident {
            pub fn meta() -> &'static djangors_orm::ModelMeta {
                static META: std::sync::OnceLock<djangors_orm::ModelMeta> = std::sync::OnceLock::new();
                META.get_or_init(|| djangors_orm::ModelMeta {
                    struct_name: #struct_name,
                    app_label: #app_label,
                    table_name: #table_name,
                    fields: &[
                        #(#fields_expanded),*
                    ],
                    relations: &[
                        #(#relations_expanded),*
                    ],
                    indexes: &[],
                    unique_together: #unique_together_tok,
                    ordering: #ordering_tok,
                })
            }
        }

        impl djangors_orm::Model for #struct_name_ident {
            fn meta() -> &'static djangors_orm::ModelMeta {
                #struct_name_ident::meta()
            }
        }

        djangors_orm::inventory::submit! {
            djangors_orm::ModelRegistration {
                meta_fn: #struct_name_ident::meta,
            }
        }
    })
}

fn get_foreign_key_target(ty: &Type) -> Option<&Type> {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "ForeignKey" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(syn::GenericArgument::Type(target_ty)) = args.args.first() {
                        return Some(target_ty);
                    }
                }
            }
        }
    }
    None
}

fn resolve_option_type(ty: &Type) -> (&Type, bool) {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Option" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
                        return (inner_ty, true);
                    }
                }
            }
        }
    }
    (ty, false)
}

fn get_last_path_segment_ident(ty: &Type) -> Option<&Ident> {
    if let Type::Path(type_path) = ty {
        type_path.path.segments.last().map(|seg| &seg.ident)
    } else {
        None
    }
}

fn parse_default_value(expr: &syn::Expr) -> syn::Result<proc_macro2::TokenStream> {
    match expr {
        syn::Expr::Lit(syn::ExprLit { lit, .. }) => match lit {
            syn::Lit::Str(lit_str) => Ok(quote! { djangors_orm::DefaultValue::Text(#lit_str) }),
            syn::Lit::Bool(lit_bool) => Ok(quote! { djangors_orm::DefaultValue::Bool(#lit_bool) }),
            syn::Lit::Int(lit_int) => {
                let val: i64 = lit_int.base10_parse()?;
                Ok(quote! { djangors_orm::DefaultValue::I64(#val) })
            }
            syn::Lit::Float(lit_float) => {
                let val: f64 = lit_float.base10_parse()?;
                Ok(quote! { djangors_orm::DefaultValue::F64(#val) })
            }
            _ => Err(syn::Error::new_spanned(
                lit,
                "unsupported default value literal",
            )),
        },
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
                let neg_val = -val;
                Ok(quote! { djangors_orm::DefaultValue::I64(#neg_val) })
            } else if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Float(lit_float),
                ..
            }) = &**inner_expr
            {
                let val: f64 = lit_float.base10_parse()?;
                let neg_val = -val;
                Ok(quote! { djangors_orm::DefaultValue::F64(#neg_val) })
            } else {
                Err(syn::Error::new_spanned(
                    expr,
                    "unsupported default value expression",
                ))
            }
        }
        _ => Err(syn::Error::new_spanned(
            expr,
            "unsupported default value expression",
        )),
    }
}

fn snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}
