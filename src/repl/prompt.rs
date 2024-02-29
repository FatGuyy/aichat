use crate::config::GlobalConfig;

use reedline::{Prompt, PromptHistorySearch, PromptHistorySearchStatus};
use std::borrow::Cow;

// struct for rendering prompts in a REPL environment
#[derive(Clone)]
pub struct ReplPrompt {
    config: GlobalConfig,
}

impl ReplPrompt {
    // constructor function that initializes a new ReplPrompt instance
    pub fn new(config: &GlobalConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }
}

impl Prompt for ReplPrompt {
    // this function renders the left part of the prompt
    fn render_prompt_left(&self) -> Cow<str> {
        // retrieving the prompt content from the global configuration
        Cow::Owned(self.config.read().render_prompt_left())
    }

    // this function renders the right part of the prompt
    fn render_prompt_right(&self) -> Cow<str> {
        Cow::Owned(self.config.read().render_prompt_right())
    }

    // this function renders the prompt indicator
    fn render_prompt_indicator(&self, _prompt_mode: reedline::PromptEditMode) -> Cow<str> {
        Cow::Borrowed("")
    }

    // this function renders the indicator for multiline input
    fn render_prompt_multiline_indicator(&self) -> Cow<str> {
        Cow::Borrowed("")
    }

    // this function renders the history search indicator
    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch, // object containing information about the history search status and term
    ) -> Cow<str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        // NOTE: magic strings, given there is logic on how these compose I am not sure if it
        // is worth extracting in to static constant
        Cow::Owned(format!(
            "({}reverse-search: {}) ",
            prefix, history_search.term
        ))
    }
}
