use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{Error, Fields, Ident, ItemStruct};

#[proc_macro_attribute]
pub fn vapp_state(attr: TokenStream, item: TokenStream) -> TokenStream {
    let _ = attr; // simplified prototype

    let input: ItemStruct = match syn::parse(item) {
        Ok(item) => item,
        Err(err) => return err.to_compile_error().into(),
    };

    let vis = input.vis;
    let ident = input.ident;
    let fields = match input.fields {
        Fields::Named(named) => named.named,
        _ => {
            return Error::new(Span::call_site(), "vapp_state requires named fields")
                .to_compile_error()
                .into();
        }
    };

    let module_ident = Ident::new(&ident.to_string().to_lowercase(), ident.span());

    let execute_fields: Vec<_> = fields
        .into_iter()
        .map(|mut field| {
            let field_ident = field.ident.take().unwrap();
            let ty = field.ty;
            quote! { pub #field_ident: #ty }
        })
        .collect();

    let output = quote! {
        #[allow(non_snake_case)]
        #vis mod #module_ident {
            use super::{AssetInfo, Balance, UserInfo};

            #[derive(Debug, Default, Clone)]
            pub struct ExecuteState {
                #(#execute_fields,)*
            }

            #[derive(Debug, Default, Clone)]
            pub struct FullState {
                pub execute_state: ExecuteState,
            }

            #[derive(Debug, Default, Clone)]
            pub struct ZkVmState;
        }
    };

    output.into()
}
