#![allow(dead_code, unused)]
//! SWAI — ArenaWindow: GTK4/Libadwaita debate arena desktop window.
//!
//! Provides a sidebar for browsing saved debates and a live stream view
//! for displaying turn-by-turn council deliberations.

use gtk::prelude::*;
use gtk4 as gtk;
use std::cell::RefCell;
use std::rc::Rc;

use super::history;
use super::view;
use swai_core::council::DebateTranscript;

/// ArenaWindow: Libadwaita window with history sidebar and transcript view.
pub struct ArenaWindow {
    /// The GTK application window.
    widget: gtk::ApplicationWindow,
    /// Sidebar listbox showing saved debates.
    debate_list: gtk::ListBox,
    /// Right panel containing the transcript view.
    transcript_view: gtk::ScrolledWindow,
    /// Currently loaded transcript (if any).
    current_transcript: Rc<RefCell<Option<DebateTranscript>>>,
}

impl ArenaWindow {
    /// Create a new ArenaWindow.
    pub fn new() -> Self {
        // ── Window setup ────────────────────────────────────────
        let widget = gtk::ApplicationWindow::builder()
            .title("Arena — Debate History")
            .default_width(960)
            .default_height(640)
            .build();

        // ── Header bar ──────────────────────────────────────────
        let header = gtk::HeaderBar::new();
        header.set_show_title_buttons(true);

        // New debate button.
        let new_btn = gtk::Button::builder().label("New Debate").build();
        new_btn.add_css_class("suggested-action");
        new_btn.set_margin_end(6);

        // Save current button.
        let save_btn = gtk::Button::builder().label("Save Current").build();
        save_btn.set_margin_end(6);

        header.pack_start(&new_btn);
        header.pack_end(&save_btn);
        widget.set_titlebar(Some(&header));

        // ── Main layout: sidebar + content ──────────────────────
        let main_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        main_box.set_margin_start(0);
        main_box.set_margin_end(0);
        main_box.set_margin_top(0);
        main_box.set_margin_bottom(0);

        // ── Sidebar ─────────────────────────────────────────────
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.set_width_request(240);
        sidebar.add_css_class("view");

        let sidebar_header = gtk::Label::new(Some("Saved Debates"));
        sidebar_header.set_css_classes(&["heading-2"]);
        sidebar_header.set_margin_start(12);
        sidebar_header.set_margin_top(12);
        sidebar.append(&sidebar_header);

        // Debate list (ListBox).
        let debate_list = gtk::ListBox::new();
        debate_list.set_selection_mode(gtk::SelectionMode::Single);
        debate_list.add_css_class("navigation-sidebar");

        // Populate initial list.
        populate_debate_list(&debate_list);

        let sidebar_scroll = gtk::ScrolledWindow::new();
        sidebar_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        sidebar_scroll.set_child(Some(&debate_list));

        sidebar.append(&sidebar_scroll);
        main_box.append(&sidebar);

        // ── Content area ────────────────────────────────────────
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_hexpand(true);
        content.set_vexpand(true);

        // Empty state placeholder.
        let empty_label = gtk::Label::new(Some("Select a debate from the sidebar to view it.\n\nOr start a new debate via the Council API."));
        empty_label.set_justify(gtk::Justification::Center);
        empty_label.set_margin_start(24);
        empty_label.set_margin_end(24);
        empty_label.set_margin_top(48);
        empty_label.set_margin_bottom(48);
        content.append(&empty_label);

        // Transcript view (initially hidden).
        let transcript_view = gtk::ScrolledWindow::new();
        transcript_view.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        transcript_view.set_visible(false);

        content.append(&transcript_view);
        main_box.append(&content);

        widget.set_child(Some(&main_box));

        // ── Current transcript tracking ─────────────────────────
        let current_transcript = Rc::new(RefCell::new(None::<DebateTranscript>));

        // ── Sidebar selection handler ───────────────────────────
        let ct_clone = Rc::clone(&current_transcript);
        debate_list.connect_row_activated(move |_listbox, row| {
            // Get the label text from the first child of the row.
            let label_text = if let Some(first) = row.first_child() {
                if let Some(label) = first.downcast_ref::<gtk::Label>() {
                    label.text().to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            if let Ok(transcript) = history::load_transcript(&label_text) {
                *ct_clone.borrow_mut() = Some(transcript.clone());

                // Replace content area with transcript view.
                let tv = view::create_transcript_view(&transcript);
                tv.set_visible(true);

                // Remove empty label if present.
                if let Some(child) = content.first_child() {
                    content.remove(&child);
                }
                content.append(&tv);
            } else {
                tracing::error!("Failed to load debate: {}", label_text);
            }
        });

        // ── Save button handler ─────────────────────────────────
        let ct_save = Rc::clone(&current_transcript);
        save_btn.connect_clicked(move |_| {
            if let Some(transcript) = ct_save.borrow().as_ref() {
                match history::save_transcript(transcript) {
                    Ok(path) => tracing::info!("Saved debate to: {}", path.display()),
                    Err(e) => tracing::error!("Failed to save debate: {}", e),
                }
            } else {
                tracing::warn!("No debate loaded to save.");
            }
        });

        Self {
            widget,
            debate_list,
            transcript_view,
            current_transcript,
        }
    }

    /// Present the window (make it visible and raise it).
    pub fn present(&self) {
        self.widget.present();
    }

    /// Load and display a debate by session ID.
    pub fn load_debate(&self, id: &str) -> Result<(), String> {
        let transcript = history::load_transcript(id)?;

        *self.current_transcript.borrow_mut() = Some(transcript.clone());

        // Remove empty state if present.
        if let Some(child) = self.widget.first_child() {
            if let Some(box_widget) = child.downcast_ref::<gtk::Box>() {
                if let Some(empty) = box_widget.first_child() {
                    if empty.is::<gtk::Label>() {
                        box_widget.remove(&empty);
                    }
                }
            }
        }

        // Replace content with transcript view.
        self.transcript_view.set_child(Some(&view::create_transcript_view(&transcript)));
        self.transcript_view.set_visible(true);

        Ok(())
    }

    /// Save the currently loaded debate to disk.
    pub fn save_current(&self) -> Result<std::path::PathBuf, String> {
        let transcript = self
            .current_transcript
            .borrow()
            .as_ref()
            .ok_or_else(|| "No debate loaded".to_string())?
            .clone();

        history::save_transcript(&transcript)
    }
}

/// Populate the debate listbox with saved debates.
fn populate_debate_list(listbox: &gtk::ListBox) {
    let debates = match history::list_debates() {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!("Failed to list debates: {}", e);
            return;
        }
    };

    for id in debates {
        let row = gtk::ListBoxRow::new();
        let label = gtk::Label::new(Some(&id));
        label.set_xalign(0.0);
        label.set_margin_start(12);
        label.set_margin_end(12);
        label.set_margin_top(6);
        label.set_margin_bottom(6);

        row.add_css_class("selectable");
        row.set_activatable(true);
        // Use set_child instead of append for ListBoxRow.
        row.set_child(Some(&label));
        listbox.append(&row);
    }
}
