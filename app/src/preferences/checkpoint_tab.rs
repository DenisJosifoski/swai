//! SWAI — Checkpointing Preferences Tab.
//!
//! Context checkpointing / compaction controls (Phase 24_8 feature).

use gtk::{DropDown, StringList};
use gtk4 as gtk;

use adw::prelude::*;
use adw::{ActionRow, PreferencesGroup, PreferencesPage, SwitchRow};

use swai_core::config::Config;

/// Widget handles for the Checkpointing tab.
pub struct CheckpointWidgets {
    pub enable_checkpointing_switch: SwitchRow,
    pub summarizer_model_combo: DropDown,
}

/// Build the Checkpointing preferences page.
pub fn build_checkpoint_tab(config: &Config) -> (PreferencesPage, CheckpointWidgets) {
    let page = PreferencesPage::new();
    page.set_title("Checkpointing");

    let group = PreferencesGroup::new();
    group.set_title("Context Checkpointing");

    let enable_switch = add_enable_checkpointing_row(&group, config);
    let summarizer_dropdown = add_summarizer_model_row(&group, config);

    page.add(&group);

    let widgets = CheckpointWidgets {
        enable_checkpointing_switch: enable_switch,
        summarizer_model_combo: summarizer_dropdown,
    };

    (page, widgets)
}

/// Add a switch row for enabling/disabling context checkpointing.
pub fn add_enable_checkpointing_row(parent: &PreferencesGroup, config: &Config) -> SwitchRow {
    let row = SwitchRow::builder()
        .title("Preserve conversation history")
        .subtitle("Summarizes earlier turns at ~70% context capacity (~165k chars for 64k models) to prevent context exhaustion")
        .build();

    let enabled = config.enable_checkpointing();
    row.set_active(enabled);

    parent.add(&row);
    row
}

/// Add a dropdown row for selecting the checkpoint summarizer model.
pub fn add_summarizer_model_row(parent: &PreferencesGroup, config: &Config) -> DropDown {
    let mut display_names: Vec<&str> = vec!["Same as active model (Default)"];
    let mut model_ids: Vec<Option<String>> = vec![None];

    for (id, name) in config.configured_models() {
        display_names.push(name);
        model_ids.push(Some(id.to_string()));
    }

    let string_list = StringList::new(&display_names);

    let dropdown = DropDown::new(Some(string_list), None::<gtk::Expression>);
    dropdown.set_valign(gtk::Align::Center);

    if let Some(ref preferred) = config.preferences.checkpoint_summarizer_model {
        for (i, opt_id) in model_ids.iter().enumerate() {
            if let Some(ref id) = opt_id {
                if id == preferred {
                    dropdown.set_selected(i as u32);
                    break;
                }
            }
        }
    }

    let row = ActionRow::builder()
        .title("Checkpoint Summarizer Model")
        .subtitle("Select a secondary model to offload background summarization")
        .build();
    row.add_suffix(&dropdown);

    parent.add(&row);
    dropdown
}
