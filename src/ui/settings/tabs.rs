//! The settings picker: one tab for each section of [`Settings`], and the button
//! restoring their defaults.

use ratatui::text::Span;

use super::appearance::AppearanceSetting;
use super::editor;
use super::keymap::{self, Keymaps};
use super::lsp::LspSetting;
use super::rendering::RenderingSetting;
use super::row::SettingRow;
use super::statusline;
use super::Settings;
use crate::ui::controller::picker::{PickerItem, PickerTab, Row};
use crate::ui::controller::EditorEvent;
use crate::ui::theme::groups::scheme;

/// Every tab, its rows showing `settings`.
pub fn tabs(settings: Settings, keymaps: &Keymaps) -> Vec<PickerTab> {
    vec![
        tab("appearance", AppearanceSetting::CHOICES, settings),
        tab("rendering", RenderingSetting::CHOICES, settings),
        tab("LSP", LspSetting::CHOICES, settings),
        PickerTab {
            mode: None,
            label: "editor",
            items: editor::rows(settings),
        },
        PickerTab {
            mode: None,
            label: "statusline",
            items: statusline::rows(settings),
        },
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
