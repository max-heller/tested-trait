## `tested-trait`

`tested-trait` provides two macros -- [`tested_trait`] and [`test_impl`] -- that make it
possible to include associated tests in trait definitions and instantiate associated tests
to test implementations of the trait.

### Example

Consider a memory allocator trait like [`GlobalAlloc`](core::alloc::GlobalAlloc).

The [`alloc`](core::alloc::GlobalAlloc::alloc) method takes a [`Layout`](core::alloc::Layout)
describing size and alignment requirements, and returns a pointer -- the returned pointer
*should* adhere to layout description, but nothing enforces this contract.

By annotating the trait definition with the [`tested_trait`] macro, a test can be associated
with the trait to verify that allocations result in validly aligned pointers -- at least for a
simple sequence of allocations:

```rust
use std::alloc::Layout;

#[tested_trait]
trait Allocator {
    unsafe fn alloc(&mut self, layout: Layout) -> *mut u8;

    #[test]
    fn alloc_respects_alignment() where Self: Default {
        let mut alloc = Self::default();
        let layout = Layout::from_size_align(10, 4).unwrap();
        for _ in 0..10 {
            let ptr = unsafe { alloc.alloc(layout) };
            assert_eq!(ptr.align_offset(layout.align()), 0);
        }
    }
}
```

Note the test's `where Self: Default` bound, which it uses to construct an allocator.
Unlike freestanding `#[test]`s, associated tests may have `where` clauses to require additional
functionality for testing purposes.

Implementers can then use [`test_impl`] to verify that their allocators pass this tests and any
others associated with the trait.
For instance, we can test the default system allocator:

```rust
use std::alloc;

#[test_impl]
impl Allocator for alloc::System {
    unsafe fn alloc(&mut self, layout: Layout) -> *mut u8 {
        alloc::GlobalAlloc::alloc(self, layout)
    }
}
```

... and a flawed allocator that ignores alignment:

```rust
struct BadAllocator<const SIZE: usize> {
    buf: Box<[u8; SIZE]>,
    next: usize,
}

// Note the `BadAllocator<1024>: Allocator` argument here -- the implementation is generic,
// so we use it to specify which concrete implementation should be tested.
#[test_impl(BadAllocator<1024>: Allocator)]
impl<const SIZE: usize> Allocator for BadAllocator<SIZE> {
    unsafe fn alloc(&mut self, layout: Layout) -> *mut u8 {
        if self.next + layout.size() <= self.buf.len() {
            let ptr = &mut self.buf[self.next] as *mut u8;
            self.next += layout.size();
            ptr
        } else {
            core::ptr::null_mut()
        }
    }
}

// Implement Default since the associated tests require it -- if this implementation
// is omitted, the #[test_impl] attribute will emit a compilation error.
impl<const SIZE: usize> Default for BadAllocator<SIZE> {
    fn default() -> Self {
        Self { buf: Box::new([0; SIZE]), next: 0 }
    }
}
```

### Features

- [x] Associating tests with trait definitions
- [x] Running associated tests against non-generic trait implementations and concrete
      instantiations of generic implementations (see [below](#testing-generic-implementations))
- [x] Most of the standard `#[test]` syntax (see [below](#supported-test-syntax))
- [ ] Understandable names for generated tests: currently, annotating `impl<T> Foo<T> for
      Bar<T>` with [`test_impl`] generates tests named `tested_trait_test_impl_Foo_{N}` --
      ideally they'd be named `tested_trait_test_impl_Foo<{T}>_for_Bar<{T}>`, but converting
      types into valid identifiers is difficult
- [ ] Testing trait implementations for unsized types
- [ ] Support for property-based tests with
      [`quickcheck`](https://docs.rs/quickcheck/latest/quickcheck/) and
      [`proptest`](https://docs.rs/proptest/latest/proptest/)
- [ ] `#![no_std]` support: this crate itself is `#![no-std]`, but the tests it defines require
      [`std::println!`] and [`std::panic::catch_unwind()`]

#### Testing generic implementations

Generic implementations of traits generate *concrete implementations* for each instantiation of
their generic parameters. It's impossible to test all of these implementations, so annotating a
generic implementation with *just* [`#[test_impl]`](test_impl) fails to compile:

```compile_fail
# use tested_trait::{tested_trait, test_impl};
#[tested_trait]
trait Wrapper<T> {
    fn wrap(value: T) -> Self;
    fn unwrap(self) -> T;

    #[test]
    fn wrap_then_unwrap() where T: Default + PartialEq + Clone {
        let value = T::default();
        assert!(Self::wrap(value.clone()).unwrap() == value);
    }
}

#[test_impl]
impl<T> Wrapper<T> for Option<T> {
    fn wrap(value: T) -> Self {
        Some(value)
    }
    fn unwrap(self) -> T {
        self.unwrap()
    }
}
```

To test such an implementation, pass a non-empty list of `Type: Trait` arguments to
[`test_impl`] to specify which concrete implementations to test:

```rust
#[test_impl(Option<u32>: Wrapper<u32>, Option<String>: Wrapper<String>)]
impl<T> Wrapper<T> for Option<T> {
    fn wrap(value: T) -> Self {
        Some(value)
    }
    fn unwrap(self) -> T {
        self.unwrap()
    }
}
```

#### Supported `#[test]` syntax

Most of the standard `#[test]` syntax is supported:

```rust
#[tested_trait]
trait Foo {
    #[test]
    fn standard_test() {}

    #[test]
    fn result_returning_test() -> Result<(), String> {
        Ok(())
    }

    #[test]
    #[should_panic]
    fn should_panic_test1() {
        panic!()
    }

    #[test]
    #[should_panic = "ahhh"]
    fn should_panic_test2() {
        panic!("ahhhhh")
    }

    #[test]
    #[should_panic(expected = "ahhh")]
    fn should_panic_test3() {
        panic!("ahhhhh")
    }
}

#[test_impl]
impl Foo for () {}
```

### Comparison to `trait_tests`

This crate provides similar functionality to the [`trait_tests`] crate, with the following
notable differences:

- `trait_tests` defines tests in separate `FooTests` traits,
  while this crate defines them inline in trait definitions
- `trait_tests` allows placing bounds on `FooTests` traits,
  while this crate allows placing them on test functions themselves
- `trait_tests` defines tests as unmarked associated functions,
  while this crate supports the standard `#[test]` syntax and the niceties that come with it
- From my testing, this crate's macros are more hygienic and robust to varying inputs than those
  of `trait_tests`

[`trait_tests`]: https://crates.io/crates/trait_tests

License: MIT OR Apache-2.0
