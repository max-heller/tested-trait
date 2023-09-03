#![cfg_attr(not(any(test, doc)), no_std)]
#![deny(missing_docs, unnameable_test_items)]

//! # `tested-trait`
//!
//! `tested-trait` provides two macros -- [`tested_trait`] and [`test_impl`] -- that make it
//! possible to include associated tests in trait definitions and instantiate associated tests
//! to test implementations of the trait.
//!
//! ## Example
//!
//! Consider a memory allocator trait like [`GlobalAlloc`](core::alloc::GlobalAlloc).
//!
//! The [`alloc`](core::alloc::GlobalAlloc::alloc) method takes a [`Layout`](core::alloc::Layout)
//! describing size and alignment requirements, and returns a pointer -- the returned pointer
//! *should* adhere to layout description, but nothing enforces this contract.
//!
//! By annotating the trait definition with the [`tested_trait`] macro, a test can be associated
//! with the trait to verify that allocations result in validly aligned pointers -- at least for a
//! simple sequence of allocations:
//!
//! ```
//! # use tested_trait::{tested_trait, test_impl};
//! use std::alloc::Layout;
//!
//! #[tested_trait]
//! trait Allocator {
//!     unsafe fn alloc(&mut self, layout: Layout) -> *mut u8;
//!
//!     #[test]
//!     fn alloc_respects_alignment() where Self: Default {
//!         let mut alloc = Self::default();
//!         let layout = Layout::from_size_align(10, 4).unwrap();
//!         for _ in 0..10 {
//!             let ptr = unsafe { alloc.alloc(layout) };
//!             assert_eq!(ptr.align_offset(layout.align()), 0);
//!         }
//!     }
//! }
//! ```
//!
//! Note the test's `where Self: Default` bound, which it uses to construct an allocator.
//! Unlike freestanding `#[test]`s, associated tests may have `where` clauses to require additional
//! functionality for testing purposes.
//!
//! Implementers can then use [`test_impl`] to verify that their allocators pass this tests and any
//! others associated with the trait.
//! For instance, we can test the default system allocator:
//!
//! ```
//! # use tested_trait::{tested_trait, test_impl};
//! # use std::alloc::Layout;
//! # #[tested_trait]
//! # trait Allocator {
//! #     unsafe fn alloc(&mut self, layout: Layout) -> *mut u8;
//! #     #[test]
//! #     fn alloc_respects_alignment() where Self: Default {
//! #         let mut alloc = Self::default();
//! #         let layout = Layout::from_size_align(10, 4).unwrap();
//! #         for _ in 0..10 {
//! #             let ptr = unsafe { alloc.alloc(layout) };
//! #             assert_eq!(ptr.align_offset(layout.align()), 0);
//! #         }
//! #     }
//! # }
//! use std::alloc;
//!
//! #[test_impl]
//! # #[in_integration_test]
//! impl Allocator for alloc::System {
//!     unsafe fn alloc(&mut self, layout: Layout) -> *mut u8 {
//!         alloc::GlobalAlloc::alloc(self, layout)
//!     }
//! }
//! ```
//!
//! ... and a flawed allocator that ignores alignment:
//!
//! ```should_panic
//! # use tested_trait::{tested_trait, test_impl};
//! # use std::alloc::Layout;
//! # #[tested_trait]
//! # trait Allocator {
//! #     unsafe fn alloc(&mut self, layout: Layout) -> *mut u8;
//! #     #[test]
//! #     fn alloc_respects_alignment() where Self: Default {
//! #         let mut alloc = Self::default();
//! #         let layout = Layout::from_size_align(10, 4).unwrap();
//! #         for _ in 0..10 {
//! #             let ptr = unsafe { alloc.alloc(layout) };
//! #             assert_eq!(ptr.align_offset(layout.align()), 0);
//! #         }
//! #     }
//! # }
//! struct BadAllocator<const SIZE: usize> {
//!     buf: Box<[u8; SIZE]>,
//!     next: usize,
//! }
//!
//! // Note the `BadAllocator<1024>: Allocator` argument here -- the implementation is generic,
//! // so we use it to specify which concrete implementation should be tested.
//! #[test_impl(BadAllocator<1024>: Allocator)]
//! # #[in_integration_test]
//! impl<const SIZE: usize> Allocator for BadAllocator<SIZE> {
//!     unsafe fn alloc(&mut self, layout: Layout) -> *mut u8 {
//!         if self.next + layout.size() <= self.buf.len() {
//!             let ptr = &mut self.buf[self.next] as *mut u8;
//!             self.next += layout.size();
//!             ptr
//!         } else {
//!             core::ptr::null_mut()
//!         }
//!     }
//! }
//!
//! // Implement Default since the associated tests require it -- if this implementation
//! // is omitted, the #[test_impl] attribute will emit a compilation error.
//! impl<const SIZE: usize> Default for BadAllocator<SIZE> {
//!     fn default() -> Self {
//!         Self { buf: Box::new([0; SIZE]), next: 0 }
//!     }
//! }
//! ```
//!
//! ## Features
//!
//! - [x] Associating tests with trait definitions
//! - [x] Running associated tests against non-generic trait implementations and concrete
//!       instantiations of generic implementations (see [below](#testing-generic-implementations))
//! - [x] Most of the standard `#[test]` syntax (see [below](#supported-test-syntax))
//! - [ ] Understandable names for generated tests: currently, annotating `impl<T> Foo<T> for
//!       Bar<T>` with [`test_impl`] generates tests named `tested_trait_test_impl_Foo_{N}` --
//!       ideally they'd be named `tested_trait_test_impl_Foo<{T}>_for_Bar<{T}>`, but converting
//!       types into valid identifiers is difficult
//! - [ ] Testing trait implementations for unsized types
//! - [ ] Support for property-based tests with
//!       [`quickcheck`](https://docs.rs/quickcheck/latest/quickcheck/) and
//!       [`proptest`](https://docs.rs/proptest/latest/proptest/)
//! - [ ] `#![no_std]` support: this crate itself is `#![no-std]`, but the tests it defines require
//!       [`std::println!`] and [`std::panic::catch_unwind()`]
//!
//! ### Testing generic implementations
//!
//! Generic implementations of traits generate *concrete implementations* for each instantiation of
//! their generic parameters. It's impossible to test all of these implementations, so annotating a
//! generic implementation with *just* [`#[test_impl]`](test_impl) fails to compile:
//!
//! ```compile_fail
//! # use tested_trait::{tested_trait, test_impl};
//! #[tested_trait]
//! trait Wrapper<T> {
//!     fn wrap(value: T) -> Self;
//!     fn unwrap(self) -> T;
//!
//!     #[test]
//!     fn wrap_then_unwrap() where T: Default + PartialEq + Clone {
//!         let value = T::default();
//!         assert!(Self::wrap(value.clone()).unwrap() == value);
//!     }
//! }
//!
//! #[test_impl]
//! impl<T> Wrapper<T> for Option<T> {
//!     fn wrap(value: T) -> Self {
//!         Some(value)
//!     }
//!     fn unwrap(self) -> T {
//!         self.unwrap()
//!     }
//! }
//! ```
//!
//! To test such an implementation, pass a non-empty list of `Type: Trait` arguments to
//! [`test_impl`] to specify which concrete implementations to test:
//!
//! ```
//! # use tested_trait::{tested_trait, test_impl};
//! # #[tested_trait]
//! # trait Wrapper<T> {
//! #     fn wrap(value: T) -> Self;
//! #     fn unwrap(self) -> T;
//! #     #[test]
//! #     fn wrap_then_unwrap() where T: Default + PartialEq + Clone {
//! #         let value = T::default();
//! #         assert!(Self::wrap(value.clone()).unwrap() == value);
//! #     }
//! # }
//! #[test_impl(Option<u32>: Wrapper<u32>, Option<String>: Wrapper<String>)]
//! impl<T> Wrapper<T> for Option<T> {
//!     fn wrap(value: T) -> Self {
//!         Some(value)
//!     }
//!     fn unwrap(self) -> T {
//!         self.unwrap()
//!     }
//! }
//! ```
//!
//! ### Supported `#[test]` syntax
//!
//! Most of the standard `#[test]` syntax is supported:
//!
//! ```
//! # use tested_trait::{tested_trait, test_impl};
//! #[tested_trait]
//! trait Foo {
//!     #[test]
//!     fn standard_test() {}
//!
//!     #[test]
//!     fn result_returning_test() -> Result<(), String> {
//!         Ok(())
//!     }
//!
//!     #[test]
//!     #[should_panic]
//!     fn should_panic_test1() {
//!         panic!()
//!     }
//!
//!     #[test]
//!     #[should_panic = "ahhh"]
//!     fn should_panic_test2() {
//!         panic!("ahhhhh")
//!     }
//!
//!     #[test]
//!     #[should_panic(expected = "ahhh")]
//!     fn should_panic_test3() {
//!         panic!("ahhhhh")
//!     }
//! }
//!
//! #[test_impl]
//! # #[in_integration_test]
//! impl Foo for () {}
//! ```
//!
//! ## Comparison to `trait_tests`
//!
//! This crate provides similar functionality to the [`trait_tests`] crate, with the following
//! notable differences:
//!
//! - `trait_tests` defines tests in separate `FooTests` traits,
//!   while this crate defines them inline in trait definitions
//! - `trait_tests` allows placing bounds on `FooTests` traits,
//!   while this crate allows placing them on test functions themselves
//! - `trait_tests` defines tests as unmarked associated functions,
//!   while this crate supports the standard `#[test]` syntax and the niceties that come with it
//! - From my testing, this crate's macros are more hygienic and robust to varying inputs than those
//!   of `trait_tests`
//!
//! [`trait_tests`]: https://crates.io/crates/trait_tests

/// Compiles functions marked with `#[test]` in the definition of the annotated trait into a test
/// suite that can be instantiated with [`test_impl`] to verify an implementation of the trait.
///
/// See the [crate-level docs](crate) for examples and more details.
pub use tested_trait_macros::tested_trait;

/// Tests the annotated trait implementation against associated tests defined with [`tested_trait`].
///
/// See the [crate-level docs](crate) for examples and more details.
pub use tested_trait_macros::test_impl;

#[cfg(test)]
mod tests {
    use super::*;

    mod object_safety {
        #[super::tested_trait]
        trait Foo {}
        #[super::test_impl]
        impl Foo for () {}
        const _: &dyn Foo = &();
    }

    mod default_bound {
        #[super::tested_trait]
        trait Foo {
            fn must_be_true(&self) -> bool;

            #[test]
            fn test_simple()
            where
                Self: Default,
            {
                let foo = Self::default();
                assert!(foo.must_be_true());
            }
        }

        #[super::test_impl]
        impl Foo for () {
            fn must_be_true(&self) -> bool {
                true
            }
        }
    }

    mod should_panic {
        #[super::tested_trait]
        trait OptionLike<T> {
            const NONE: Self;
            fn unwrap(self) -> T;

            #[test]
            #[should_panic]
            fn unwrap_none() {
                Self::NONE.unwrap();
            }
        }

        #[super::test_impl(Option<()>: OptionLike<()>)]
        impl<T> OptionLike<T> for Option<T> {
            const NONE: Self = None;
            fn unwrap(self) -> T {
                self.unwrap()
            }
        }
    }

    #[test]
    #[should_panic = "test did not panic as expected"]
    fn should_panic_doesnt_panic() {
        #[tested_trait]
        trait Foo {
            #[test]
            #[should_panic]
            fn doesnt_panic() {}
        }

        #[test_impl]
        #[in_integration_test]
        impl Foo for () {}
    }

    #[test]
    fn concrete_impls() {
        #[tested_trait]
        trait Foo {}

        #[test_impl((): Foo, u32: Foo, String: Foo)]
        #[in_integration_test]
        impl<T> Foo for T {}
    }
}

#[cfg(doctest)]
mod doctests {
    // TODO: this should be a test in `tests/ui`, but stable and nightly
    // currently produce different compiler errors.
    /// ```compile_fail
    /// use tested_trait::tested_trait;
    ///
    /// fn main() {
    ///     #[tested_trait]
    ///     trait Foo {
    ///         #[test]
    ///         #[should_panic]
    ///         fn test() -> Result<(), ()> {
    ///             Ok(())
    ///         }
    ///     }
    /// }
    /// ```
    struct ShouldPanicReturnsResult;
}
