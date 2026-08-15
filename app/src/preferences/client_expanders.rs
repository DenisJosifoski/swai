use gtk4 as gtk;
use gtk::prelude::*;
use gtk::Orientation;

use adw::prelude::*;
use adw::ExpanderRow;

use swai_core::config::Config;
use super::gateway_tab::{copy_to_clipboard, open_claude_cli_config, open_codex_config};

pub fn build_claude_cli_expander(config: &Config) -> ExpanderRow {
        let port = config.proxy_port();
        let base_url = format!("http://127.0.0.1:{port}/v1", port = port);

        let expander = ExpanderRow::builder()
            .title("<span foreground=\"#DA7756\" weight=\"bold\">Claude Code CLI</span>")
            .build();

        let content = gtk::Box::new(Orientation::Vertical, 4);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(6);
        content.set_margin_bottom(6);

        // Config file path row.
        let config_path_row = gtk::Box::new(Orientation::Horizontal, 6);
        let config_path_label = gtk::Label::builder()
            .label("Config file")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        config_path_row.append(&config_path_label);

        let config_path_value = gtk::Label::builder()
            .label("~/.bashrc (or ~/.zshrc)")
            .xalign(0.0)
            .selectable(true)
            .build();
        config_path_row.append(&config_path_value);

        content.append(&config_path_row);

        // Base URL row with copy button.
        let url_row = gtk::Box::new(Orientation::Horizontal, 6);
        let url_label = gtk::Label::builder()
            .label("Base URL")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        url_row.append(&url_label);

        let url_value = gtk::Label::builder()
            .label(&base_url)
            .xalign(0.0)
            .selectable(true)
            .build();
        url_row.append(&url_value);

        let copy_url_btn = gtk::Button::builder()
            .label("Copy")
            .css_classes(vec!["flat"])
            .build();
        let base_url_clone = base_url.clone();
        copy_url_btn.connect_clicked(move |_| {
            copy_to_clipboard(&base_url_clone);
        });
        url_row.append(&copy_url_btn);

        content.append(&url_row);

        // API Key row with copy button.
        let key_row = gtk::Box::new(Orientation::Horizontal, 6);
        let key_label = gtk::Label::builder()
            .label("API Key")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        key_row.append(&key_label);

        let key_value = gtk::Label::builder()
            .label("local")
            .xalign(0.0)
            .selectable(true)
            .build();
        key_row.append(&key_value);

        let copy_key_btn = gtk::Button::builder()
            .label("Copy")
            .css_classes(vec!["flat"])
            .build();
        copy_key_btn.connect_clicked(move |_| {
            copy_to_clipboard("local");
        });
        key_row.append(&copy_key_btn);

        content.append(&key_row);

        // Explanatory header label.
        let header_label = gtk::Label::builder()
            .label("Add the following function to your shell profile (~/.bashrc or ~/.zshrc):")
            .halign(gtk::Align::Start)
            .margin_top(6)
            .build();
        content.append(&header_label);

        // Monospace bash function display label.
        let func_display = format!(
            "claude-local() {{\n  export ANTHROPIC_BASE_URL=http://127.0.0.1:{port}\n  \
             export ANTHROPIC_AUTH_TOKEN=local\n  export ANTHROPIC_API_KEY=\"\"\n  local live_model\n  \
             live_model=$(curl -s http://127.0.0.1:{port}/v1/models 2>/dev/null | grep -o '\"id\":\"[^\"]*\"' | head -1 | cut -d'\"' -f4 | sed 's/^claude-//')\n  \
             export ANTHROPIC_MODEL=\"${{live_model:-unknown}}[1m]\"\n  \
             export ANTHROPIC_SMALL_FAST_MODEL=\"$ANTHROPIC_MODEL\"\n  echo \"🚀 Claude Code → $live_model\"\n  claude \"$@\"\n}}\n\n\
             claude-with() {{\n  local target=\"$1\"; shift\n  export ANTHROPIC_BASE_URL=http://127.0.0.1:{port}\n  \
             export ANTHROPIC_AUTH_TOKEN=local\n  export ANTHROPIC_API_KEY=\"\"\n  local live_model\n  \
             live_model=$(curl -s http://127.0.0.1:{port}/v1/models 2>/dev/null | grep -o '\"id\":\"[^\"]*\"' | cut -d'\"' -f4 | grep -i \"$target\" | head -1 | sed 's/^claude-//')\n  \
             if [ -z \"$live_model\" ]; then echo \"⚠️  No SWAI model matching '$target'\"; return 1; fi\n  \
             export ANTHROPIC_MODEL=\"${{live_model}}[1m]\"\n  export ANTHROPIC_SMALL_FAST_MODEL=\"$ANTHROPIC_MODEL\"\n  \
             echo \"🚀 Claude Code → $live_model\"\n  claude \"$@\"\n}}",
            port = port,
        );
        let instructions = gtk::Label::builder()
            .label(&func_display)
            .use_markup(false)
            .wrap(true)
            .xalign(0.0)
            .css_classes(vec!["monospace"])
            .margin_top(4)
            .build();
        content.append(&instructions);

        // Explanatory footer label.
        let footer_label = gtk::Label::builder()
            .label("Reload with `source ~/.bashrc`. Run `claude-local` for auto-selected model, or `claude-with <name>` (e.g. `claude-with qwopus`) to target a specific running model.")
            .halign(gtk::Align::Start)
            .wrap(true)
            .margin_top(6)
            .build();
        content.append(&footer_label);

        // Button row: "Copy Config Block" and "Open Config File".
        let btn_box = gtk::Box::new(Orientation::Horizontal, 6);
        btn_box.set_halign(gtk::Align::End);
        btn_box.set_margin_top(6);

        // Copy bash function button (primary action).
        let func = func_display;
        let copy_func_btn = gtk::Button::builder()
            .label("Copy Config Block")
            .css_classes(vec!["flat", "suggested-action"])
            .build();
        let func_clone = func.clone();
        copy_func_btn.connect_clicked(move |_| {
            copy_to_clipboard(&func_clone);
        });
        btn_box.append(&copy_func_btn);

        // Open Config File button.
        let open_cfg_btn = gtk::Button::builder()
            .label("Open Config File")
            .css_classes(vec!["flat"])
            .build();
        open_cfg_btn.connect_clicked(move |_| {
            open_claude_cli_config();
        });
        btn_box.append(&open_cfg_btn);

        content.append(&btn_box);

        expander.add_row(&content);
        expander
    }

    /// Build the Claude Desktop setup guide expander row.
pub fn build_claude_desktop_expander(config: &Config) -> ExpanderRow {
        let port = config.proxy_port();
        let base_url = format!("http://127.0.0.1:{port}/v1", port = port);

        let expander = ExpanderRow::builder()
            .title("<span foreground=\"#DA7756\" weight=\"bold\">Claude Desktop</span>")
            .build();

        let content = gtk::Box::new(Orientation::Vertical, 4);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(6);
        content.set_margin_bottom(6);

        // Gateway URL row.
        let url_row = gtk::Box::new(Orientation::Horizontal, 6);
        let url_label = gtk::Label::builder()
            .label("Gateway URL")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        url_row.append(&url_label);

        let url_value = gtk::Label::builder()
            .label(&base_url)
            .xalign(0.0)
            .selectable(true)
            .build();
        url_row.append(&url_value);

        let copy_url_btn = gtk::Button::builder()
            .label("Copy")
            .css_classes(vec!["flat"])
            .build();
        let base_url_clone = base_url.clone();
        copy_url_btn.connect_clicked(move |_| {
            copy_to_clipboard(&base_url_clone);
        });
        url_row.append(&copy_url_btn);

        content.append(&url_row);

        // API Key row.
        let key_row = gtk::Box::new(Orientation::Horizontal, 6);
        let key_label = gtk::Label::builder()
            .label("API Key")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        key_row.append(&key_label);

        let key_value = gtk::Label::builder()
            .label("swai-local")
            .xalign(0.0)
            .selectable(true)
            .build();
        key_row.append(&key_value);

        let copy_key_btn = gtk::Button::builder()
            .label("Copy")
            .css_classes(vec!["flat"])
            .build();
        copy_key_btn.connect_clicked(move |_| {
            copy_to_clipboard("swai-local");
        });
        key_row.append(&copy_key_btn);

        content.append(&key_row);

        // Instructions label.
        let instructions = gtk::Label::builder()
            .label(
                &format!(
                    "To connect Claude Desktop:\n\
                     1. Enable Developer Mode in Claude Desktop's Help menu.\n\
                     2. Open Developer Menu → Configure Third-Party Inference → Gateway.\n\
                     3. Set Gateway Base URL to http://127.0.0.1:{port}/ and API Key to \"swai-local\".\n\
                     4. Under Models → Model list, click \"+ Add model\": set Model ID to \"claude\" and Display name to \"SWAI\". Toggle 1M-context ON.",
                    port = port,
                ),
            )
            .use_markup(false)
            .wrap(true)
            .xalign(0.0)
            .margin_top(6)
            .build();
        content.append(&instructions);

        expander.add_row(&content);
        expander
    }

    /// Build the OpenAI Codex CLI setup guide expander row.
pub fn build_codex_expander(config: &Config) -> ExpanderRow {
        let port = config.proxy_port();
        let base_url = format!("http://127.0.0.1:{port}/v1", port = port);

        let expander = ExpanderRow::builder()
            .title("<span foreground=\"#7297FF\" weight=\"bold\">OpenAI Codex CLI</span>")
            .build();

        let content = gtk::Box::new(Orientation::Vertical, 4);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(6);
        content.set_margin_bottom(6);

        // Config file path row.
        let config_path_row = gtk::Box::new(Orientation::Horizontal, 6);
        let config_path_label = gtk::Label::builder()
            .label("Config file")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        config_path_row.append(&config_path_label);

        let config_path_value = gtk::Label::builder()
            .label("~/.codex/config.toml")
            .xalign(0.0)
            .selectable(true)
            .build();
        config_path_row.append(&config_path_value);

        content.append(&config_path_row);

        // Base URL row.
        let url_row = gtk::Box::new(Orientation::Horizontal, 6);
        let url_label = gtk::Label::builder()
            .label("Base URL")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        url_row.append(&url_label);

        let url_value = gtk::Label::builder()
            .label(&base_url)
            .xalign(0.0)
            .selectable(true)
            .build();
        url_row.append(&url_value);

        let copy_url_btn = gtk::Button::builder()
            .label("Copy")
            .css_classes(vec!["flat"])
            .build();
        let base_url_clone = base_url.clone();
        copy_url_btn.connect_clicked(move |_| {
            copy_to_clipboard(&base_url_clone);
        });
        url_row.append(&copy_url_btn);

        content.append(&url_row);

        // API Key row.
        let key_row = gtk::Box::new(Orientation::Horizontal, 6);
        let key_label = gtk::Label::builder()
            .label("API Key")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        key_row.append(&key_label);

        let key_value = gtk::Label::builder()
            .label("swai-local")
            .xalign(0.0)
            .selectable(true)
            .build();
        key_row.append(&key_value);

        let copy_key_btn = gtk::Button::builder()
            .label("Copy")
            .css_classes(vec!["flat"])
            .build();
        copy_key_btn.connect_clicked(move |_| {
            copy_to_clipboard("swai-local");
        });
        key_row.append(&copy_key_btn);

        content.append(&key_row);

        // Instructions label.
        let instructions = gtk::Label::builder()
            .label(
                &format!(
                    "Add the following to `~/.codex/config.toml`:\n\n\
                     model_provider = \"swai\"\n\n\
                     [model_providers.swai]\n\
                     name = \"SWAI Local AI\"\n\
                     base_url = \"{base_url}\"\n\
                     wire_api = \"responses\"\n\
                     api_key = \"local\"\n\n\
                     Restart Codex CLI after editing the config file.",
                    base_url = base_url,
                ),
            )
            .use_markup(false)
            .wrap(true)
            .xalign(0.0)
            .margin_top(6)
            .build();
        content.append(&instructions);

        // Button row: "Copy Config Block" and "Open Config File".
        let btn_box = gtk::Box::new(Orientation::Horizontal, 6);
        btn_box.set_halign(gtk::Align::End);
        btn_box.set_margin_top(6);

        let config_block = format!(
            "model_provider = \"swai\"\n\n[model_providers.swai]\nname = \"SWAI Local AI\"\nbase_url = \"{base_url}\"\nwire_api = \"responses\"\napi_key = \"local\"",
            base_url = base_url,
        );
        let copy_config_btn = gtk::Button::builder()
            .label("Copy Config Block")
            .css_classes(vec!["flat", "suggested-action"])
            .build();
        let config_block_clone = config_block.clone();
        copy_config_btn.connect_clicked(move |_| {
            copy_to_clipboard(&config_block_clone);
        });
        btn_box.append(&copy_config_btn);

        // Open Config File button.
        let open_cfg_btn = gtk::Button::builder()
            .label("Open Config File")
            .css_classes(vec!["flat"])
            .build();
        open_cfg_btn.connect_clicked(move |_| {
            open_codex_config();
        });
        btn_box.append(&open_cfg_btn);

        content.append(&btn_box);

        expander.add_row(&content);
        expander
    }
