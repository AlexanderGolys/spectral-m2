//! Five independently configurable, evenly anchored statusline positions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use these_macros_should_be_illegal::discriminated_str;

use super::tabs::SettingRow;
use super::Settings;
use crate::ui::action::Action;
use crate::ui::controller::picker::{PickerInput, PickerItem, PickerValue, Row};
use crate::ui::controller::EditorEvent;
use crate::ui::cycle::cyclic;
use crate::ui::key::KeyEvent;
use crate::ui::theme::groups::scheme;
use crossterm::event::KeyCode;
use ratatui::text::Span;

/// Number of fixed positions across the statusline.
pub const STATUS_SLOT_COUNT: usize = 5;

/// Placement and content of the statusline, ordered from left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "StoredStatusLineSettings", into = "StoredStatusLineSettings")]
pub struct StatusLineSettings {
    pub position: StatusLinePosition,
    pub components: [StatusSegment; StatusComponent::CHOICES.len()],
}

impl Default for StatusLineSettings {
    fn default() -> Self {
        let mut settings = Self {
            position: StatusLinePosition::default(),
            components: [StatusSegment::Hidden; StatusComponent::CHOICES.len()],
        };
        for (component, segment) in [
            (StatusComponent::Theme, StatusSegment::Left),
            (StatusComponent::Directory, StatusSegment::LeftCentre),
            (StatusComponent::View, StatusSegment::Centre),
            (StatusComponent::State, StatusSegment::RightCentre),
            (StatusComponent::KeyHints, StatusSegment::Right),
        ] {
            settings.set_placement(component, segment);
        }
        settings
    }
}

impl StatusLineSettings {
    /// The segment assigned to this component, or hidden.
    pub fn placement(&self, component: StatusComponent) -> StatusSegment {
        self.components[component as usize]
    }

    /// Assign a component to a segment. Components sharing it retain list order.
    pub fn set_placement(&mut self, component: StatusComponent, segment: StatusSegment) {
        self.components[component as usize] = segment;
    }
}

cyclic! {
    /// A component's checklist destination.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    #[non_exhaustive]
    pub enum StatusSegment {
        #[default]
        Hidden = "off",
        Left = "L",
        LeftCentre = "LC",
        Centre = "C",
        RightCentre = "RC",
        Right = "R",
    }
}

impl StatusSegment {
    /// Zero-based anchor, absent for a hidden component.
    pub fn slot(self) -> Option<usize> {
        Self::CHOICES
            .iter()
            .position(|segment| *segment == self)
            .and_then(|index| index.checked_sub(1))
    }
}

cyclic! {
    /// Placement of the editor statusline.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    #[non_exhaustive]
    pub enum StatusLinePosition {
        #[default]
        BelowInput = "below input",
        AboveInput = "above input",
        Off = "off",
    }
}

cyclic! {
    /// Content available to every statusline position.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    #[non_exhaustive]
    pub enum StatusComponent {
        #[default]
        Empty = "empty",
        Theme = "theme",
        MathRenderer = "math renderer",
        LanguageServer = "language server",
        RenderingSettings = "rendering settings",
        Directory = "directory",
        View = "view",
        State = "session state",
        KeyHints = "key hints",
        LanguageServerVersion = "m2-ls version",
        Sp2Version = "sp2 version",
        M2Version = "M2 version",
        Versions = "versions",
        Debug = "debug",
    }
}

/// A row of the component checklist, or the statusline's screen placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StatusLineSetting {
    Position,
    Component(StatusComponent),
}

impl StatusLineSetting {
    /// Rows in the same order used when composing a segment.
    pub fn choices() -> impl Iterator<Item = Self> {
        std::iter::once(Self::Position).chain(
            StatusComponent::CHOICES
                .iter()
                .copied()
                .filter(|component| *component != StatusComponent::Empty)
                .map(Self::Component),
        )
    }
}

/// The statusline's checklist, with one row per available component.
pub fn rows(settings: Settings) -> Vec<Box<dyn PickerItem>> {
    StatusLineSetting::choices()
        .map(|row| row.row(settings))
        .collect()
}

impl SettingRow for StatusLineSetting {
    fn label(self) -> &'static str {
        match self {
            Self::Position => "position",
            Self::Component(component) => component.label(),
        }
    }

    fn description(self, _: Settings) -> &'static str {
        match self {
            Self::Position => "where the statusline sits",
            _ => "0 disables; 1–5 choose L, left centre, centre, right centre, R; shared content follows list order",
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        match self {
            Self::Component(component) => PickerValue::checklist(
                StatusSegment::CHOICES,
                &settings.statusline.placement(component),
                |segment| segment.label().into(),
            ),
            Self::Position => PickerValue::among(
                StatusLinePosition::CHOICES,
                &settings.statusline.position,
                |position| position.label().into(),
            ),
        }
    }

    fn adjust(self, settings: &mut Settings, delta: i8) {
        match self {
            Self::Component(component) => settings.statusline.set_placement(
                component,
                settings.statusline.placement(component).cycle(delta),
            ),
            Self::Position => {
                settings.statusline.position = settings.statusline.position.cycle(delta)
            }
        }
    }

    fn row(self, settings: Settings) -> Box<dyn PickerItem> {
        let label = Span::styled(self.label(), scheme().settings_item_label());
        Row::new(vec![label.into()], move |input| {
            let mut changed = settings;
            match input {
                PickerInput::Key(KeyEvent {
                    code: KeyCode::Left,
                    ..
                }) => self.adjust(&mut changed, -1),
                PickerInput::Key(KeyEvent {
                    code: KeyCode::Right,
                    ..
                }) => self.adjust(&mut changed, 1),
                PickerInput::Key(KeyEvent {
                    code: KeyCode::Char(digit),
                    ..
                }) => {
                    let Self::Component(component) = self else {
                        return None;
                    };
                    let destination = digit
                        .to_digit(10)
                        .and_then(|index| StatusSegment::CHOICES.get(index as usize))?;
                    changed.statusline.set_placement(component, *destination);
                }
                PickerInput::Action(Action::Submit) => return Some(EditorEvent::ClosePickers),
                _ => return None,
            }
            Some(EditorEvent::SetSettings(changed))
        })
        .with_value(self.value(settings))
        .described(self.description(settings))
        .selectable_if(self.applies(settings))
        .into()
    }

    fn applies(self, settings: Settings) -> bool {
        self == Self::Position || settings.statusline.position != StatusLinePosition::Off
    }
}

/// Saved settings from either the five-position or former three-side layout.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct StoredStatusLineSettings {
    #[serde(alias = "statusline")]
    position: StatusLinePosition,
    #[serde(skip_serializing_if = "Option::is_none")]
    components: Option<BTreeMap<StatusComponent, StatusSegment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    slots: Option<[StatusComponent; STATUS_SLOT_COUNT]>,
    #[serde(alias = "idle_statusline", skip_serializing_if = "Option::is_none")]
    idle: Option<LegacyIdleStatusLine>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LegacyPlacement {
    Left,
    Center,
    Right,
    Hidden,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct LegacyIdleStatusLine {
    scheme: LegacyPlacement,
    directory: LegacyPlacement,
    hints: LegacyPlacement,
}

impl Default for LegacyIdleStatusLine {
    fn default() -> Self {
        Self {
            scheme: LegacyPlacement::Left,
            directory: LegacyPlacement::Left,
            hints: LegacyPlacement::Right,
        }
    }
}

impl From<StoredStatusLineSettings> for StatusLineSettings {
    fn from(stored: StoredStatusLineSettings) -> Self {
        let mut settings = Self {
            position: stored.position,
            ..Self::default()
        };
        if let Some(components) = stored.components {
            settings.components.fill(StatusSegment::Hidden);
            for (component, segment) in components {
                settings.set_placement(component, segment);
            }
        } else if let Some(slots) = stored.slots {
            settings.components.fill(StatusSegment::Hidden);
            for (slot, component) in slots.into_iter().enumerate() {
                if component != StatusComponent::Empty {
                    settings.set_placement(component, StatusSegment::CHOICES[slot + 1]);
                }
            }
        } else if let Some(legacy) = stored.idle {
            for (component, placement) in [
                (StatusComponent::Theme, legacy.scheme),
                (StatusComponent::Directory, legacy.directory),
                (StatusComponent::KeyHints, legacy.hints),
            ] {
                settings.set_placement(
                    component,
                    match placement {
                        LegacyPlacement::Left => StatusSegment::Left,
                        LegacyPlacement::Center => StatusSegment::Centre,
                        LegacyPlacement::Right => StatusSegment::Right,
                        LegacyPlacement::Hidden => StatusSegment::Hidden,
                    },
                );
            }
        }
        settings
    }
}

impl From<StatusLineSettings> for StoredStatusLineSettings {
    fn from(settings: StatusLineSettings) -> Self {
        Self {
            position: settings.position,
            components: Some(
                StatusComponent::CHOICES
                    .iter()
                    .copied()
                    .filter(|component| *component != StatusComponent::Empty)
                    .map(|component| (component, settings.placement(component)))
                    .collect(),
            ),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiple_components_can_share_a_segment_and_survive_serialization() {
        let mut settings = StatusLineSettings::default();
        settings.components.fill(StatusSegment::Hidden);
        for component in [
            StatusComponent::M2Version,
            StatusComponent::Sp2Version,
            StatusComponent::LanguageServerVersion,
        ] {
            settings.set_placement(component, StatusSegment::Right);
        }
        let stored = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            serde_json::from_str::<StatusLineSettings>(&stored).unwrap(),
            settings
        );
    }

    #[test]
    fn checklist_numbers_assign_or_hide_the_selected_component() {
        let settings = Settings::default();
        for (digit, expected) in [
            ('0', StatusSegment::Hidden),
            ('1', StatusSegment::Left),
            ('3', StatusSegment::Centre),
            ('5', StatusSegment::Right),
        ] {
            let mut row = StatusLineSetting::Component(StatusComponent::Versions).row(settings);
            let event = row.handle(PickerInput::Key(KeyEvent::new(KeyCode::Char(digit))));
            let Some(EditorEvent::SetSettings(changed)) = event else {
                panic!("number did not assign a segment");
            };
            assert_eq!(
                changed.statusline.placement(StatusComponent::Versions),
                expected
            );
            assert_eq!(
                changed.statusline.placement(StatusComponent::Theme),
                settings.statusline.placement(StatusComponent::Theme)
            );
        }
    }

    #[test]
    fn a_saved_five_slot_layout_migrates_to_component_assignments() {
        let settings: StatusLineSettings = serde_json::from_str(
            r#"{"slots":["theme","directory","versions","empty","key_hints"]}"#,
        )
        .unwrap();
        assert_eq!(
            settings.placement(StatusComponent::Theme),
            StatusSegment::Left
        );
        assert_eq!(
            settings.placement(StatusComponent::Directory),
            StatusSegment::LeftCentre
        );
        assert_eq!(
            settings.placement(StatusComponent::Versions),
            StatusSegment::Centre
        );
        assert_eq!(
            settings.placement(StatusComponent::KeyHints),
            StatusSegment::Right
        );
    }

    #[test]
    fn component_rows_lock_while_the_statusline_is_off() {
        let mut settings = Settings::default();
        settings.statusline.position = StatusLinePosition::Off;
        for row in StatusLineSetting::choices() {
            assert_eq!(row.applies(settings), row == StatusLineSetting::Position);
        }
    }
}
