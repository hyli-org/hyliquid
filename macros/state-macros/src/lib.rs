use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{
    AngleBracketedGenericArguments, Error, Fields, Ident, ItemStruct, PathArguments, Result, Token,
    Type, TypePath,
};

#[derive(Clone, Copy)]
enum FieldKind {
    Commit,
    Ident,
    Plain,
}

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
    let fields_vec: Vec<_> = match input.fields {
        Fields::Named(named) => named.named.into_iter().collect(),
        _ => {
            return Error::new(Span::call_site(), "vapp_state requires named fields")
                .to_compile_error()
                .into();
        }
    };

    let module_ident = Ident::new(&ident.to_string().to_lowercase(), ident.span());

    let action_ty = args.action;
    let event_ty = args.event;

    let mut execute_fields: Vec<TokenStream2> = Vec::new();
    let mut full_fields: Vec<TokenStream2> = Vec::new();
    let mut zk_fields: Vec<TokenStream2> = Vec::new();
    let mut drain_fields: Vec<TokenStream2> = Vec::new();
    let mut load_statements: Vec<TokenStream2> = Vec::new();
    let mut field_names: Vec<Ident> = Vec::new();

    for field in fields_vec.iter() {
        let field_ident = field.ident.clone().unwrap();
        field_names.push(field_ident.clone());
        let ty = field.ty.clone();
        execute_fields.push(quote! { pub #field_ident: #ty });

        let mut field_kind = FieldKind::Plain;
        for attr in &field.attrs {
            if attr.path().is_ident("commit") {
                let args = attr
                    .parse_args::<CommitArgs>()
                    .unwrap_or_else(|err| panic!("invalid commit attribute on field: {}", err));
                if args.kind == "SMT" {
                    let commit_ty = build_commit_type(&ty);
                    let witness_ty = build_witness_type(&ty);
                    full_fields.push(quote! { pub #field_ident: #commit_ty });
                    zk_fields.push(quote! { pub #field_ident: #witness_ty });
                    drain_fields.push(build_witness_drain(&field_ident, &ty));
                    load_statements.push(build_witness_load(&field_ident, &ty));
                    field_kind = FieldKind::Commit;
                    break;
                }
            } else if attr.path().is_ident("ident") {
                let args = attr
                    .parse_args::<IdentArgs>()
                    .unwrap_or_else(|err| panic!("invalid ident attribute on field: {}", err));
                if args.kind == "borsh" {
                    full_fields.push(quote! { pub #field_ident: #ty });
                    zk_fields.push(quote! { pub #field_ident: #ty });
                    drain_fields
                        .push(quote! { #field_ident: ::std::mem::take(&mut self.#field_ident) });
                    load_statements.push(quote! { self.#field_ident = #field_ident; });
                    field_kind = FieldKind::Ident;
                    break;
                }
            }
        }

        if matches!(field_kind, FieldKind::Plain) {
            full_fields.push(quote! { pub #field_ident: #ty });
            zk_fields.push(quote! { pub #field_ident: #ty });
            drain_fields.push(quote! { #field_ident: ::std::mem::take(&mut self.#field_ident) });
            load_statements.push(quote! { self.#field_ident = #field_ident; });
        }
    }

    let pattern = quote! { ExecuteState { #( #field_names, )* } };

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
                #( #full_fields, )*
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

                fn populate_from_execute_state(&mut self, state: ExecuteState) {
                    let #pattern = state;
                    #( #load_statements )*
                }
            }
        }
    };

    output.into()
}

fn build_commit_type(value_ty: &Type) -> TokenStream2 {
    if let Some((key_ty, inner_ty)) = parse_hash_map(value_ty) {
        if let Some((_inner_key, inner_value_ty)) = parse_hash_map(&inner_ty) {
            quote! { ::std::collections::HashMap<#key_ty, ::state_core::SMT<#inner_value_ty>> }
        } else {
            quote! { ::state_core::SMT<#inner_ty> }
        }
    } else {
        quote! { ::state_core::SMT<#value_ty> }
    }
}

fn build_witness_type(value_ty: &Type) -> TokenStream2 {
    if let Some((key_ty, inner_ty)) = parse_hash_map(value_ty) {
        if let Some((_inner_key, inner_value_ty)) = parse_hash_map(&inner_ty) {
            quote! { ::std::collections::HashMap<#key_ty, ::state_core::ZkWitnessSet<#inner_value_ty>> }
        } else {
            quote! { ::state_core::ZkWitnessSet<#inner_ty> }
        }
    } else {
        quote! { ::state_core::ZkWitnessSet<#value_ty> }
    }
}

fn build_witness_drain(field_ident: &Ident, value_ty: &Type) -> TokenStream2 {
    if let Some((_key_ty, inner_ty)) = parse_hash_map(value_ty) {
        if parse_hash_map(&inner_ty).is_some() {
            quote! {
                #field_ident: {
                    let map = ::std::mem::take(&mut self.#field_ident);
                    map.into_iter()
                        .map(|(key, mut witness)| {
                            let inner = witness.take_inner();
                            (key, inner)
                        })
                        .collect()
                }
            }
        } else {
            quote! { #field_ident: self.#field_ident.take_inner() }
        }
    } else {
        quote! { #field_ident: self.#field_ident.take_inner() }
    }
}

fn build_witness_load(field_ident: &Ident, value_ty: &Type) -> TokenStream2 {
    if let Some((_key_ty, inner_ty)) = parse_hash_map(value_ty) {
        if parse_hash_map(&inner_ty).is_some() {
            quote! {
                self.#field_ident = #field_ident
                    .into_iter()
                    .map(|(key, inner)| (key, ::state_core::ZkWitnessSet::from_map(inner)))
                    .collect();
            }
        } else {
            quote! {
                self.#field_ident = ::state_core::ZkWitnessSet::from_map(#field_ident);
            }
        }
    } else {
        quote! {
            self.#field_ident = ::state_core::ZkWitnessSet::from_map(#field_ident);
        }
    }
}

fn parse_hash_map(ty: &Type) -> Option<(Type, Type)> {
    if let Type::Path(TypePath { path, .. }) = ty {
        if let Some(last) = path.segments.last() {
            if last.ident == "HashMap" {
                if let PathArguments::AngleBracketed(AngleBracketedGenericArguments {
                    args, ..
                }) = &last.arguments
                {
                    let mut iter = args.iter();
                    let key_ty = match iter.next()? {
                        syn::GenericArgument::Type(t) => t.clone(),
                        _ => return None,
                    };
                    let value_ty = match iter.next()? {
                        syn::GenericArgument::Type(t) => t.clone(),
                        _ => return None,
                    };
                    return Some((key_ty, value_ty));
                }
            }
        }
    }
    None
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
