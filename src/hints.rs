use rand::seq::IteratorRandom;

const HINTS: &[&str] = &[
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
    "Type `$help` to see the list of spectral-m2 commands",
    "Use command `$code` to toggle code submitted during this session",
];

pub fn get_hint() -> String {
    HINTS
        .iter()
        .copied()
        .choose(&mut rand::thread_rng())
        .unwrap_or("Type M2 code...")
        .to_string()
}
