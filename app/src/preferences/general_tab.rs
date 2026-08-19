use gtk::{DropDown, FileChooserAction, ResponseType, SpinButton, StringList, Window};
use gtk4 as gtk;

use adw::prelude::*;
use adw::{ActionRow, EntryRow, SwitchRow};

use std::path::PathBuf;
use swai_core::config::Config;

pub fn add_log_dir_row(
    parent: &impl IsA<gtk::Box>,
    dialog_parent: &impl IsA<gtk::Window>,
    config: &Config,
) -> EntryRow {
    let row = EntryRow::builder().title("Log directory").build();

    let current_path = config.log_dir();
    row.set_text(current_path.to_string_lossy().as_ref());

    // Add a Browse button to the end of the row.
    let entry_clone = row.clone();
    let dialog_parent_clone = dialog_parent.clone();
    let browse_btn = gtk::Button::builder()
        .label("Browse…")
        .css_classes(vec!["flat"])
        .build();
    browse_btn.connect_clicked(move |_| {
        show_folder_chooser(&entry_clone, &dialog_parent_clone);
    });
    row.add_suffix(&browse_btn);

    // Append the completed row to the dialog's content box.
    parent.as_ref().append(&row);
    row
}

/// Show a folder chooser dialog using the async run_async pattern.
fn show_folder_chooser<T: IsA<Window>>(entry: &EntryRow, parent: &T) {
    let chooser = gtk::FileChooserDialog::new(
        Some("Select Log Directory"),
        Some(parent),
        FileChooserAction::SelectFolder,
        &[
            ("_Cancel", ResponseType::Cancel),
            ("_Select", ResponseType::Ok),
        ],
    );

    if let Ok(path) = std::env::var("HOME") {
        let _ = chooser.set_current_folder(Some(&gio::File::for_path(PathBuf::from(path))));
    }

    let entry_clone = entry.clone();
    chooser.run_async(move |chooser, response| {
        if response == ResponseType::Ok {
            if let Some(folder) = chooser.current_folder() {
                if let Some(path) = folder.path() {
                    entry_clone.set_text(&path.to_string_lossy());
                }
            }
        }
        chooser.destroy();
    });
}

pub fn add_proxy_port_row(parent: &gtk::Box, config: &Config) -> EntryRow {
    let row = EntryRow::builder().title("Proxy port").build();

    let proxy_port = config.proxy_port();
    row.set_text(&proxy_port.to_string());

    parent.append(&row);
    row
}

pub fn add_auto_restart_row(parent: &gtk::Box, config: &Config) -> SwitchRow {
    let row = SwitchRow::builder()
        .title("Auto-restart on context full")
        .build();

    let auto_restart = config.auto_restart_on_context_full();
    row.set_active(auto_restart);

    parent.append(&row);
    row
}

pub fn add_auto_follow_logs_row(parent: &gtk::Box, config: &Config) -> SwitchRow {
    let row = SwitchRow::builder()
        .title("Auto-follow active model in logs")
        .build();

    let auto_follow = config.auto_follow_logs();
    row.set_active(auto_follow);

    parent.append(&row);
    row
}

pub fn add_enable_notifications_row(parent: &gtk::Box, config: &Config) -> SwitchRow {
    let row = SwitchRow::builder()
        .title("Enable desktop notifications")
        .build();

    let enable = config.enable_notifications();
    row.set_active(enable);

    parent.append(&row);
    row
}

pub fn add_notify_on_switch_row(parent: &gtk::Box, config: &Config) -> SwitchRow {
    let row = SwitchRow::builder().title("Notify on model switch").build();

    let notify = config.notify_on_switch();
    row.set_active(notify);

    parent.append(&row);
    row
}

pub fn add_autostart_on_login_row(parent: &gtk::Box, config: &Config) -> SwitchRow {
    let row = SwitchRow::builder()
        .title("Start SWAI automatically on login")
        .build();

    let autostart = config.autostart_on_login();
    row.set_active(autostart);

    parent.append(&row);
    row
}

/// Add a spin button for configuring the maximum number of concurrent
/// model servers (1–4). Placed in the System section of the preferences.
pub fn add_max_concurrent_models_row(parent: &gtk::Box, config: &Config) -> SpinButton {
    let current = config.max_concurrent_models().clamp(1, 4) as f64;

    let adj = gtk::Adjustment::new(
        current, // value
        1.0,     // lower bound
        4.0,     // upper bound
        1.0,     // step increment
        1.0,     // page increment
        0.0,     // page size
    );

    let spin = SpinButton::new(Some(&adj), 0.0, 0);
    spin.set_snap_to_ticks(true);

    // Wrap in an ActionRow for consistent styling with the rest of the dialog.
    let row = ActionRow::builder()
        .title("Max concurrent models")
        .subtitle("Number of model servers allowed to run simultaneously (1–4)")
        .build();
    row.add_prefix(&spin);

    parent.append(&row);
    spin
}

/// Add a dropdown row for selecting the checkpoint summarizer model.
///
/// Options include "Same as active model (Default)" plus each configured
/// model's display name. The selected value is stored as either `None`
/// (default) or the model's configured id.
pub fn add_summarizer_model_row(parent: &gtk::Box, config: &Config) -> DropDown {
    // Build the dropdown options: "Same as active model (Default)" first,
    // then each configured model's name. We track the mapping from display
    // text → model id (or None for the default option) separately.
    let mut display_names: Vec<&str> = vec!["Same as active model (Default)"];
    let mut model_ids: Vec<Option<String>> = vec![None];

    for (id, name) in config.configured_models() {
        display_names.push(name);
        model_ids.push(Some(id.to_string()));
    }

    // StringList::new expects &[&str].
    let string_list = StringList::new(&display_names);

    let dropdown = DropDown::new(Some(string_list), None::<gtk::Expression>);

    // Set the initial selection based on current config.
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

    // Wrap in an ActionRow for consistent styling with the rest of the dialog.
    let row = ActionRow::builder()
            .title("Checkpoint Summarizer Model")
            .subtitle("Model used to summarize evicted conversation history. Leaving it on the active model means summarization shares context; selecting a secondary model offloads summarization to keep the primary model's context free.")
            .build();
    row.add_prefix(&dropdown);

    parent.append(&row);
    dropdown
}

/// Add a switch row for enabling/disabling context checkpointing.
///
/// When disabled, the proxy bypasses diff generation for file-write tools and
/// skips the loop-breaker heuristic, reducing CPU overhead at the cost of
/// losing milestone ledgers and loop detection. Recommended only for models
/// with 128k+ context windows.
pub fn add_enable_checkpointing_row(parent: &gtk::Box, config: &Config) -> SwitchRow {
    let row = SwitchRow::builder()
        .title("Enable Context Checkpointing")
        .build();

    let enabled = config.enable_checkpointing();
    row.set_active(enabled);

    // Add helper text as a subtitle on the switch row.
    let helper = gtk::Label::builder()
        .label("Disable milestone ledgers and loop-breaking. Recommended only for models with 128k+ context to improve latency.")
        .use_markup(false)
        .css_classes(vec!["dim-label"])
        .halign(gtk::Align::Start)
        .margin_start(16)
        .margin_top(2)
        .build();
    row.add_suffix(&helper);

    parent.append(&row);
    row
}
