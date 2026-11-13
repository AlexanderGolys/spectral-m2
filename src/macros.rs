/// span macro
#[macro_export]
macro_rules! span {
    ($x:expr, $grp:expr) => {
        Span::styled($x.to_string(), $grp)
    };
}
