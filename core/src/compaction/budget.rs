//! Context budget calculations and management.
//!
//! This module computes dynamic context budgets based on a model's context window size
//! in tokens. It determines various thresholds for compaction, history retention, and
//! tool result truncation.

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
    /// Create a context budget based on the model's context size in tokens.
    pub fn from_ctx_size(ctx_tokens: usize) -> Self {
        Self {
            ctx_tokens,
            max_history_chars: ctx_tokens * 4 * 35 / 100,
            compaction_trigger_chars: ctx_tokens * 4 * 70 / 100,
            tool_result_max_chars: (ctx_tokens * 4 * 8 / 100).clamp(8_000, 64_000),
            recent_turns_keep: (ctx_tokens / 16_384).max(3).min(16),
            file_reread_compress_threshold: 2,
        }
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
    fn test_budget_64k() {
        let budget = ContextBudget::default_64k();
        assert_eq!(budget.ctx_tokens, 65_536);
        assert_eq!(budget.max_history_chars, 91_750); // 65536 * 4 * 0.35
        assert_eq!(budget.compaction_trigger_chars, 183_500); // 65536 * 4 * 0.70
        assert_eq!(budget.tool_result_max_chars, 20_971); // 65536 * 4 * 0.08
        assert_eq!(budget.recent_turns_keep, 4); // 65536 / 16384
    }

    #[test]
    fn test_budget_128k() {
        let budget = ContextBudget::default_128k();
        assert_eq!(budget.ctx_tokens, 131_072);
        assert_eq!(budget.recent_turns_keep, 8);
    }

    #[test]
    fn test_budget_262k() {
        let budget = ContextBudget::default_262k();
        assert_eq!(budget.ctx_tokens, 262_144);
        assert_eq!(budget.recent_turns_keep, 16);
        assert_eq!(budget.tool_result_max_chars, 64_000); // clamped
    }

    #[test]
    fn test_budget_32k() {
        let budget = ContextBudget::from_ctx_size(32_768);
        assert_eq!(budget.ctx_tokens, 32_768);
        assert_eq!(budget.recent_turns_keep, 3); // max(3)
        assert_eq!(budget.tool_result_max_chars, 10_485); // 32768 * 4 * 0.08
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
}
