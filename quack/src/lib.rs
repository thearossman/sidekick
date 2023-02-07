pub mod arithmetic {
    mod modint;
    mod evaluator;

    pub use modint::ModularInteger;
    pub use evaluator::MonicPolynomialEvaluator;

}

mod quack;
mod decoded_quack;

pub use crate::quack::{PowerSumQuack, Identifier};
pub use decoded_quack::{DecodedQuack, IdentifierLog};
