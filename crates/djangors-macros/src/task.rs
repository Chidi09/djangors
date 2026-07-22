use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse2, ItemFn, LitStr};

struct TaskArgs {
    name: Option<String>,
}

fn parse_task_args(attr: TokenStream2) -> syn::Result<TaskArgs> {
    if attr.is_empty() {
        return Ok(TaskArgs { name: None });
    }
    let mut name = None;
    let attr_parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("name") {
            let lit: LitStr = meta.value()?.parse()?;
            name = Some(lit.value());
            Ok(())
        } else {
            Err(meta.error(
                "unrecognized option for #[task] attribute macro (expected `name = \"...\"`)",
            ))
        }
    });
    syn::parse::Parser::parse2(attr_parser, attr)?;
    Ok(TaskArgs { name })
}

pub fn expand_task(attr: TokenStream2, item: TokenStream2) -> syn::Result<TokenStream2> {
    let args = parse_task_args(attr)?;
    let input_fn = parse2::<ItemFn>(item)?;

    let fn_ident = &input_fn.sig.ident;
    let task_name_str = args.name.unwrap_or_else(|| fn_ident.to_string());
    let wrapper_ident = quote::format_ident!("__djangors_task_wrapper_{}", fn_ident);

    let (payload_deserialize_stmt, call_arg) = match input_fn.sig.inputs.first() {
        Some(syn::FnArg::Typed(pat_type)) => {
            let ty = &pat_type.ty;
            (
                quote! {
                    let payload: #ty = djangors_tasks::serde_json::from_value(val)
                        .map_err(|e| djangors_tasks::TaskError::PayloadDeserialization(e.to_string()))?;
                },
                quote! { payload },
            )
        }
        Some(syn::FnArg::Receiver(r)) => {
            return Err(syn::Error::new_spanned(
                r,
                "#[task] attribute cannot be applied to self methods",
            ));
        }
        None => (quote! {}, quote! {}),
    };

    Ok(quote! {
        #input_fn

        #[allow(non_snake_case)]
        pub fn #wrapper_ident(
            val: djangors_tasks::serde_json::Value,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), djangors_tasks::TaskError>> + Send>> {
            Box::pin(async move {
                #payload_deserialize_stmt
                #fn_ident(#call_arg).await
            })
        }

        djangors_tasks::inventory::submit! {
            djangors_tasks::TaskRegistration {
                name: #task_name_str,
                handler: #wrapper_ident,
            }
        }
    })
}
