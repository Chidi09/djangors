use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse2, ItemFn, LitStr};

struct MgmtCmdArgs {
    name: Option<String>,
}

fn parse_mgmt_cmd_args(attr: TokenStream2) -> syn::Result<MgmtCmdArgs> {
    if attr.is_empty() {
        return Ok(MgmtCmdArgs { name: None });
    }
    let mut name = None;
    let attr_parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("name") {
            let lit: LitStr = meta.value()?.parse()?;
            name = Some(lit.value());
            Ok(())
        } else {
            Err(meta.error(
                "unrecognized option for #[management_command] attribute macro (expected `name = \"...\"`)",
            ))
        }
    });
    syn::parse::Parser::parse2(attr_parser, attr)?;
    Ok(MgmtCmdArgs { name })
}

pub fn expand_management_command(
    attr: TokenStream2,
    item: TokenStream2,
) -> syn::Result<TokenStream2> {
    let args = parse_mgmt_cmd_args(attr)?;
    let input_fn = parse2::<ItemFn>(item)?;

    let fn_ident = &input_fn.sig.ident;
    let cmd_name_str = args.name.unwrap_or_else(|| fn_ident.to_string());
    let wrapper_ident = quote::format_ident!("__djangors_mgmt_wrapper_{}", fn_ident);

    Ok(quote! {
        #input_fn

        #[allow(non_snake_case)]
        fn #wrapper_ident(
            args: Vec<String>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
            Box::pin(async move {
                #fn_ident(args).await
            })
        }

        djangors_core::inventory::submit! {
            djangors_core::ManagementCommandRegistration {
                name: #cmd_name_str,
                handler: #wrapper_ident,
            }
        }
    })
}
