use tested_trait::{test_impl, tested_trait};

fn main() {
    #[tested_trait]
    trait Foo<T> {}

    #[test_impl]
    #[in_integration_test]
    impl<T> Foo<T> for () {}
}
