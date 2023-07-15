use tested_trait::tested_trait;

fn main() {
    #[tested_trait]
    trait Foo {
        #[test]
        fn test() {}
        #[test]
        fn test() {}
    }
}
