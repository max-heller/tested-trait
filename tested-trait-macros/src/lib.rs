use std::{fmt::Display, sync::atomic};

use manyhow::manyhow;
use proc_macro::TokenStream;

#[manyhow]
#[proc_macro_attribute]
pub fn tested_trait(args: TokenStream, item: TokenStream) -> manyhow::Result<TokenStream> {
    tested_trait::tested_trait(args.into(), item.into()).map(Into::into)
}

#[manyhow]
#[proc_macro_attribute]
pub fn test_impl(args: TokenStream, item: TokenStream) -> manyhow::Result<TokenStream> {
    test_impl::test_impl(args.into(), item.into()).map(Into::into)
}

struct AssociatedTestFnIdent;

impl quote::ToTokens for AssociatedTestFnIdent {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        const NAME: &str = "__internal_tested_trait_test_all";
        syn::Ident::new(NAME, proc_macro2::Span::call_site()).to_tokens(tokens);
    }
}

fn gensym() -> impl Display {
    static GENSYM: atomic::AtomicU64 = atomic::AtomicU64::new(0);
    GENSYM.fetch_add(1, atomic::Ordering::Relaxed)
}

mod tested_trait {
    use std::{borrow::Borrow, collections::HashSet};

    use manyhow::{bail, error_message};
    use proc_macro2::TokenStream;
    use quote::quote;
    use syn::{
        parse_quote, spanned::Spanned, Attribute, Block, Expr, Ident, ItemTrait, Meta,
        MetaNameValue, ReturnType, TraitItem, TraitItemFn, Type, WhereClause,
    };

    use super::AssociatedTestFnIdent;

    pub fn tested_trait(args: TokenStream, item: TokenStream) -> manyhow::Result<TokenStream> {
        let ast = parse(args, item)?;
        let model = analyze(ast)?;
        let ir = lower(model);
        Ok(codegen(ir))
    }

    const MACRO: &str = "tested_trait";

    type Ast = ItemTrait;

    #[allow(clippy::needless_pass_by_value)]
    fn parse(args: TokenStream, item: TokenStream) -> manyhow::Result<Ast> {
        if !args.is_empty() {
            bail!(args, "#[{MACRO}] takes no arguments")
        }
        syn::parse2(item)
            .map_err(|err| {
                error_message!(
                    err.span(),
                    "#[{MACRO}] can only be used to annotate trait definitions"
                )
            })
            .map_err(Into::into)
    }

    struct Model {
        tests: Vec<AssociatedTest>,
        trait_defn: ItemTrait,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct AssociatedTest {
        kind: TestKind,
        ident: Ident,
        bounds: Option<WhereClause>,
        body: Block,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum TestKind {
        Standard,
        ReturnsResult { output: Box<Type> },
        ShouldPanic { expected: Option<Expr> },
    }

    fn analyze(mut trait_defn: Ast) -> manyhow::Result<Model> {
        fn find_attr<Attrs>(attrs: Attrs, name: &str) -> Option<Attrs::Item>
        where
            Attrs: IntoIterator,
            Attrs::Item: Borrow<Attribute>,
        {
            (attrs.into_iter()).find(|attr| attr.borrow().meta.path().is_ident(name))
        }
        let is_test = |item: &TraitItemFn| match find_attr(&item.attrs, "test") {
            Some(attr) => attr.meta.require_path_only().is_ok(),
            None => false,
        };
        let num_tests = (trait_defn.items)
            .iter()
            .filter(|item| matches!(item, TraitItem::Fn(item) if is_test(item)))
            .count();
        let mut tests = Vec::with_capacity(num_tests);
        let mut items = Vec::with_capacity(trait_defn.items.len() - num_tests);
        for item in trait_defn.items {
            match item {
                TraitItem::Fn(item) if is_test(&item) => {
                    let span = item.span();
                    let body = item.default.ok_or_else(|| {
                        error_message!(span, "associated #[test]s must have a body")
                    })?;
                    let returns_result = match item.sig.output {
                        // fn test() {}
                        ReturnType::Default => None,
                        // fn test() -> () {}
                        ReturnType::Type(_, ty) if matches!(ty.as_ref(), Type::Tuple(tup) if tup.elems.is_empty()) => {
                            None
                        }
                        // fn test() -> _ {}
                        // Assume return type is a result
                        ReturnType::Type(_, ty) => Some(ty),
                    };
                    let should_panic = find_attr(item.attrs, "should_panic")
                        .map(|attr| -> manyhow::Result<_> {
                            let expected = match attr.meta {
                                Meta::Path(_) => None,
                                // #[should_panic = ""]
                                Meta::NameValue(meta) => Some(meta.value),
                                // #[should_panic(expected = "")]
                                Meta::List(meta) => {
                                    let meta = meta.parse_args::<MetaNameValue>()?;
                                    if meta.path.is_ident("expected") {
                                        Some(meta.value)
                                    } else {
                                        bail!(meta, "invalid #[should_panic] syntax")
                                    }
                                }
                            };
                            Ok(expected)
                        })
                        .transpose()?;
                    let kind = match (returns_result, should_panic) {
                        (None, None) => TestKind::Standard,
                        (None, Some(expected)) => TestKind::ShouldPanic { expected },
                        (Some(output), None) => TestKind::ReturnsResult { output },
                        (Some(_), Some(_)) => {
                            bail!(span, "#[should_panic] tests cannot return Result")
                        }
                    };
                    tests.push(AssociatedTest {
                        kind,
                        ident: item.sig.ident,
                        bounds: item.sig.generics.where_clause,
                        body,
                    });
                }
                item => items.push(item),
            }
        }
        trait_defn.items = items;

        // Check the same name isn't used for multiple tests
        let mut test_idents = HashSet::with_capacity(tests.len());
        for test in &tests {
            if !test_idents.insert(&test.ident) {
                bail!(
                    test.ident,
                    "the test `{}` is defined multiple times",
                    test.ident
                )
            }
        }

        Ok(Model { tests, trait_defn })
    }

    struct Ir {
        trait_defn: ItemTrait,
        new_trait_items: Vec<TraitItem>,
    }

    fn lower(model: Model) -> Ir {
        let Model { trait_defn, tests } = model;
        let trait_name = &trait_defn.ident;
        let run_tests = (tests.iter())
            .map(|test| {
                let AssociatedTest {
                    kind,
                    ident,
                    bounds: _,
                    body,
                } = test;
                let run_test = match kind {
                    TestKind::Standard => quote! {{
                        let (): () = #body;
                    }},
                    TestKind::ReturnsResult { output } => quote! {{
                        let result: #output = #body;
                        result.unwrap();
                    }},
                    TestKind::ShouldPanic { expected } => {
                        let check_panic = match expected {
                            Some(expected) => quote! {
                                let err = ::core::ops::Deref::deref(&err);
                                let message = <dyn ::core::any::Any>::downcast_ref::<::std::string::String>(err)
                                    .map(|s| s.as_str())
                                    .or_else(|| <dyn ::core::any::Any>::downcast_ref::<&str>(err).copied())
                                    .unwrap_or_else(|| ::core::panic!(
                                        "expected panic with string value, found non-string value"
                                    ));
                                ::core::assert!(message.contains(#expected));
                            },
                            None => quote! {},
                        };
                        quote! {{
                            match ::std::panic::catch_unwind(|| #body) {
                                ::core::result::Result::Ok(()) => {
                                    ::core::panic!("test did not panic as expected")
                                }
                                ::core::result::Result::Err(err) => {
                                    #check_panic
                                }
                            }
                        }}
                    }
                };
                quote! {
                    ::std::println!(
                        "test {}::{}",
                        ::core::stringify!(#trait_name),
                        ::core::stringify!(#ident)
                    );
                    #run_test;
                }
            });
        let bounds = (tests.iter())
            .flat_map(|test| &test.bounds)
            .flat_map(|bounds| &bounds.predicates);
        let num_tests = tests.len();
        let test_all_fn = parse_quote! {
            #[doc(hidden)]
            fn #AssociatedTestFnIdent()
            where
                Self: ::core::marker::Sized,
                #(#bounds),*
            {
                ::std::println!(
                    "running {} test{} for implementation of {}",
                    #num_tests,
                    if #num_tests == 1 { "" } else { "s" },
                    ::core::stringify!(#trait_name),
                );
                #(#run_tests)*
            }
        };

        Ir {
            trait_defn,
            new_trait_items: vec![test_all_fn],
        }
    }

    fn codegen(ir: Ir) -> TokenStream {
        let Ir {
            mut trait_defn,
            new_trait_items,
        } = ir;
        trait_defn.items.extend(new_trait_items);
        quote! { #trait_defn }
    }

    #[cfg(test)]
    mod tests {
        use quote::quote;
        use syn::{parse_quote, TraitItem, TraitItemFn};

        use crate::tested_trait::{AssociatedTest, TestKind};

        use super::{analyze, parse};

        #[test]
        fn valid_syntax() {
            parse(
                quote!(),
                quote! {
                    trait Foo {
                        fn foo() -> bool;

                        #[test]
                        fn foo_is_true() {
                            assert!(Self::foo());
                        }
                    }
                },
            )
            .unwrap();
        }

        #[test]
        fn tests_extracted_from_trait() {
            let test: TraitItemFn = parse_quote! { #[test] fn test() {} };
            let model = analyze(parse_quote! {
                trait Foo {
                    fn foo();
                    #test
                    fn bar() {}
                }
            })
            .unwrap();
            assert!(!(model.trait_defn.items).contains(&TraitItem::Fn(test.clone())));
            assert_eq!(
                [AssociatedTest {
                    kind: TestKind::Standard,
                    ident: test.sig.ident,
                    bounds: test.sig.generics.where_clause,
                    body: test.default.unwrap(),
                }]
                .as_slice(),
                model.tests
            );
        }
    }
}

mod test_impl {
    use manyhow::{bail, error_message, ResultExt};
    use proc_macro2::{Span, TokenStream};
    use quote::{quote, ToTokens};
    use syn::{
        parse::{Parse, ParseStream, Parser},
        parse_quote,
        punctuated::Punctuated,
        token::Colon,
        Block, Ident, ItemImpl, Path, ReturnType, Token, Type,
    };

    use super::AssociatedTestFnIdent;

    pub fn test_impl(args: TokenStream, item: TokenStream) -> manyhow::Result<TokenStream> {
        let ast = parse(args, item)?;
        let model = analyze(ast)?;
        let ir = lower(model);
        Ok(codegen(ir))
    }

    const MACRO: &str = "test_impl";

    struct Ast {
        trait_impl: ItemImpl,
        concrete_impls: Punctuated<ConcreteImpl, Token![,]>,
    }

    struct ConcreteImpl {
        implementer: Type,
        colon: Colon,
        trait_: Path,
    }

    impl Parse for ConcreteImpl {
        fn parse(input: ParseStream) -> syn::Result<Self> {
            Ok(Self {
                implementer: input.parse()?,
                colon: input.parse()?,
                trait_: input.parse()?,
            })
        }
    }

    impl ToTokens for ConcreteImpl {
        fn to_tokens(&self, tokens: &mut TokenStream) {
            let Self {
                implementer,
                colon,
                trait_,
            } = self;
            implementer.to_tokens(tokens);
            colon.to_tokens(tokens);
            trait_.to_tokens(tokens);
        }
    }

    fn parse(args: TokenStream, item: TokenStream) -> manyhow::Result<Ast> {
        let trait_impl = syn::parse2(item).map_err(|err| {
            error_message!(
                err.span(),
                "#[{MACRO}] can only be used to annotate trait implementations"
            )
        })?;
        let concrete_impls = Punctuated::parse_terminated
            .parse2(args)
            .map_err(|err| error_message!(err.span(), "#[{MACRO}] received invalid arguments"))?;
        Ok(Ast {
            trait_impl,
            concrete_impls,
        })
    }

    struct Model {
        trait_impl: ItemImpl,
        concrete_impls: Punctuated<ConcreteImpl, Token![,]>,
        in_integration_test: bool,
    }

    fn analyze(ast: Ast) -> manyhow::Result<Model> {
        let Ast {
            mut trait_impl,
            mut concrete_impls,
        } = ast;
        let (negative_impl, trait_, _) = trait_impl.trait_.as_ref().ok_or_else(|| {
            error_message!(
                trait_impl,
                "#[{MACRO}] can only be used to annotate trait implementations"
            )
        })?;
        if let Some(negative_impl) = negative_impl {
            bail!(
                negative_impl,
                "#[{MACRO}] does not support negative trait implementations"
            )
        }

        let implementer = trait_impl.self_ty.as_ref().clone();
        if trait_impl.generics.params.is_empty() {
            if !concrete_impls.is_empty() {
                bail!(
                    concrete_impls,
                    "#[{MACRO}] on a non-generic impl does not support specifying concrete implementations";
                )
            }
            concrete_impls.push(ConcreteImpl {
                implementer,
                colon: Colon::default(),
                trait_: trait_.clone(),
            });
        } else if concrete_impls.is_empty() {
            return Err(error_message!(
                concrete_impls,
                "#[{MACRO}] on a generic impl requires specifying concrete implementations with #[{MACRO}({implementer}: {trait_})]",
                implementer = implementer.to_token_stream(),
                trait_ = trait_.to_token_stream(),
            ))
            .context(error_message!(
                trait_impl.generics,
                "associated tests for this generic implementation can only be instantiated for concrete types"
            ));
        }

        let in_integration_test = (trait_impl.attrs.iter())
            .enumerate()
            .find_map(|(idx, attr)| {
                attr.meta
                    .path()
                    .is_ident("in_integration_test")
                    .then_some(idx)
            })
            .map(|idx| trait_impl.attrs.remove(idx))
            .is_some();

        Ok(Model {
            trait_impl,
            concrete_impls,
            in_integration_test,
        })
    }

    struct Ir {
        trait_impl: ItemImpl,
        tests: Vec<Test>,
        in_integration_test: bool,
    }

    struct Test {
        name: Ident,
        output: ReturnType,
        body: Block,
    }

    fn lower(model: Model) -> Ir {
        let Model {
            trait_impl,
            concrete_impls,
            in_integration_test,
        } = model;
        let tests = (concrete_impls.into_iter())
            .flat_map(|concrete| {
                let ConcreteImpl {
                    implementer,
                    colon: _,
                    trait_,
                } = concrete;
                let trait_name = &(trait_.segments)
                    .last()
                    .expect("trait `Path`s contain at least one segment")
                    .ident;
                [Test {
                    name: Ident::new(
                        &format!("tested_trait_test_impl_{trait_name}_{}", super::gensym()),
                        Span::call_site(),
                    ),
                    output: ReturnType::Default,
                    body: parse_quote! {{
                        <#implementer as #trait_>::#AssociatedTestFnIdent()
                    }},
                }]
            })
            .collect();
        Ir {
            trait_impl,
            tests,
            in_integration_test,
        }
    }

    fn codegen(ir: Ir) -> TokenStream {
        let Ir {
            trait_impl,
            tests,
            in_integration_test,
        } = ir;
        let test_fns = tests.iter().map(|Test { name, body, output }| {
            let test_attr = (!in_integration_test).then(|| quote! { #[test] });
            quote! {
                #test_attr
                #[doc(hidden)]
                fn #name() #output {
                    #body
                }
            }
        });
        let run_tests_manually = in_integration_test.then(|| {
            let test_names = tests.iter().map(|Test { name, .. }| name);
            quote! {
                #(#test_names();)*
            }
        });
        quote! {
            #trait_impl
            #(#test_fns)*
            #run_tests_manually
        }
    }
}
