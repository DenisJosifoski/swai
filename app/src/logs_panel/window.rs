use gtk::prelude::*;
use gtk4 as gtk;

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::poller::{resolve_checkpoint_path, resolve_log_file, start_tail_poller};
use super::types::ViewMode;

#[allow(dead_code)]
pub struct LogViewerWindow {
    /// The GTK application window.
    widget: gtk::ApplicationWindow,
    /// The text buffer holding the current log content.
    text_buffer: gtk::TextBuffer,
    /// Path to the log file being viewed (used for clear/export).
    log_file: PathBuf,
    /// Tracks the byte offset of the last read to detect new appends.
    last_offset: Rc<Cell<usize>>,
    /// The glib timeout source ID for auto-tail polling. Stored so we can
    /// remove it when the window is destroyed.
    timeout_id: Rc<Cell<Option<glib::SourceId>>>,
    /// Directory containing log files — needed to resolve new model logs
    /// when switching via the dropdown.
    log_dir: PathBuf,
    /// All configured models — used to populate the dropdown and look up
    /// script paths when the user switches models.
    all_models: Vec<swai_core::config::ModelConfig>,
    /// The model-selector dropdown in the header bar. Stored so we can
    /// programmatically update its selection when auto-follow is enabled.
    dropdown: gtk::DropDown,
    /// Current view mode (Logs or Checkpoints).
    view_mode: Rc<Cell<ViewMode>>,
    /// Path to the checkpoint file for the current model.
    checkpoint_path: Rc<Cell<Option<PathBuf>>>,
}

impl LogViewerWindow {
    /// Create a new log viewer window for the given model's log file.
    ///
    /// Resolves the most recent log file from the log directory by matching
    /// the script stem (e.g., `run-llama_20260724_143022.log`).
    ///
    /// `all_models` is used to populate the model-selector dropdown; the
    /// currently active model (`model_id`) is pre-selected.
    pub fn new(
        model_name: &str,
        script_path: &Path,
        log_dir: &Path,
        model_id: &str,
        all_models: &[swai_core::config::ModelConfig],
    ) -> Self {
        let log_file = resolve_log_file(script_path, log_dir);

        // ── Window setup ───────────────────────────────────────────
        let widget = gtk::ApplicationWindow::builder()
            .title(format!("Logs — {}", model_name))
            .default_width(720)
            .default_height(500)
            .build();

        // ── Header bar with action buttons & model selector ────────
        let header = gtk::HeaderBar::new();
        header.set_show_title_buttons(true);

        // View mode tracking (Logs / Checkpoints).
        let view_mode = Rc::new(Cell::new(ViewMode::Logs));
        let checkpoint_path = Rc::new(Cell::new(None::<PathBuf>));

        // Model selector dropdown — populate with all configured models.
        let model_names: Vec<glib::GString> = all_models
            .iter()
            .map(|m| glib::GString::from(m.name.clone()))
            .collect();
        let str_refs: Vec<&str> = model_names.iter().map(|s| s.as_str()).collect();
        let string_list = gtk::StringList::new(&str_refs);

        let dropdown = gtk::DropDown::new(Some(string_list), None::<gtk::Expression>);
        dropdown.set_margin_start(6);
        dropdown.set_margin_end(6);

        // Pre-select the model that opened the viewer.
        if let Some(idx) = all_models.iter().position(|m| m.id == model_id) {
            dropdown.set_selected(idx as u32);
        } else if !all_models.is_empty() {
            dropdown.set_selected(0);
        }

        // Clear button — empties the log file and the TextView.
        let clear_btn = gtk::Button::builder().label("Clear").build();
        clear_btn.set_css_classes(&["flat"]);

        // Export button — opens save dialog to copy log file elsewhere.
        let export_btn = gtk::Button::builder().label("Export").build();
        export_btn.set_css_classes(&["flat"]);

        // Checkpoints toggle — switches between live log tail and checkpoint view.
        let checkpoints_btn = gtk::Button::builder().label("Checkpoints").build();
        checkpoints_btn.set_css_classes(&["flat"]);

        // Close button — destroys the window and stops the poller.
        let close_btn = gtk::Button::builder().label("Close").build();
        close_btn.set_css_classes(&["suggested-action", "flat"]);

        header.pack_start(&dropdown);
        header.pack_end(&clear_btn);
        header.pack_end(&export_btn);
        header.pack_end(&checkpoints_btn);
        header.pack_end(&close_btn);

        // ── Log file path label in the header ──────────────────────
        let filepath_label = gtk::Label::new(Some(&log_file.display().to_string()));
        filepath_label.set_css_classes(&["caption", "dim-label"]);
        filepath_label.set_halign(gtk::Align::Start);
        filepath_label.set_hexpand(true);
        filepath_label.set_margin_start(12);
        filepath_label.set_margin_end(6);
        filepath_label.set_max_width_chars(40);
        filepath_label.set_width_chars(40);

        // Put the filepath in a secondary bar below the header
        let info_bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        info_bar.append(&filepath_label);

        let toolbar_stack = gtk::Box::new(gtk::Orientation::Vertical, 0);
        toolbar_stack.append(&header);
        toolbar_stack.append(&info_bar);

        // ── Scrollable TextView with monospace font ────────────────
        let text_buffer = gtk::TextBuffer::new(None);
        let text_view = gtk::TextView::builder()
            .buffer(&text_buffer)
            .monospace(true)
            .editable(false)
            .wrap_mode(gtk::WrapMode::WordChar)
            .left_margin(8)
            .right_margin(8)
            .top_margin(4)
            .bottom_margin(4)
            .build();

        let scrolled = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        scrolled.set_child(Some(&text_view));

        // ── Assemble the window ────────────────────────────────────
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&toolbar_stack);
        content.append(&scrolled);

        widget.set_child(Some(&content));

        // Store the poller ID so we can clean it up on window close.
        let timeout_id = Rc::new(Cell::new(None::<glib::SourceId>));
        let last_offset = Rc::new(Cell::new(0usize));

        // Clone refs for the dropdown handler and the destroy handler.
        let tid_rc_dropdown = Rc::clone(&timeout_id);
        let lo_rc_dropdown = Rc::clone(&last_offset);
        let text_buffer_dropdown = text_buffer.clone();
        let filepath_label_dropdown = filepath_label.clone();
        let log_dir_for_dropdown = PathBuf::from(log_dir);
        let all_models_for_dropdown = all_models.to_vec();

        // Clone values before moving into the dropdown closure.
        let tid_rc_clone = Rc::clone(&tid_rc_dropdown);
        let lo_rc_clone = Rc::clone(&lo_rc_dropdown);
        let text_view_clone = text_view.clone();
        let log_dir_clone = PathBuf::from(&log_dir_for_dropdown);

        // Wire Close button → destroy window.
        let win_close = widget.clone();
        close_btn.connect_clicked(move |_| {
            win_close.destroy();
        });

        // Wire Clear button → empty text buffer & truncate log file.
        let text_buffer_clear = text_buffer.clone();
        let lo_rc_clear = Rc::clone(&last_offset);
        let filepath_label_clear = filepath_label.clone();
        clear_btn.connect_clicked(move |_| {
            text_buffer_clear.set_text("");
            lo_rc_clear.set(0);
            let path_str = filepath_label_clear.text();
            let path = Path::new(path_str.as_str());
            if path.exists() {
                let _ = fs::write(path, "");
            }
        });

        // Wire Export button → FileChooserDialog to save log copy.
        let win_export = widget.clone();
        let filepath_label_export = filepath_label.clone();
        export_btn.connect_clicked(move |_| {
            let dialog = gtk::FileChooserDialog::new(
                Some("Export Log File"),
                Some(&win_export),
                gtk::FileChooserAction::Save,
                &[
                    ("Cancel", gtk::ResponseType::Cancel),
                    ("Save", gtk::ResponseType::Accept),
                ],
            );
            dialog.set_modal(true);
            let path_str = filepath_label_export.text();
            let src_path = PathBuf::from(path_str.as_str());
            if let Some(stem) = src_path.file_name() {
                dialog.set_current_name(&stem.to_string_lossy());
            } else {
                dialog.set_current_name("swai_model.log");
            }

            dialog.connect_response(move |d, response| {
                if response == gtk::ResponseType::Accept {
                    if let Some(dest_file) = d.file() {
                        if let Some(dest_path) = dest_file.path() {
                            let _ = fs::copy(&src_path, &dest_path);
                        }
                    }
                }
                d.destroy();
            });
            dialog.present();
        });

        // Wire Checkpoints toggle → switch between log tail and checkpoint view.
        let view_mode_rc = Rc::clone(&view_mode);
        let text_buffer_cp = text_buffer.clone();
        let filepath_label_cp = filepath_label.clone();
        let tid_rc_cp = Rc::clone(&timeout_id);
        let last_offset_cp = Rc::clone(&last_offset);
        let checkpoint_path_rc = Rc::clone(&checkpoint_path);

        // Clone values that are also used in the dropdown closure.
        let dropdown_cp = dropdown.clone();
        let all_models_cp = all_models_for_dropdown.clone();
        let log_dir_cp = log_dir_clone.clone();
        let text_view_cp = text_view_clone.clone();
        let lo_rc_cp = Rc::clone(&lo_rc_clone);
        let tid_rc_dropdown_cp = Rc::clone(&tid_rc_clone);

        checkpoints_btn.connect_clicked(move |btn| {
            let current_mode = view_mode_rc.get();
            let new_mode = match current_mode {
                ViewMode::Logs => ViewMode::Checkpoints,
                ViewMode::Checkpoints => ViewMode::Logs,
            };
            view_mode_rc.set(new_mode);

            match new_mode {
                ViewMode::Checkpoints => {
                    // Stop the log tail poller.
                    if let Some(id) = tid_rc_cp.take() {
                        id.remove();
                    }

                    // Clear the text buffer.
                    text_buffer_cp.set_text("");

                    // Resolve the checkpoint file path for the current model.
                    let selected_idx = dropdown_cp.selected() as usize;
                    if selected_idx < all_models_cp.len() {
                        let model = &all_models_cp[selected_idx];
                        let cp_path = resolve_checkpoint_path(&model.id);
                        checkpoint_path_rc.set(Some(cp_path.clone()));

                        // Update filepath label.
                        filepath_label_cp.set_text(&format!("Checkpoints: {}", cp_path.display()));

                        // Load checkpoint content into the text buffer.
                        if let Ok(content) = fs::read_to_string(&cp_path) {
                            text_buffer_cp.set_text(&content);
                        } else {
                            text_buffer_cp.set_text(
                                "No checkpoints recorded yet for this session.\n\n\
                                 Checkpoints are written when message compaction occurs.\n\
                                 Start a long coding session to trigger compaction.",
                            );
                        }
                    }

                    // Update button appearance.
                    btn.set_css_classes(&["suggested-action", "flat"]);
                }
                ViewMode::Logs => {
                    // Clear the text buffer.
                    text_buffer_cp.set_text("");
                    last_offset_cp.set(0);

                    // Resolve the current model's log file and restart the poller.
                    let selected_idx = dropdown_cp.selected() as usize;
                    if selected_idx < all_models_cp.len() {
                        let model = &all_models_cp[selected_idx];
                        let log_file = resolve_log_file(&model.script_path, &log_dir_cp);

                        // Update filepath label.
                        filepath_label_cp.set_text(&log_file.display().to_string());

                        // Restart the tail poller.
                        let tid = start_tail_poller(
                            log_file,
                            text_view_cp.clone(),
                            lo_rc_cp.clone(),
                            Rc::clone(&tid_rc_dropdown_cp),
                        );
                        tid_rc_cp.set(Some(tid));
                    }

                    // Reset button appearance.
                    btn.set_css_classes(&["flat"]);
                }
            }
        });

        // Wire dropdown selection change → switch model.
        dropdown.connect_selected_notify(move |dd| {
            let selected_idx = dd.selected() as usize;
            if selected_idx >= all_models_for_dropdown.len() {
                return;
            }
            let model = &all_models_for_dropdown[selected_idx];
            tracing::info!(
                "LogViewer: switching to model '{}' (id={})",
                model.name,
                model.id
            );

            // 1. Stop the current auto-tail poller.
            if let Some(id) = tid_rc_clone.take() {
                id.remove();
            }

            // 2. Clear the text buffer.
            text_buffer_dropdown.set_text("");

            // 3. Reset offset to zero for the new file.
            lo_rc_clone.set(0);

            // 4. Resolve the new model's log file.
            let new_log = resolve_log_file(&model.script_path, &log_dir_clone);

            // 5. Update filepath label and restart the poller.
            filepath_label_dropdown.set_text(&new_log.display().to_string());

            let tid = start_tail_poller(
                new_log,
                text_view_clone.clone(),
                lo_rc_clone.clone(),
                Rc::clone(&tid_rc_clone),
            );
            tid_rc_clone.set(Some(tid));
        });

        // Clone tid_rc for the destroy handler.
        let tid_rc_destroy = Rc::clone(&timeout_id);
        widget.connect_destroy(move |_win| {
            if let Some(id) = tid_rc_destroy.take() {
                id.remove();
            }
        });

        // Start the auto-tail poller.
        let tid = start_tail_poller(
            log_file.clone(),
            text_view.clone(),
            lo_rc_dropdown.clone(),
            Rc::clone(&tid_rc_dropdown),
        );
        tid_rc_dropdown.set(Some(tid));

        Self {
            widget,
            text_buffer,
            log_file,
            last_offset: lo_rc_dropdown.clone(),
            timeout_id,
            log_dir: log_dir_for_dropdown,
            all_models: all_models.to_vec(),
            dropdown,
            view_mode: Rc::clone(&view_mode),
            checkpoint_path: Rc::clone(&checkpoint_path),
        }
    }

    /// Present the window (make it visible and raise it).
    pub fn present(&self) {
        self.widget.present();
    }

    /// Update the dropdown selection to the index of the given model ID.
    ///
    /// Used by MainWindow to auto-follow the active model when a switch occurs.
    /// No-op if the model ID is not found in the dropdown list.
    pub fn select_model_by_id(&self, model_id: &str) {
        for (idx, model) in self.all_models.iter().enumerate() {
            if model.id == model_id {
                self.dropdown.set_selected(idx as u32);
                return;
            }
        }
    }

    /// Get the currently selected model ID from the dropdown.
    #[allow(dead_code)]
    pub fn selected_model_id(&self) -> Option<String> {
        let selected = self.dropdown.selected() as usize;
        self.all_models.get(selected).map(|m| m.id.clone())
    }
}
