//! Stepping through a fixed set of choices, wrapping at either end.

/// The choice `delta` steps after `current`, wrapping around `choices`.
pub fn cycled<T: Copy + PartialEq>(choices: &[T], current: T, delta: i8) -> T {
    let position = choices
        .iter()
        .position(|choice| *choice == current)
        .expect("the current choice is among the choices");
    choices[(position as isize + isize::from(delta)).rem_euclid(choices.len() as isize) as usize]
}

/// Declares an enum stepped through its variants in the order written: `CHOICES`
/// lists them, and `cycle` steps `delta` of them on, wrapping at either end.
macro_rules! cyclic {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($(#[$variant_meta:meta])* $variant:ident $(= $discriminant:tt)?),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $($(#[$variant_meta])* $variant $(= $discriminant)?),+
        }

        impl $name {
            /// Every choice, in the order stepped through.
            $vis const CHOICES: &[Self] = &[$(Self::$variant),+];

            /// The choice `delta` steps on from this one, wrapping around.
            $vis fn cycle(self, delta: i8) -> Self {
                $crate::ui::cycle::cycled(Self::CHOICES, self, delta)
            }
        }
    };
}

pub(crate) use cyclic;
