//! Shared behavior for rows in the settings picker.

use ratatui::text::Span;

use super::Settings;
use crate::ui::controller::picker::{PickerItem, PickerTab, PickerValue, Row};
use crate::ui::controller::EditorEvent;
use crate::ui::theme::groups::scheme;

/// One setting listed on a tab, reading and stepping its value in [`Settings`].
pub trait SettingRow: Copy + Send + 'static {
    /// The setting's name, leading its row.
    fn label(self) -> &'static str;

    /// What the setting controls, or why its value does not take effect here,
    /// shown beneath the rows while it is selected.
    fn description(self, settings: Settings) -> &'static str;

    fn value(self, settings: Settings) -> PickerValue;

    fn adjust(self, settings: &mut Settings, delta: i8);

    /// Whether the setting takes effect, given the others.
    fn applies(self, _settings: Settings) -> bool {
        true
    }

    /// A row for each of `rows`, in order, showing `settings`.
    fn rows(rows: &[Self], settings: Settings) -> impl Iterator<Item = Box<dyn PickerItem>> {
        rows.iter().map(move |row| row.row(settings))
    }

    fn row(self, settings: Settings) -> Box<dyn PickerItem> {
        let label = Span::styled(self.label(), scheme().settings_item_label());
        Row::adjust(vec![label.into()], move |delta| {
            let mut stepped = settings;
            self.adjust(&mut stepped, delta);
            EditorEvent::SetSettings(stepped)
        })
        .with_value(self.value(settings))
        .described(self.description(settings))
        .selectable_if(self.applies(settings))
        .into()
    }
}

/// A section of [`Settings`], shown as one tab of the settings picker: the
/// section owns its tab, so neither can exist without the other.
pub trait SettingsTab {
    /// The tab's name in the picker.
    const LABEL: &'static str;

    fn rows(settings: Settings) -> Vec<Box<dyn PickerItem>>;

    fn tab(settings: Settings) -> PickerTab {
        PickerTab {
            mode: None,
            label: Self::LABEL,
            items: Self::rows(settings),
        }
    }
}
