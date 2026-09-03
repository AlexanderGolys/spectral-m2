use these_macros_should_be_illegal::discriminated_str;

#[discriminated_str(symbol)]
#[allow(dead_code)]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Tab = "󰌒",
    Enter = "󰌑",
    Space = "󱁐",
    Backspace = "",
    Left = "",
    Right = "",
    Up = "",
    Down = "",
    Shift = "󰘶",
    Opt = "󰘵",
    Cmd = "󰘳",
    Symbol = "",
}
