use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Error, Fields, Ident, ItemStruct, Result, Token, Type};

#[proc_macro_attribute]
pub fn vapp_state(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = match syn::parse::<MacroArgs>(attr) {
        Ok(args) => args,
        Err(err) => return err.to_compile_error().into(),
    };

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

    let action_ty = args.action;
    let event_ty = args.event;

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
            use super::*;

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

            pub type Action = super::#action_ty;
            pub type Event = super::#event_ty;

            pub trait Logic {
                fn compute_events(&self, action: &Action) -> Vec<Event>;
                fn apply_events(&mut self, events: &[Event]);
            }

            #[allow(dead_code)]
            const _: fn() = || {
                fn needs_logic<T: Logic>() {}
                needs_logic::<ExecuteState>();
            };

            impl ExecuteState {
                pub fn compute_events(&self, action: &Action) -> Vec<Event>
                where
                    Self: Logic,
                {
                    <Self as Logic>::compute_events(self, action)
                }

                pub fn apply_events(&mut self, events: &[Event])
                where
                    Self: Logic,
                {
                    <Self as Logic>::apply_events(self, events);
                }
            }

            impl FullState {
                pub fn apply_action(&mut self, action: &Action) -> Vec<Event>
                where
                    ExecuteState: Logic,
                {
                    let events = self.execute_state.compute_events(action);
                    self.execute_state.apply_events(&events);
                    events
                }
            }
        }
    };

    output.into()
}

struct MacroArgs {
    action: Type,
    event: Type,
}

impl Parse for MacroArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut action: Option<Type> = None;
        let mut event: Option<Type> = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            let _: Token![=] = input.parse()?;
            let ty: Type = input.parse()?;

            match ident.to_string().as_str() {
                "action" => action = Some(ty),
                "event" => event = Some(ty),
                other => {
                    return Err(Error::new(
                        ident.span(),
                        format!("unknown argument `{}` for vapp_state", other),
                    ));
                }
            }

            if input.peek(Token![,]) {
                let _: Token![,] = input.parse()?;
            }
        }

        match (action, event) {
            (Some(action), Some(event)) => Ok(MacroArgs { action, event }),
            _ => Err(Error::new(
                Span::call_site(),
                "vapp_state requires `action = ...` and `event = ...` arguments",
            )),
        }
    }
}
