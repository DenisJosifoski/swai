use gtk::prelude::*;
use gtk4 as gtk;

use super::view::ModelCard;

impl ModelCard {
    /// Set the live generation speed label (e.g., "⚡ 41.5 tok/s").
    ///
    /// Called from the main thread when a new /slots response includes
    /// predicted_per_second. The label is shown only when the model is Ready.
    pub fn set_speed(&self, speed: f64) {
        self.block_signals();

        if speed > 0.0 {
            let text = format!("⚡ {:.1} tok/s", speed);
            self.speed_label.set_text(&text);
            self.speed_label
                .set_css_classes(&["caption", "speed-label"]);
            self.speed_label.set_visible(true);
        }

        self.unblock_signals();
    }

    /// Set the live prompt evaluation speed label (e.g., "📥 450.2 p-tok/s").
    ///
    /// Called from the main thread when a new /slots response includes
    /// prompt_per_second. The label is shown only when the model is Ready.
    pub fn set_prompt_speed(&self, prompt_per_second: f64) {
        self.block_signals();

        if prompt_per_second > 0.0 {
            let text = format!("📥 {:.1} p-tok/s", prompt_per_second);
            self.prompt_speed_label.set_text(&text);
            self.prompt_speed_label
                .set_css_classes(&["caption", "prompt-speed-label"]);
            self.prompt_speed_label.set_visible(true);
        }

        self.unblock_signals();
    }

    /// Set the live stopwatch label (e.g., "⏱ 4.2s").
    ///
    /// Called from the main thread when a SlotUpdate includes
    /// elapsed_duration_sec. The label shows live time during generation
    /// and latches the final time when generation completes.
    pub fn set_stopwatch(&self, elapsed_duration_sec: Option<f64>) {
        self.block_signals();

        if let Some(duration) = elapsed_duration_sec {
            let text = format!("⏱ {:.1}s", duration);
            self.stopwatch_label.set_text(&text);
            self.stopwatch_label
                .set_css_classes(&["caption", "stopwatch-label"]);
            self.stopwatch_label.set_visible(true);
        }

        self.unblock_signals();
    }

    /// Clear the prompt speed label (used when model stops or enters transitional state).
    pub fn clear_prompt_speed(&self) {
        self.block_signals();
        self.prompt_speed_label.set_text("");
        self.prompt_speed_label
            .set_css_classes(&["dim-label", "caption"]);
        self.prompt_speed_label.set_visible(false);
        self.unblock_signals();
    }

    /// Clear the stopwatch label (used when model stops or enters transitional state).
    pub fn clear_stopwatch(&self) {
        self.block_signals();
        self.stopwatch_label.set_text("");
        self.stopwatch_label
            .set_css_classes(&["dim-label", "caption"]);
        self.stopwatch_label.set_visible(false);
        self.unblock_signals();
    }

    /// Clear the speed label (used when model stops or enters transitional state).
    pub fn clear_speed(&self) {
        self.block_signals();
        self.speed_label.set_text("");
        self.speed_label.set_css_classes(&["dim-label", "caption"]);
        self.speed_label.set_visible(false);
        self.unblock_signals();
    }
}
