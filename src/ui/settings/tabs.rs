//! The settings picker: one tab for each section of [`Settings`], and the button
//! restoring their defaults.

use ratatui::text::Span;

use super::appearance::AppearanceSetting;
use super::keymap::{self, Keymaps};
use super::layout::LayoutSetting;
use super::rendering::RenderingSetting;
use super::selection::SelectionSetting;
use super::statusline;
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

/// Every tab, its rows showing `settings`.
pub fn tabs(settings: Settings, keymaps: &Keymaps) -> Vec<PickerTab> {
    vec![
        tab("appearance", AppearanceSetting::CHOICES, settings),
        tab("layout", LayoutSetting::CHOICES, settings),
        PickerTab {
            mode: None,
            label: "statusline",
            items: statusline::rows(settings),
        },
        tab("rendering", RenderingSetting::CHOICES, settings),
        tab("selection", SelectionSetting::CHOICES, settings),
        keymap::settings_tab(keymaps),
    ]
}

fn tab<Setting: SettingRow>(
    label: &'static str,
    rows: &[Setting],
    settings: Settings,
) -> PickerTab {
    PickerTab {
        mode: None,
        label,
        items: rows.iter().map(|row| row.row(settings)).collect(),
    }
}

/// The button resetting settings, keymaps and appearance.
pub fn restore_defaults() -> Box<dyn PickerItem> {
    let label = Span::styled("restore defaults", scheme().settings_button());
    Row::enter(vec![label.into()], || EditorEvent::RestoreDefaults)
        .described("reset settings, keymaps and appearance")
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_steps_back<Setting: SettingRow>(rows: &[Setting]) {
        let settings = Settings::default();
        for row in rows {
            let mut stepped = settings;
            row.adjust(&mut stepped, 1);
            row.adjust(&mut stepped, -1);
            assert_eq!(stepped, settings, "{}", row.label());
        }
    }

    #[test]
    fn stepping_any_setting_forward_then_back_restores_it() {
        assert_steps_back(AppearanceSetting::CHOICES);
        assert_steps_back(LayoutSetting::CHOICES);
        assert_steps_back(&statusline::StatusLineSetting::choices().collect::<Vec<_>>());
        assert_steps_back(RenderingSetting::CHOICES);
        assert_steps_back(SelectionSetting::CHOICES);
    }
}
