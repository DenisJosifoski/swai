use gtk4 as gtk;

pub const CSS: &str = r#"
    /* ── Kill every possible headerbar underline source ────────── */
    headerbar, headerbar:backdrop {
        box-shadow: none;
        border: none;
        border-bottom: none;
        border-bottom-width: 0;
        border-bottom-style: none;
        border-bottom-color: transparent;
    }
    .titlebar, .titlebar:backdrop {
        box-shadow: none;
        border-bottom: none;
        border-bottom-width: 0;
    }
    .titlebar separator,
    headerbar separator {
        min-height: 0;
        opacity: 0;
        background: transparent;
    }
    window.background > decoration {
        box-shadow: none;
    }

    /* ── Model cards: rounded with subtle native border ────────── */
    .card, .card-active {
        background-color: alpha(@theme_fg_color, 0.06);
        border: 1px solid alpha(@theme_fg_color, 0.12);
        border-radius: 12px;
        padding: 12px;
    }
    .card-active {
        background-color: alpha(@theme_fg_color, 0.08);
        border-left: 3px solid #2dd4f0;
    }

    /* ── Switch: pill toggle matching button.png design ────────── */
    switch,
    switch:hover,
    switch:active,
    switch:backdrop {
        border-radius: 10px; /* 20px height / 2 = 10px */
        border: 1px solid alpha(@theme_fg_color, 0.25);
        background-color: alpha(@theme_fg_color, 0.12);
        box-shadow: none;
        outline: none;
    }
    switch:checked,
    switch:checked:hover,
    switch:checked:active,
    switch:checked:backdrop {
        background-color: #2dd4f0;
        border-color: #2dd4f0;
    }
    switch slider,
    switch slider:hover,
    switch slider:active,
    switch slider:backdrop {
        border-radius: 8px; /* 16px diameter / 2 = 8px */
        background-color: #ffffff;
        box-shadow: 0 1px 2px rgba(0, 0, 0, 0.25);
        border: none;
        margin: 2px;
        min-width: 16px;
        min-height: 16px;
    }

    /* ── Context meter: subtle rounded progress bar ────────────── */
    .context-meter {
        border-radius: 4px;
        background-color: alpha(@theme_fg_color, 0.08);
        min-height: 6px;
    }
    .context-meter progress {
        border-radius: 4px;
        background-color: #2dd4f0;
    }

    /* ── Log viewer: monospace body ────────────────────────────── */
    .log-viewer-text {
        font-family: monospace;
        font-size: 11pt;
    }
"#;

pub fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(CSS);
    if let Some(display) = adw::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
