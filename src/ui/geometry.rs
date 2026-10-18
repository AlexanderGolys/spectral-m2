//! Screen positions and the displacements between them.
//!
//! A position and a shift are different things, so they are different types: two
//! positions subtract to a shift, a shift moves a position, and adding two positions
//! is unrepresentable rather than merely wrong. Shifts are signed, which is what lets
//! a parent place a child above its own origin — the case a scrolled viewport needs
//! when a child's first visible row is not its first row.

use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

use ratatui::layout::Rect;

/// Extend a by-value binary operator to its borrowed and compound-assignment forms.
///
/// Rust auto-refs method receivers but not operands, and offers no way to derive
/// `AddAssign` from `Add`, so one conceptual operation costs up to six impls. Write the
/// owned one by hand and name it here; both operands must be `Copy`.
///
/// The compound-assignment arm only accepts an operation whose output is its left
/// operand, so it refuses to generate `Coords -= Coords` — a subtraction that leaves
/// the affine space cannot be written back into a position.
macro_rules! forward_binop {
    ($trait:ident::$method:ident, $lhs:ty, $rhs:ty) => {
        impl $trait<&$rhs> for $lhs {
            type Output = <$lhs as $trait<$rhs>>::Output;

            fn $method(self, rhs: &$rhs) -> Self::Output {
                $trait::$method(self, *rhs)
            }
        }

        impl $trait<$rhs> for &$lhs {
            type Output = <$lhs as $trait<$rhs>>::Output;

            fn $method(self, rhs: $rhs) -> Self::Output {
                $trait::$method(*self, rhs)
            }
        }

        impl $trait<&$rhs> for &$lhs {
            type Output = <$lhs as $trait<$rhs>>::Output;

            fn $method(self, rhs: &$rhs) -> Self::Output {
                $trait::$method(*self, *rhs)
            }
        }
    };
    ($trait:ident::$method:ident, $assign:ident::$assign_method:ident, $lhs:ty, $rhs:ty) => {
        forward_binop!($trait::$method, $lhs, $rhs);

        impl $assign<$rhs> for $lhs {
            fn $assign_method(&mut self, rhs: $rhs) {
                *self = $trait::$method(*self, rhs);
            }
        }

        impl $assign<&$rhs> for $lhs {
            fn $assign_method(&mut self, rhs: &$rhs) {
                *self = $trait::$method(*self, *rhs);
            }
        }
    };
}

/// A cell position, always inside some buffer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Coords {
    pub x: u16,
    pub y: u16,
}

impl Coords {
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }

    pub const fn is_in(self, rect: Rect) -> bool {
        self.x >= rect.x && self.x < rect.right() && self.y >= rect.y && self.y < rect.bottom()
    }

    /// The rectangle spanned by this position and an opposite corner, in either order.
    pub fn rect(self, opposite: Self) -> Rect {
        Rect::new(
            self.x.min(opposite.x),
            self.y.min(opposite.y),
            self.x.abs_diff(opposite.x),
            self.y.abs_diff(opposite.y),
        )
    }
}

/// A signed displacement between two positions.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct AffineShift {
    pub x: i16,
    pub y: i16,
}

impl AffineShift {
    pub const fn new(x: i16, y: i16) -> Self {
        Self { x, y }
    }
}

/// Signed distance from `from` to `to`, saturating rather than wrapping at the cast.
fn distance(to: u16, from: u16) -> i16 {
    let magnitude = i16::try_from(to.abs_diff(from)).unwrap_or(i16::MAX);
    if to >= from {
        magnitude
    } else {
        -magnitude
    }
}

impl Add<AffineShift> for Coords {
    type Output = Self;

    fn add(self, rhs: AffineShift) -> Self {
        Self::new(
            self.x.saturating_add_signed(rhs.x),
            self.y.saturating_add_signed(rhs.y),
        )
    }
}

impl Sub<AffineShift> for Coords {
    type Output = Self;

    fn sub(self, rhs: AffineShift) -> Self {
        self + -rhs
    }
}

impl Sub<Coords> for Coords {
    type Output = AffineShift;

    fn sub(self, rhs: Self) -> AffineShift {
        AffineShift::new(distance(self.x, rhs.x), distance(self.y, rhs.y))
    }
}

impl Add<AffineShift> for AffineShift {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(self.x.saturating_add(rhs.x), self.y.saturating_add(rhs.y))
    }
}

impl Sub<AffineShift> for AffineShift {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x.saturating_sub(rhs.x), self.y.saturating_sub(rhs.y))
    }
}

impl Neg for AffineShift {
    type Output = Self;

    fn neg(self) -> Self {
        Self::new(self.x.saturating_neg(), self.y.saturating_neg())
    }
}

impl Add<AffineShift> for Rect {
    type Output = Self;

    fn add(self, rhs: AffineShift) -> Self {
        let origin = Coords::new(self.x, self.y) + rhs;
        Self::new(origin.x, origin.y, self.width, self.height)
    }
}

impl Sub<AffineShift> for Rect {
    type Output = Self;

    fn sub(self, rhs: AffineShift) -> Self {
        self + -rhs
    }
}

forward_binop!(Add::add, AddAssign::add_assign, Coords, AffineShift);
forward_binop!(Sub::sub, SubAssign::sub_assign, Coords, AffineShift);
forward_binop!(Sub::sub, Coords, Coords);
forward_binop!(Add::add, AddAssign::add_assign, AffineShift, AffineShift);
forward_binop!(Sub::sub, SubAssign::sub_assign, AffineShift, AffineShift);
forward_binop!(Add::add, AddAssign::add_assign, Rect, AffineShift);
forward_binop!(Sub::sub, SubAssign::sub_assign, Rect, AffineShift);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_subtract_to_the_shift_that_reproduces_them() {
        let from = Coords::new(3, 9);
        let to = Coords::new(11, 2);

        let shift = to - from;

        assert_eq!(shift, AffineShift::new(8, -7));
        assert_eq!(from + shift, to);
        assert_eq!(to - shift, from);
        assert_eq!(from - to, -shift);
    }

    #[test]
    fn a_shift_past_the_origin_saturates_instead_of_wrapping() {
        assert_eq!(
            Coords::new(2, 2) + AffineShift::new(-5, -5),
            Coords::default()
        );
        assert_eq!(
            Coords::new(u16::MAX, 0) - Coords::new(0, 0),
            AffineShift::new(i16::MAX, 0)
        );
    }

    #[test]
    fn a_rectangle_spans_its_corners_in_either_order() {
        let corner = Coords::new(2, 7);
        let opposite = Coords::new(6, 3);

        assert_eq!(corner.rect(opposite), Rect::new(2, 3, 4, 4));
        assert_eq!(opposite.rect(corner), corner.rect(opposite));
    }

    // Borrowed operands are the point here, so the lint against them does not apply.
    #[test]
    #[allow(clippy::op_ref)]
    fn forwarded_forms_agree_with_the_owned_operator() {
        let position = Coords::new(4, 4);
        let shift = AffineShift::new(1, -2);
        let mut assigned = position;

        assigned += shift;

        assert_eq!(&position + shift, position + shift);
        assert_eq!(position + &shift, position + shift);
        assert_eq!(&position + &shift, position + shift);
        assert_eq!(assigned, position + shift);
    }

    #[test]
    fn rectangles_move_without_changing_extent() {
        let rect = Rect::new(5, 5, 3, 2);

        let moved = rect + AffineShift::new(-2, 4);

        assert_eq!(moved, Rect::new(3, 9, 3, 2));
        assert_eq!(moved - AffineShift::new(-2, 4), rect);
    }
}
