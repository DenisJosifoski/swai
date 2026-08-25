//! SWAI — Checkpointing Preferences Tab.
//!
//! Context checkpointing / compaction controls (Phase 24_8 feature).
//! Phase 31 adds a configurable compaction trigger threshold slider.

use gtk::{DropDown, SpinButton, StringList};
use gtk4 as gtk;

use adw::prelude::*;
use adw::{ActionRow, PreferencesGroup, PreferencesPage, SwitchRow};

use swai_core::config::Config;

/// Widget handles for the Checkpointing tab.
pub struct CheckpointWidgets {
    pub enable_checkpointing_switch: SwitchRow,
    pub summarizer_model_combo: DropDown,
    pub threshold_spin: SpinButton,
}

/// Build the Checkpoint preferences page.
pub fn build_checkpoint_tab(config: &Config) -> (PreferencesPage, CheckpointWidgets) {
    let page = PreferencesPage::new();
    page.set_title("Checkpoint");

    let group = PreferencesGroup::new();
    group.set_title("Context Checkpoint");
    group.set_description(Some(
        "Summarizes earlier turns at ~70% context capacity to prevent context exhaustion and retain conversation memory across long sessions.",
    ));

    let enable_switch = add_enable_checkpointing_row(&group, config);
    let summarizer_dropdown = add_summarizer_model_row(&group, config);
    let threshold_spin = add_compaction_threshold_row(&group, config);

    page.add(&group);

    let widgets = CheckpointWidgets {
        enable_checkpointing_switch: enable_switch,
        summarizer_model_combo: summarizer_dropdown,
        threshold_spin,
    };

    (page, widgets)
}

/// Add a switch row for enabling/disabling context checkpointing.
pub fn add_enable_checkpointing_row(parent: &PreferencesGroup, config: &Config) -> SwitchRow {
    let row = SwitchRow::builder()
        .title("Enable Context Checkpoint")
        .subtitle("Switch Context Checkpoint ON/OFF")
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

/// Add a spinbutton row for configuring the compaction trigger threshold.
///
/// Range: 50%–85%, default 70%. Step size: 5%.
/// Displays a live readout of the resulting character budget for the
/// currently active model.
fn add_compaction_threshold_row(parent: &PreferencesGroup, config: &Config) -> SpinButton {
    let current_pct = config.compaction_threshold_pct();

    let min = swai_core::compaction::MIN_THRESHOLD_PCT as f64;
    let max = swai_core::compaction::MAX_THRESHOLD_PCT as f64;

    let adjustment = gtk::Adjustment::builder()
        .value(current_pct as f64)
        .lower(min)
        .upper(max)
        .step_increment(5.0)
        .page_increment(10.0)
        .build();

    let spin = SpinButton::new(Some(&adjustment), 0.0, 0);
    spin.set_digits(0);
    spin.set_value(current_pct as f64);
    spin.set_width_chars(6);

    // Build a readout label showing the current budget at this threshold.
    // We compute it from the active model's context size if known,
    // otherwise fall back to 64k.
    let active_ctx = config.models.first().map(|m| m.ctx_size).unwrap_or(65_536);

    let budget =
        swai_core::compaction::ContextBudget::from_ctx_size_and_threshold(active_ctx, current_pct);
    let readout = gtk::Label::builder()
        .label(&budget.summary_display())
        .use_markup(false)
        .halign(gtk::Align::Start)
        .wrap(true)
        .max_width_chars(40)
        .build();

    // Update the readout whenever the spin value changes.
    let readout_clone = readout.clone();
    spin.connect_value_changed(move |spin| {
        let pct = spin.value() as u8;
        let b = swai_core::compaction::ContextBudget::from_ctx_size_and_threshold(active_ctx, pct);
        readout_clone.set_label(&b.summary_display());
    });

    let row = ActionRow::builder()
        .title("Compaction Trigger Threshold")
        .subtitle("Trigger checkpoint summarization when context usage reaches this percentage (50%–85%, default 70%).")
        .build();
    row.add_suffix(&spin);
    row.add_suffix(&readout);

    parent.add(&row);
    spin
}
