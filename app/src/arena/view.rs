#![allow(dead_code, unused)]
//! SWAI — Visual card components for debate turn display.
//!
//! Color-coded cards for Generator Draft (Cyan), Auditor Critiques (Orange),
//! and Synthesizer Consensus (Green) turns.

use gtk::prelude::*;
use gtk4 as gtk;

use swai_core::council::{CouncilRole, DebateTranscript, TurnResult};

/// CSS classes for accent colors.
const CYAN_ACCENT: &str = "arena-generator";
const ORANGE_ACCENT: &str = "arena-auditor";
const GREEN_ACCENT: &str = "arena-synthesizer";

/// Apply accent color CSS to a widget.
fn apply_accent_style<W: IsA<gtk::Widget>>(widget: &W, css_class: &str) {
    let provider = gtk::CssProvider::new();
    let css = format!(
        "{} {{
            background-color: alpha(currentColor, 0.05);
            border-radius: 8px;
            padding: 12px;
            margin: 6px 0;
        }}",
        css_class
    );
    provider.load_from_data(&css);
    widget.add_css_class(css_class);
}

/// Get the accent color name for a role.
fn role_accent(role: &CouncilRole) -> &'static str {
    match role {
        CouncilRole::Generator => CYAN_ACCENT,
        CouncilRole::Auditor => ORANGE_ACCENT,
        CouncilRole::Synthesizer => GREEN_ACCENT,
        CouncilRole::Custom(_) => ORANGE_ACCENT,
    }
}

/// Get a human-readable role label.
fn role_label(role: &CouncilRole) -> String {
    match role {
        CouncilRole::Generator => "Generator Draft".to_string(),
        CouncilRole::Auditor => "Auditor Critique".to_string(),
        CouncilRole::Synthesizer => "Consensus / Synthesizer".to_string(),
        CouncilRole::Custom(name) => format!("Custom: {}", name),
    }
}

/// Create a single turn card with color-coded accent.
///
/// Returns a `gtk::Box` containing the role header and output text view.
pub fn create_turn_card(turn: &TurnResult) -> gtk::Box {
    let accent = role_accent(&turn.role);
    let label = role_label(&turn.role);

    // Card container.
    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.set_margin_start(12);
    card.set_margin_end(12);
    card.set_margin_top(6);
    card.set_margin_bottom(6);
    apply_accent_style(&card, accent);

    // Header bar with role name and model ID.
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let role_label = gtk::Label::new(Some(&label));
    role_label.set_css_classes(&[accent]);
    role_label.set_halign(gtk::Align::Start);

    let model_label = gtk::Label::new(Some(&format!("({})", turn.model_id)));
    model_label.set_css_classes(&["dim-label"]);
    model_label.set_halign(gtk::Align::End);
    model_label.set_hexpand(true);

    let duration_label = gtk::Label::new(Some(&format!("{}s", turn.duration.as_secs())));
    duration_label.set_css_classes(&["dim-label"]);

    header.append(&role_label);
    header.append(&model_label);
    header.append(&duration_label);
    card.append(&header);

    // Error indicator.
    if let Some(ref error) = turn.error {
        let error_label = gtk::Label::new(Some(&format!("Error: {}", error)));
        error_label.add_css_class("error-label");
        error_label.set_margin_start(6);
        card.append(&error_label);
    }

    // Output text view (read-only).
    let text_view = gtk::TextView::new();
    text_view.set_editable(false);
    text_view.set_wrap_mode(gtk::WrapMode::WordChar);
    text_view.set_monospace(true);

    let buffer = text_view.buffer();
    buffer.set_text(&turn.output);

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_min_content_height(100);
    scroll.set_max_content_height(400);
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroll.set_child(Some(&text_view));

    card.append(&scroll);
    card
}

/// Create a full transcript view with all turns stacked vertically.
///
/// Returns a `gtk::ScrolledWindow` containing the turn cards.
pub fn create_transcript_view(transcript: &DebateTranscript) -> gtk::ScrolledWindow {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.set_margin_start(6);
    container.set_margin_end(6);
    container.set_margin_top(6);
    container.set_margin_bottom(6);

    // Input prompt header.
    let prompt_card = gtk::Box::new(gtk::Orientation::Vertical, 4);
    prompt_card.set_css_classes(&["card"]);
    prompt_card.set_margin_start(12);
    prompt_card.set_margin_end(12);
    prompt_card.set_margin_top(6);
    prompt_card.set_margin_bottom(6);

    let prompt_header = gtk::Label::new(Some("Input Prompt"));
    prompt_header.set_css_classes(&["heading-3"]);
    prompt_header.set_halign(gtk::Align::Start);

    let prompt_text = gtk::Label::new(Some(&transcript.input_prompt));
    prompt_text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    prompt_text.set_selectable(true);
    prompt_text.set_margin_start(6);
    prompt_text.set_margin_end(6);

    prompt_card.append(&prompt_header);
    prompt_card.append(&prompt_text);
    container.append(&prompt_card);

    // Turn cards.
    for turn in &transcript.turns {
        let card = create_turn_card(turn);
        container.append(&card);
    }

    // Summary footer.
    let summary = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    summary.set_margin_start(12);
    summary.set_margin_end(12);
    summary.set_margin_top(12);
    summary.set_margin_bottom(6);

    let turn_count = gtk::Label::new(Some(&format!("{} turns", transcript.turn_count())));
    turn_count.set_css_classes(&["dim-label"]);

    let status_label = if transcript.all_succeeded() {
        gtk::Label::new(Some("✓ All stages succeeded"))
    } else {
        gtk::Label::new(Some("⚠ Some stages had errors"))
    };
    status_label.set_hexpand(true);
    status_label.set_halign(gtk::Align::End);

    summary.append(&turn_count);
    summary.append(&status_label);
    container.append(&summary);

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scrolled.set_child(Some(&container));
    scrolled
}
