use tested_trait::tested_trait;

fn main() {
    #[tested_trait]
    trait Foo {
        #[test]
        fn test() {
            5
        }

        #[test]
        fn test2() -> Result<(), ()> {}
    }
}
