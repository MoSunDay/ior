//! Local-LLM polish pipeline planning surface for ior (phase 2
//! placeholder). Pure functions only, no dependencies, no model code.
//!
//! M0 ships the decision helpers and prompt templates:
//! - [`choose_degradation`]: latency budget vs. async correction policy.
//! - [`build_punctuate_prompt`] / [`build_polish_prompt`]: templates for
//!   the local Qwen models.
//! - [`recommend_model`]: model tier by estimated prefill latency.
//!
//! Actual llama.cpp-style inference lands in phase 2 (V1-V4).

/// Maximum wall time before an LLM-assisted edit degrades to async mode.
pub const LATENCY_BUDGET_MS: u32 = 1500;

/// How an LLM-assisted result is delivered to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Degradation {
    /// Fits the latency budget: commit the result directly.
    Immediate,
    /// Too slow: commit raw text now and correct it asynchronously.
    AsyncCorrect,
}

/// Pick a delivery mode from an estimated end-to-end latency.
pub fn choose_degradation(estimated_ms: u32) -> Degradation {
    if estimated_ms <= LATENCY_BUDGET_MS {
        Degradation::Immediate
    } else {
        Degradation::AsyncCorrect
    }
}

/// Prompt asking the model to add Chinese punctuation. The model must
/// output only the result, with no explanations.
pub fn build_punctuate_prompt(raw: &str) -> String {
    format!(
        "You are a punctuation assistant for Chinese text.\n\
         Add proper Chinese punctuation to the input. Do not add, remove, or \
         rewrite any word.\n\
         Output ONLY the punctuated result.\n\n\
         Input: {raw}\n\
         Output:"
    )
}

/// Prompt asking the model to polish a speech transcript: smooth
/// colloquial wording, add Chinese punctuation, and correct proper nouns.
/// The user-vocabulary section appears only when `user_words` is
/// non-empty, rendered as a comma-joined list.
pub fn build_polish_prompt(raw: &str, user_words: &[&str]) -> String {
    let vocabulary = if user_words.is_empty() {
        String::new()
    } else {
        format!(
            "\nUser vocabulary (keep these proper nouns exactly as written): {}\n",
            user_words.join(", ")
        )
    };
    format!(
        "You are a speech-to-text polisher for Chinese.\n\
         Rewrite the transcript so it reads smoothly: remove filler words, \
         add Chinese punctuation, and correct proper nouns.{vocabulary}\n\
         Output ONLY the polished result.\n\n\
         Transcript: {raw}\n\
         Output:"
    )
}

/// Recommend a local Qwen model tier by estimated prefill latency.
pub fn recommend_model(estimated_prefill_ms: u32) -> &'static str {
    if estimated_prefill_ms <= 300 {
        "qwen2.5-1.5b-instruct-q4"
    } else if estimated_prefill_ms <= 800 {
        "qwen2.5-3b-instruct-q4"
    } else {
        "qwen2.5-7b-instruct-q4"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degradation_switches_just_above_budget() {
        assert_eq!(choose_degradation(0), Degradation::Immediate);
        assert_eq!(
            choose_degradation(LATENCY_BUDGET_MS),
            Degradation::Immediate
        );
        assert_eq!(
            choose_degradation(LATENCY_BUDGET_MS - 1),
            Degradation::Immediate
        );
        assert_eq!(
            choose_degradation(LATENCY_BUDGET_MS + 1),
            Degradation::AsyncCorrect
        );
        assert_eq!(choose_degradation(60_000), Degradation::AsyncCorrect);
    }

    #[test]
    fn punctuate_prompt_quotes_raw_text_once() {
        let prompt = build_punctuate_prompt("ni hao shijie");
        assert!(prompt.contains("ni hao shijie"), "{prompt}");
        assert!(prompt.contains("Output ONLY"), "{prompt}");
        assert_eq!(prompt.matches("ni hao shijie").count(), 1);
    }

    #[test]
    fn polish_prompt_includes_user_words_only_when_provided() {
        let with_words = build_polish_prompt("raw text", &["Rust", "fcitx5"]);
        assert!(with_words.contains("Rust, fcitx5"), "{with_words}");
        assert!(with_words.contains("User vocabulary"), "{with_words}");
        assert!(with_words.contains("raw text"), "{with_words}");

        let without_words = build_polish_prompt("raw text", &[]);
        assert!(
            !without_words.contains("User vocabulary"),
            "section header must be absent: {without_words}"
        );
        assert!(without_words.contains("raw text"));
    }

    #[test]
    fn recommend_model_follows_prefill_thresholds() {
        assert_eq!(recommend_model(0), "qwen2.5-1.5b-instruct-q4");
        assert_eq!(recommend_model(300), "qwen2.5-1.5b-instruct-q4");
        assert_eq!(recommend_model(301), "qwen2.5-3b-instruct-q4");
        assert_eq!(recommend_model(800), "qwen2.5-3b-instruct-q4");
        assert_eq!(recommend_model(801), "qwen2.5-7b-instruct-q4");
        assert_eq!(recommend_model(5_000), "qwen2.5-7b-instruct-q4");
    }
}
