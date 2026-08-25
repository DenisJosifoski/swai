//! Context budget calculations and management.
//!
//! This module computes dynamic context budgets based on a model's context window size
//! in tokens. It determines various thresholds for compaction, history retention, and
//! tool result truncation.
//!
//! The trigger threshold is configurable by the user (50%–85%, default 70%). The
//! history retention ratio is derived proportionally at 50% of the trigger ratio.
//! Tool result truncation uses a fixed 8% of the context window.

/// Minimum allowed compaction trigger threshold (percentage of context window).
pub const MIN_THRESHOLD_PCT: u8 = 50;

/// Maximum allowed compaction trigger threshold (percentage of context window).
pub const MAX_THRESHOLD_PCT: u8 = 85;

/// Default compaction trigger threshold (70% of context window).
pub const DEFAULT_THRESHOLD_PCT: u8 = 70;

/// Proportional factor: history retention ratio = compaction_ratio * 0.50.
const HISTORY_RETENTION_FACTOR: f64 = 0.50;

/// Proportional factor: tool result budget = 8% of context window (in chars).
const TOOL_RESULT_FACTOR: f64 = 8.0;

#[derive(Debug, Clone)]
pub struct ContextBudget {
    /// The model's context window size in tokens
    pub ctx_tokens: usize,
    /// Maximum characters to retain in conversation history
    pub max_history_chars: usize,
    /// Payload size in chars that triggers compaction
    pub compaction_trigger_chars: usize,
    /// Maximum chars per individual tool result before truncation
    pub tool_result_max_chars: usize,
    /// Number of most-recent turns to never evict
    pub recent_turns_keep: usize,
    /// After this many reads of the same file, compress re-reads
    pub file_reread_compress_threshold: usize,
}

impl ContextBudget {
    /// Create a context budget based on the model's context size in tokens and
    /// a user-configurable compaction trigger threshold (50–85%).
    ///
    /// Derived ratios:
    /// - `compaction_trigger_chars` = ctx_tokens * 4 * (threshold / 100)
    /// - `max_history_chars`       = ctx_tokens * 4 * (threshold / 100) * 0.50
    /// - `tool_result_max_chars`   = clamp(ctx_tokens * 4 * 0.08, 8_000, 64_000)
    pub fn from_ctx_size_and_threshold(ctx_tokens: usize, threshold_pct: u8) -> Self {
        let clamped = threshold_pct.clamp(MIN_THRESHOLD_PCT, MAX_THRESHOLD_PCT);
        let trigger_ratio = clamped as f64 / 100.0;

        let total_char_budget = ctx_tokens as f64 * 4.0;
        let compaction_trigger_chars = (total_char_budget * trigger_ratio) as usize;
        let max_history_chars =
            (total_char_budget * trigger_ratio * HISTORY_RETENTION_FACTOR) as usize;
        let tool_result_max_chars =
            (total_char_budget * TOOL_RESULT_FACTOR / 100.0).clamp(8_000.0, 64_000.0) as usize;

        Self {
            ctx_tokens,
            max_history_chars,
            compaction_trigger_chars,
            tool_result_max_chars,
            recent_turns_keep: (ctx_tokens / 16_384).max(3).min(16),
            file_reread_compress_threshold: 2,
        }
    }

    /// Create a context budget based on the model's context size in tokens.
    ///
    /// Uses the default threshold of 70%. Kept for backward compatibility with
    /// callers that haven't been updated to read the user-configurable threshold
    /// from the proxy state yet.
    pub fn from_ctx_size(ctx_tokens: usize) -> Self {
        Self::from_ctx_size_and_threshold(ctx_tokens, DEFAULT_THRESHOLD_PCT)
    }

    /// Default preset for a 64k token context window.
    pub fn default_64k() -> Self {
        Self::from_ctx_size(65_536)
    }

    /// Default preset for a 128k token context window.
    pub fn default_128k() -> Self {
        Self::from_ctx_size(131_072)
    }

    /// Default preset for a 262k token context window.
    pub fn default_262k() -> Self {
        Self::from_ctx_size(262_144)
    }

    /// Check if a given payload size should trigger history compaction.
    pub fn should_trigger_compaction(&self, payload_size_chars: usize) -> bool {
        payload_size_chars >= self.compaction_trigger_chars
    }

    /// Check if reading a file should be compressed based on the number of times it has been read.
    pub fn should_compress_reread(&self, read_count: usize) -> bool {
        read_count >= self.file_reread_compress_threshold
    }

    /// Compute a human-readable summary of this budget for UI display.
    ///
    /// Returns a string like:
    ///   "Compaction triggers at ~183k chars, retaining ~91k chars"
    pub fn summary_display(&self) -> String {
        format!(
            "Compaction triggers at ~{} chars, retaining ~{} chars",
            format_chars(self.compaction_trigger_chars),
            format_chars(self.max_history_chars),
        )
    }
}

/// Format a character count as a human-readable string with appropriate units.
fn format_chars(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}k", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self::default_64k()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_64k_default() {
        let budget = ContextBudget::default_64k();
        assert_eq!(budget.ctx_tokens, 65_536);
        // 65536 * 4 * 0.70 = 183,500.8 → 183,500
        assert_eq!(budget.compaction_trigger_chars, 183_500);
        // * 0.50 = 91,750.4 → 91,750
        assert_eq!(budget.max_history_chars, 91_750);
        // 65536 * 4 * 0.08 = 20,971.52 → 20,971
        assert_eq!(budget.tool_result_max_chars, 20_971);
        assert_eq!(budget.recent_turns_keep, 4);
    }

    #[test]
    fn test_budget_128k_default() {
        let budget = ContextBudget::default_128k();
        assert_eq!(budget.ctx_tokens, 131_072);
        assert_eq!(budget.recent_turns_keep, 8);
    }

    #[test]
    fn test_budget_262k_default() {
        let budget = ContextBudget::default_262k();
        assert_eq!(budget.ctx_tokens, 262_144);
        assert_eq!(budget.recent_turns_keep, 16);
        assert_eq!(budget.tool_result_max_chars, 64_000);
    }

    #[test]
    fn test_budget_32k_default() {
        let budget = ContextBudget::from_ctx_size(32_768);
        assert_eq!(budget.ctx_tokens, 32_768);
        assert_eq!(budget.recent_turns_keep, 3);
        assert_eq!(budget.tool_result_max_chars, 10_485);
    }

    #[test]
    fn test_threshold_50_percent() {
        let budget = ContextBudget::from_ctx_size_and_threshold(65_536, 50);
        // 65536 * 4 * 0.50 = 131,072
        assert_eq!(budget.compaction_trigger_chars, 131_072);
        // * 0.50 = 65,536
        assert_eq!(budget.max_history_chars, 65_536);
    }

    #[test]
    fn test_threshold_85_percent() {
        let budget = ContextBudget::from_ctx_size_and_threshold(65_536, 85);
        // 65536 * 4 * 0.85 = 222,822.4 → 222,822
        assert_eq!(budget.compaction_trigger_chars, 222_822);
        // * 0.50 = 111,411
        assert_eq!(budget.max_history_chars, 111_411);
    }

    #[test]
    fn test_threshold_clamped_above() {
        let budget = ContextBudget::from_ctx_size_and_threshold(65_536, 100);
        // Should clamp to 85%
        assert_eq!(budget.compaction_trigger_chars, 222_822);
    }

    #[test]
    fn test_threshold_clamped_below() {
        let budget = ContextBudget::from_ctx_size_and_threshold(65_536, 10);
        // Should clamp to 50%
        assert_eq!(budget.compaction_trigger_chars, 131_072);
    }

    #[test]
    fn test_from_ctx_size_uses_70_default() {
        let budget = ContextBudget::from_ctx_size(65_536);
        let expected = ContextBudget::from_ctx_size_and_threshold(65_536, 70);
        assert_eq!(
            budget.compaction_trigger_chars,
            expected.compaction_trigger_chars
        );
        assert_eq!(budget.max_history_chars, expected.max_history_chars);
    }

    #[test]
    fn test_should_trigger_compaction() {
        let budget = ContextBudget::default_64k();
        let trigger = budget.compaction_trigger_chars;
        assert!(!budget.should_trigger_compaction(trigger - 1));
        assert!(budget.should_trigger_compaction(trigger));
        assert!(budget.should_trigger_compaction(trigger + 1));
    }

    #[test]
    fn test_should_compress_reread() {
        let budget = ContextBudget::default_64k();
        assert!(!budget.should_compress_reread(0));
        assert!(!budget.should_compress_reread(1));
        assert!(budget.should_compress_reread(2));
        assert!(budget.should_compress_reread(3));
    }

    #[test]
    fn test_default() {
        let budget = ContextBudget::default();
        let budget_64k = ContextBudget::default_64k();
        assert_eq!(budget.ctx_tokens, budget_64k.ctx_tokens);
    }

    #[test]
    fn test_summary_display() {
        let budget = ContextBudget::default_64k();
        let s = budget.summary_display();
        assert!(s.contains("Compaction triggers at"));
        assert!(s.contains("retaining"));
    }

    #[test]
    fn test_summary_display_large_numbers() {
        let budget = ContextBudget::default_262k();
        let s = budget.summary_display();
        // 262k trigger should show as ~734k
        assert!(s.contains("734k"));
    }

    // ─── Preset table verification ──────────────────────────────────────────

    #[test]
    fn test_preset_32k_at_70_default() {
        let budget = ContextBudget::from_ctx_size(32_768);
        // 32768 * 4 * 0.70 = 91,750.4 → 91,750
        assert_eq!(budget.compaction_trigger_chars, 91_750);
        // * 0.50 = 45,875.2 → 45,875
        assert_eq!(budget.max_history_chars, 45_875);
    }

    #[test]
    fn test_preset_64k_at_70_default() {
        let budget = ContextBudget::from_ctx_size(65_536);
        assert_eq!(budget.compaction_trigger_chars, 183_500);
        assert_eq!(budget.max_history_chars, 91_750);
    }

    #[test]
    fn test_preset_128k_at_70_default() {
        let budget = ContextBudget::from_ctx_size(131_072);
        // 131072 * 4 * 0.70 = 367,001.6 → 367,001
        assert_eq!(budget.compaction_trigger_chars, 367_001);
        // * 0.50 = 183,500.8 → 183,500
        assert_eq!(budget.max_history_chars, 183_500);
    }

    #[test]
    fn test_preset_256k_at_70_default() {
        let budget = ContextBudget::from_ctx_size(262_144);
        // 262144 * 4 * 0.70 = 734,003.2 → 734,003
        assert_eq!(budget.compaction_trigger_chars, 734_003);
        // * 0.50 = 367,001.6 → 367,001
        assert_eq!(budget.max_history_chars, 367_001);
    }

    #[test]
    fn test_preset_32k_at_50() {
        let budget = ContextBudget::from_ctx_size_and_threshold(32_768, 50);
        // 32768 * 4 * 0.50 = 65,536
        assert_eq!(budget.compaction_trigger_chars, 65_536);
        assert_eq!(budget.max_history_chars, 32_768);
    }

    #[test]
    fn test_preset_64k_at_80() {
        let budget = ContextBudget::from_ctx_size_and_threshold(65_536, 80);
        // 65536 * 4 * 0.80 = 209,715.2 → 209,715
        assert_eq!(budget.compaction_trigger_chars, 209_715);
        // * 0.50 = 104,857
        assert_eq!(budget.max_history_chars, 104_857);
    }

    #[test]
    fn test_tool_result_ratio_independent_of_threshold() {
        let low = ContextBudget::from_ctx_size_and_threshold(65_536, 50);
        let high = ContextBudget::from_ctx_size_and_threshold(65_536, 85);
        // tool_result_max_chars should be identical regardless of threshold
        assert_eq!(low.tool_result_max_chars, high.tool_result_max_chars);
    }
}
