//! The *statusline* tab: where the status line sits and what its idle line shows.

use serde::{Deserialize, Serialize};
use these_macros_should_be_illegal::discriminated_str;

use super::tabs::SettingRow;
use super::Settings;
use crate::ui::controller::picker::PickerValue;
use crate::ui::cycle::cyclic;

/// The values of the *statusline* tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusLineSettings {
    #[serde(alias = "statusline")]
    pub position: StatusLinePosition,
    #[serde(alias = "idle_statusline")]
    pub idle: IdleStatusLine,
}

cyclic! {
    /// Placement of the editor status line.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum StatusLinePosition {
        #[default]
        BelowInput = "below input",
        AboveInput = "above input",
        Off = "off",
    }
}

cyclic! {
    /// The side of the status line an idle segment sits on, if any.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum SegmentPlacement {
        Left = "left",
        Center = "centre",
        Right = "right",
        Hidden = "off",
    }
}

/// Where each segment sits while the session has nothing else to say. Segments
/// sharing a side keep the order of these fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IdleStatusLine {
    pub scheme: SegmentPlacement,
    pub directory: SegmentPlacement,
    pub hints: SegmentPlacement,
}

impl Default for IdleStatusLine {
    fn default() -> Self {
        Self {
            scheme: SegmentPlacement::Left,
            directory: SegmentPlacement::Left,
            hints: SegmentPlacement::Right,
        }
    }
}

cyclic! {
    /// One row of the *statusline* tab.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, PartialEq)]
    pub enum StatusLineSetting {
        Position = "position",
        Scheme = "colour scheme",
        Directory = "directory",
        Hints = "key hints",
    }
}

impl StatusLineSetting {
    /// The idle segment this row places, unless it is the line's own position.
    fn segment(self, idle: &mut IdleStatusLine) -> Option<&mut SegmentPlacement> {
        match self {
            Self::Position => None,
            Self::Scheme => Some(&mut idle.scheme),
            Self::Directory => Some(&mut idle.directory),
            Self::Hints => Some(&mut idle.hints),
        }
    }
}

impl SettingRow for StatusLineSetting {
    fn label(self) -> &'static str {
        StatusLineSetting::label(&self)
    }

    fn description(self, _: Settings) -> &'static str {
        match self {
            Self::Position => "where the status line sits",
            Self::Scheme => "where the idle line names the colour scheme",
            Self::Directory => "where the idle line names the working directory",
            Self::Hints => "where the idle line names the commands and settings keys",
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        let mut statusline = settings.statusline;
        match self.segment(&mut statusline.idle) {
            Some(segment) => PickerValue::among(SegmentPlacement::CHOICES, segment, |&placement| {
                placement.label().into()
            }),
            None => PickerValue::among(
                StatusLinePosition::CHOICES,
                &statusline.position,
                |&position| position.label().into(),
            ),
        }
    }

    fn adjust(self, settings: &mut Settings, delta: i8) {
        let statusline = &mut settings.statusline;
        match self.segment(&mut statusline.idle) {
            Some(segment) => *segment = segment.cycle(delta),
            None => statusline.position = statusline.position.cycle(delta),
        }
    }

    /// Segments need a status line to sit on.
    fn applies(self, settings: Settings) -> bool {
        matches!(self, Self::Position) || settings.statusline.position != StatusLinePosition::Off
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_rows_lock_while_the_status_line_is_off() {
        let segments = [
            StatusLineSetting::Scheme,
            StatusLineSetting::Directory,
            StatusLineSetting::Hints,
        ];
        let mut settings = Settings::default();
        for segment in segments {
            assert!(segment.row(settings).selectable());
        }
        settings.statusline.position = StatusLinePosition::Off;
        for segment in segments {
            assert!(!segment.row(settings).selectable());
        }
    }
}
