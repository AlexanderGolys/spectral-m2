use rand::seq::{IteratorRandom, SliceRandom};

const HINTS: &[&str] = &[
    "Use operator @@ to compose functions",
    "Use col/foo or foo\\col to apply function to a collection",
    "With SelfInitializingTypes you can use C-style constructor syntax",
    "You can override the default behaviour of augmented assignments",
    "Two non-mutable objects are strictly equal (===) iff all their contents are strictly equal",
    "Two mutable objects are strictly equal (===) iff they have the same address in memory",
    "Separately created polynomial rings are not strictly equal",
    "The operator == checks for mathematical equality",
    "Use the postfix (*) to make graded objects",
    "Use the operator ⧢ for shuffle products",
    "Use the operator ⊠ for exterior products",
    "Use the operator ** for tensor or Cartesian product",
    "Use the operator ^** for tensor or Cartesian powers",
    "Use the operator ++ for direct sums",
    "Use the postfix ^~ for sheafification",
    "Use the postfix ^* for pullback maps",
    "Use the postfix ^! for proper inverse image functor",
    "Use the postfix _! for proper direct image functor",
    "Use the postfix _* for pushforward or direct image",
    "Use the operator |_ for morphism restriction",
    "Use `opt >> f` to attach optional inputs to function",
    "The null coalescence `x ?? y` returns x if x is not null or error, otherwise y",
    "`x ??= y` is treated like `x ?? (x = y)` rather than `x = x ?? y`",
    "Use operator || for vertical concatenation",
];

pub(crate) fn get_hint() -> &'static str {
    HINTS
        .choose(&mut rand::thread_rng())
        .copied()
        .expect("the hint list must not be empty")
}

pub(crate) fn get_new_hint(current: &str) -> &'static str {
    HINTS
        .iter()
        .copied()
        .filter(|hint| *hint != current)
        .choose(&mut rand::thread_rng())
        .expect("the hint list must contain at least two distinct hints")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_hint_differs_from_current_hint() {
        let current = get_hint();
        assert_ne!(get_new_hint(current), current);
    }
}
