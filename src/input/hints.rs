use std::fmt::{self, Display, Formatter};

use rand::seq::IteratorRandom;

/// A randomly selected Macaulay2 editing or language hint.
#[derive(Clone, Copy)]
pub(crate) struct Hint(&'static str);

impl Hint {
    const ITEMS: &'static [&'static str] = &[
        "Use operator @@ to compose functions",
        "Use col/foo or foo\\col to apply functions to a collection",
        "With SelfInitializingTypes you can use C-style constructors",
        "For mutable objects strict equality requires the same address in memory",
        "Separately created polynomial rings are not strictly equal",
        "The operator == checks for mathematical equality",
        "Use the postfix (*) to make graded objects",
        "Use the operator ⧢ for shuffle products",
        "Use the operator ⊠ for exterior products",
        "Use the operator ** for tensor or Cartesian product",
        "Use the operator ^** for tensor or Cartesian powers",
        "Use the operator ++ for direct sums",
        "Use the postfix ^~ for sheafification",
        "Use the postfix ^* for pullback or inverse image",
        "Use the postfix ^! for upper shriek functor",
        "Use the postfix _! for lower shriek functor",
        "Use the postfix _* for pushforward or direct image",
        "Use the operator |_ for morphism restriction",
        "Use `opt >> f` to attach optional inputs to a function",
        "Operator `x ?? y` returns x if it is not null or error, otherwise y",
        "`x ??= y` is treated like `x ?? (x = y)` rather than `x = x ?? y`",
        "Use operator || for vertical concatenation",
        "To reference operators as objects use the prefix `symbol`",
        "Use `help x` to see the documentation node with key x",
        "Use `help(m, T, S)` to see the docs of method `m` with domain `(T, S)`",
        "Use prefix `?` to see a brief documentation summary of an object",
        "Use `method T` to see all methods having type T as domain component",
        "Use `method foo` to see all methods installed for function `foo`",
        "Factorial has higher precedence than adjacency, so `sin(2)! == sin(6)`",
        "Newline ends expression if it can, otherwise acts as whitespace",
        "Parse the code to a Concrete Syntax Tree using `parse`",
        "Start an empty input with `/` to open spectral-m2 commands",
        "Use `/code` in the command picker to toggle submitted session code",
    ];
}

impl Default for Hint {
    fn default() -> Self {
        Self(
            Self::ITEMS
                .iter()
                .copied()
                .choose(&mut rand::thread_rng())
                .unwrap_or("Type M2 code..."),
        )
    }
}

impl Display for Hint {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}
