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
    let mut sync_statements: Vec<TokenStream2> = Vec::new();
    let mut witness_fields: Vec<TokenStream2> = Vec::new();
    let mut ident_sync_statements: Vec<TokenStream2> = Vec::new();
    let mut commit_struct_fields: Vec<TokenStream2> = Vec::new();
    let mut commit_field_exprs_full: Vec<TokenStream2> = Vec::new();
    let mut commit_field_exprs_zk: Vec<TokenStream2> = Vec::new();
    let mut commit_uses_lifetime = false;

    for field in fields_vec.iter() {
        let field_ident = field.ident.clone().unwrap();
        let ty = field.ty.clone();
        execute_fields.push(quote! { pub #field_ident: #ty });

        let mut field_kind = FieldKind::Plain;
        let mut witness_expr: Option<TokenStream2> = None;
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
                    sync_statements.push(build_commit_sync(&field_ident, &ty));
                    witness_expr = Some(build_commit_witness_expr(&field_ident, &ty));

                    let root_ty = build_commit_root_type(&ty);
                    commit_struct_fields.push(quote! { pub #field_ident: #root_ty });
                    let full_expr = build_full_commit_expr(&field_ident, &ty);
                    commit_field_exprs_full.push(quote! { #field_ident: #full_expr });

                    let zk_expr = build_witness_commit_expr(&field_ident, &ty);
                    commit_field_exprs_zk.push(quote! { #field_ident: #zk_expr });

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

                    commit_struct_fields.push(quote! { pub #field_ident: &'a #ty });
                    commit_field_exprs_full.push(quote! { #field_ident: &self.#field_ident });

                    commit_uses_lifetime = true;

                    commit_field_exprs_zk.push(quote! { #field_ident: &self.#field_ident });

                    ident_sync_statements.push(quote! {
                        self.#field_ident = self.execute_state.#field_ident.clone();
                    });

                    field_kind = FieldKind::Ident;
                    break;
                }
            }
        }

        if matches!(field_kind, FieldKind::Plain) {
            full_fields.push(quote! { pub #field_ident: #ty });
            zk_fields.push(quote! { pub #field_ident: #ty });

            commit_struct_fields.push(quote! { pub #field_ident: &'a #ty });
            commit_field_exprs_full.push(quote! { #field_ident: &self.#field_ident });

            commit_uses_lifetime = true;

            commit_field_exprs_zk.push(quote! { #field_ident: &self.#field_ident });

            ident_sync_statements.push(quote! {
                self.#field_ident = self.execute_state.#field_ident.clone();
            });
        }

        let field_witness = witness_expr.unwrap_or_else(|| {
            quote! { self.execute_state.#field_ident.clone() }
        });
        witness_fields.push(quote! { #field_ident: #field_witness });
    }

    let commit_struct_def = if commit_uses_lifetime {
        quote! {
            #[derive(Debug, PartialEq, Eq)]
            pub struct StateCommitment<'a> {
                #( #commit_struct_fields, )*
            }
        }
    } else {
        quote! {
            #[derive(Debug, PartialEq, Eq)]
            pub struct StateCommitment {
                #( #commit_struct_fields, )*
            }
        }
    };

    let commit_return_ty = if commit_uses_lifetime {
        quote! { StateCommitment<'_> }
    } else {
        quote! { StateCommitment }
    };

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

            #commit_struct_def

            pub type Action = super::#action_ty;
            pub type Event = super::#event_ty;

            pub trait Logic {
                fn compute_events(&self, action: &Action) -> Vec<Event>;
                fn apply_events(&mut self, events: &[Event]);

                fn apply_action(&mut self, action: &Action) -> Vec<Event> {
                    let events = self.compute_events(action);
                    self.apply_events(&events);
                    events
                }
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
                    Self: Logic,
                {
                    <Self as Logic>::apply_action(self, action)
                }

                pub fn commit(&self) -> #commit_return_ty {
                    StateCommitment {
                        #( #commit_field_exprs_full, )*
                    }
                }

                pub fn build_witness_state(&self, _events: &[Event]) -> ZkVmState {
                    ZkVmState {
                        #( #witness_fields, )*
                    }
                }
            }

            impl ZkVmState {
                pub fn commit(&self) -> #commit_return_ty {
                    StateCommitment {
                        #( #commit_field_exprs_zk, )*
                    }
                }
            }

            impl Logic for FullState {
                fn compute_events(&self, action: &Action) -> Vec<Event> {
                    self.execute_state.compute_events(action)
                }

                fn apply_events(&mut self, events: &[Event]) {
                    self.execute_state.apply_events(events);
                    #( #ident_sync_statements )*
                    #( #sync_statements )*
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

fn build_commit_root_type(value_ty: &Type) -> TokenStream2 {
    if let Some((key_ty, inner_ty)) = parse_hash_map(value_ty) {
        if parse_hash_map(&inner_ty).is_some() {
            quote! { ::std::collections::HashMap<#key_ty, ::state_core::BorshableH256> }
        } else {
            quote! { ::state_core::BorshableH256 }
        }
    } else {
        quote! { ::state_core::BorshableH256 }
    }
}

fn build_full_commit_expr(field_ident: &Ident, value_ty: &Type) -> TokenStream2 {
    if let Some((_key_ty, inner_ty)) = parse_hash_map(value_ty) {
        if parse_hash_map(&inner_ty).is_some() {
            quote! {
                self
                    .#field_ident
                    .iter()
                    .map(|(outer_key, tree)| (outer_key.clone(), tree.root()))
                    .collect::<::std::collections::HashMap<_, _>>()
            }
        } else {
            quote! { self.#field_ident.root() }
        }
    } else {
        quote! { self.#field_ident.root() }
    }
}

fn build_witness_commit_expr(field_ident: &Ident, value_ty: &Type) -> TokenStream2 {
    if let Some((_key_ty, inner_ty)) = parse_hash_map(value_ty) {
        if parse_hash_map(&inner_ty).is_some() {
            quote! {
                self
                    .#field_ident
                    .iter()
                    .map(|(outer_key, witness)| (outer_key.clone(), witness.compute_root()))
                    .collect::<::std::collections::HashMap<_, _>>()
            }
        } else {
            quote! { self.#field_ident.compute_root() }
        }
    } else {
        quote! { self.#field_ident.compute_root() }
    }
}

fn build_commit_sync(field_ident: &Ident, value_ty: &Type) -> TokenStream2 {
    if let Some((_key_ty, inner_ty)) = parse_hash_map(value_ty) {
        if parse_hash_map(&inner_ty).is_some() {
            quote! {
                self.#field_ident = self
                    .execute_state
                    .#field_ident
                    .iter()
                    .map(|(outer_key, inner_map)| {
                        let mut tree = ::state_core::SMT::zero();
                        if let Err(err) = tree.update_all(inner_map.values().cloned()) {
                            panic!(
                                "failed to update {} commitments for key {}: {}",
                                stringify!(#field_ident),
                                outer_key,
                                err
                            );
                        }
                        (outer_key.clone(), tree)
                    })
                    .collect();
            }
        } else {
            quote! {
                let mut tree = ::state_core::SMT::zero();
                if let Err(err) = tree.update_all(self.execute_state.#field_ident.values().cloned()) {
                    panic!(
                        "failed to update {} commitments: {}",
                        stringify!(#field_ident),
                        err
                    );
                }
                self.#field_ident = tree;
            }
        }
    } else {
        quote! {
            let mut tree = ::state_core::SMT::zero();
            if let Err(err) = tree.update_all(::std::iter::once(self.execute_state.#field_ident.clone())) {
                panic!(
                    "failed to update {} commitments: {}",
                    stringify!(#field_ident),
                    err
                );
            }
            self.#field_ident = tree;
        }
    }
}

fn build_commit_witness_expr(field_ident: &Ident, value_ty: &Type) -> TokenStream2 {
    if let Some((_key_ty, inner_ty)) = parse_hash_map(value_ty) {
        if parse_hash_map(&inner_ty).is_some() {
            quote! {
                self
                    .execute_state
                    .#field_ident
                    .iter()
                    .map(|(outer_key, inner_map)| {
                        let values = inner_map
                            .values()
                            .cloned()
                            .collect::<::std::collections::HashSet<_>>();
                        let proof = self
                            .#field_ident
                            .get(outer_key)
                            .map(|tree| ::state_core::Proof::CurrentRootHash(tree.root()))
                            .unwrap_or_default();
                        (
                            outer_key.clone(),
                            ::state_core::ZkWitnessSet {
                                values,
                                proof,
                            },
                        )
                    })
                    .collect::<::std::collections::HashMap<_, _>>()
            }
        } else {
            quote! {
                ::state_core::ZkWitnessSet {
                    values: self
                        .execute_state
                        .#field_ident
                        .values()
                        .cloned()
                        .collect::<::std::collections::HashSet<_>>(),
                    proof: ::state_core::Proof::CurrentRootHash(self.#field_ident.root()),
                }
            }
        }
    } else {
        quote! {
            {
                let mut set = ::std::collections::HashSet::new();
                set.insert(self.execute_state.#field_ident.clone());
                ::state_core::ZkWitnessSet {
                    values: set,
                    proof: ::state_core::Proof::CurrentRootHash(self.#field_ident.root()),
                }
            }
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
