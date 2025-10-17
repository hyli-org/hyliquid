#[derive(Clone)]
enum FieldKind {
    Commit,
    Ident,
    Plain,
}

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
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
        .iter()
        .map(|field| {
            let field_ident = field.ident.clone().unwrap();
            let ty = field.ty.clone();
            quote! { pub #field_ident: #ty }
        })
        .collect();

    let mut commit_fields: Vec<TokenStream2> = Vec::new();
    let mut zk_fields: Vec<TokenStream2> = Vec::new();

    let mut field_meta = Vec::new();

    for field in fields.iter() {
        let field_ident = field.ident.clone().unwrap();
        let ty = field.ty.clone();
        let mut field_kind = FieldKind::Plain;

        for attr in &field.attrs {
            if attr.path().is_ident("commit") {
                let args = attr
                    .parse_args::<CommitArgs>()
                    .unwrap_or_else(|err| panic!("invalid commit attribute on field: {}", err));
                if args.kind == "SMT" {
                    let commit_ident = format_ident!("{}_smt", field_ident);
                    commit_fields.push(quote! { pub #commit_ident: ::state_core::SMT<#ty> });
                    zk_fields.push(quote! { pub #field_ident: ::state_core::ZkWitnessSet<#ty> });
                    field_kind = FieldKind::Commit;
                }
            } else if attr.path().is_ident("ident") {
                let args = attr
                    .parse_args::<IdentArgs>()
                    .unwrap_or_else(|err| panic!("invalid ident attribute on field: {}", err));
                if args.kind == "borsh" {
                    zk_fields.push(quote! { pub #field_ident: #ty });
                    if !matches!(field_kind, FieldKind::Commit) {
                        field_kind = FieldKind::Ident;
                    }
                }
            }
        }

        field_meta.push((field_ident.clone(), field_kind));
    }

    let drain_fields: Vec<TokenStream2> = field_meta
        .iter()
        .map(|(ident, kind)| match kind {
            FieldKind::Commit => quote! { #ident: self.#ident.take_inner() },
            FieldKind::Ident => quote! { #ident: ::std::mem::take(&mut self.#ident) },
            FieldKind::Plain => quote! { #ident: Default::default() },
        })
        .collect();

    let load_statements: Vec<TokenStream2> = field_meta
        .iter()
        .map(|(ident, kind)| match kind {
            FieldKind::Commit => {
                quote! { self.#ident = ::state_core::ZkWitnessSet::from(state.#ident); }
            }
            FieldKind::Ident => quote! { self.#ident = state.#ident; },
            FieldKind::Plain => quote! { let _ = state.#ident; },
        })
        .collect();

    let output = quote! {
        #[allow(non_snake_case)]
        #vis mod #module_ident {
            use super::*;

            #[derive(Debug, Default, Clone)]
            pub struct ExecuteState {
                #( #execute_fields, )*
            }

            #[derive(Debug, Default, Clone)]
            pub struct FullState {
                pub execute_state: ExecuteState,
                #( #commit_fields, )*
            }

            #[derive(Debug, Default, Clone)]
            pub struct ZkVmState {
                #( #zk_fields, )*
            }

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

            pub trait WitnessBridge {
                fn drain_to_execute_state(&mut self) -> ExecuteState;
                fn populate_from_execute_state(&mut self, state: ExecuteState);
            }

            impl WitnessBridge for ZkVmState {
                fn drain_to_execute_state(&mut self) -> ExecuteState {
                    ExecuteState {
                        #( #drain_fields, )*
                    }
                }

                fn populate_from_execute_state(&mut self, mut state: ExecuteState) {
                    #( #load_statements )*
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

struct CommitArgs {
    kind: String,
}

impl Parse for CommitArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let kind: Ident = input.parse()?;
        Ok(CommitArgs {
            kind: kind.to_string(),
        })
    }
}

struct IdentArgs {
    kind: String,
}

impl Parse for IdentArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let kind: Ident = input.parse()?;
        Ok(IdentArgs {
            kind: kind.to_string(),
        })
    }
}
