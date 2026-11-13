//! The width a view lays its content out at while the pane around it is resized.

use std::time::{Duration, Instant};

use crate::ui::config::UI;

/// The width a view's content is laid out at. It follows the pane's width only
/// once that holds still for [`ResizeConfig::settle`], or every
/// [`ResizeConfig::max_stale`] while it keeps moving; until then the content
/// keeps the layout it has, cropped or padded to the pane, so a border being
/// dragged moves at once instead of waiting for every block to wrap again.
///
/// [`ResizeConfig::settle`]: crate::ui::config::ResizeConfig::settle
/// [`ResizeConfig::max_stale`]: crate::ui::config::ResizeConfig::max_stale
///
/// ```ignore
/// let mut width = SettlingWidth::default();
/// assert_eq!(width.width(80), 80);
/// assert_eq!(width.width(79), 80); // still dragging: the old layout stays
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct SettlingWidth {
    /// The width content was last laid out at.
    laid: Option<u16>,
    /// The pane's latest width, differing from `laid`, and when it took it.
    pending: Option<(u16, Instant)>,
    /// When the content first fell behind the pane.
    stale_since: Option<Instant>,
}

impl SettlingWidth {
    /// The width to lay content out at for a pane `pane` columns wide.
    pub fn width(&mut self, pane: u16) -> u16 {
        let now = Instant::now();
        let Some(laid) = self.laid.filter(|laid| *laid != pane) else {
            return self.lay(pane);
        };
        if self.pending.is_none_or(|(width, _)| width != pane) {
            self.pending = Some((pane, now));
        }
        self.stale_since.get_or_insert(now);
        match self.remaining(now) {
            Some(Duration::ZERO) => self.lay(pane),
            _ => laid,
        }
    }

    /// How long until the content follows the pane, when it lags behind it.
    pub fn settles_in(&self) -> Option<Duration> {
        self.remaining(Instant::now())
    }

    /// Whether the content is due to follow the pane at the next drawing.
    pub fn due(&self) -> bool {
        self.settles_in() == Some(Duration::ZERO)
    }

    fn remaining(&self, now: Instant) -> Option<Duration> {
        let (_, changed) = self.pending?;
        let held = UI.resize.settle.saturating_sub(now - changed);
        let stale = self.stale_since.map_or(Duration::MAX, |since| {
            UI.resize.max_stale.saturating_sub(now - since)
        });
        Some(held.min(stale))
    }

    fn lay(&mut self, pane: u16) -> u16 {
        *self = Self {
            laid: Some(pane),
            pending: None,
            stale_since: None,
        };
        pane
    }
}

#[cfg(test)]
mod tests {
    use std::thread::sleep;

    use super::*;

    #[test]
    fn a_moving_width_keeps_the_old_layout_until_it_holds_still() {
        let mut width = SettlingWidth::default();
        assert_eq!(width.width(80), 80);
        assert_eq!(width.settles_in(), None);
        assert_eq!(width.width(79), 80);
        assert!(width
            .settles_in()
            .is_some_and(|left| left <= UI.resize.settle));
        sleep(UI.resize.settle);
        assert!(width.due());
        assert_eq!(width.width(79), 79);
        assert_eq!(width.settles_in(), None);
    }

    #[test]
    fn a_long_resize_still_follows_the_pane_now_and_then() {
        let mut width = SettlingWidth::default();
        width.width(80);
        let start = Instant::now();
        let mut pane = 80;
        let mut followed = false;
        while start.elapsed() < UI.resize.max_stale + UI.resize.settle {
            pane -= 1;
            followed |= width.width(pane) == pane;
            sleep(UI.resize.settle / 4);
        }
        assert!(
            followed,
            "a resize longer than the staleness limit is followed"
        );
    }
}
