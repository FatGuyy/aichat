use super::REPL_COMMANDS;

use crate::config::GlobalConfig;

use nu_ansi_term::{Color, Style};
use reedline::{Highlighter, StyledText};

// this a struct for syntax highlighting in a REPL environment
pub struct ReplHighlighter {
    config: GlobalConfig,
}

//
impl ReplHighlighter {
    // this is a constructor function that initializes a new ReplHighlighter instance
    pub fn new(config: &GlobalConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }
}

// implementing Highlighter trait for ReplHighlighter
impl Highlighter for ReplHighlighter {
    // the function holds the code for Highlighting
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        // initializing a default color
        let color = Color::Default;
        // checks if syntax highlighting is enabled in the global configuration
        let match_color = if self.config.read().highlight {
            Color::Green
        } else {
            color
        };

        // object to store the styled text
        let mut styled_text = StyledText::new();

        // if any REPL command names are found in the line, it highlights
        if REPL_COMMANDS.iter().any(|cmd| line.contains(cmd.name)) {
            // iterating over the repl commands and checking if each command name is present in the line
            let matches: Vec<&str> = REPL_COMMANDS
                .iter()
                .filter(|cmd| line.contains(cmd.name))
                .map(|cmd| cmd.name)
                .collect();
            // finding the longest matching command name
            let longest_match = matches.iter().fold(String::new(), |acc, &item| {
                // spliting the line into three parts: before the command, the matched command, after the command
                if item.len() > acc.len() {
                    item.to_string()
                } else {
                    acc
                }
            });
            let buffer_split: Vec<&str> = line.splitn(2, &longest_match).collect();

            // styling each part separately
            styled_text.push((Style::new().fg(color), buffer_split[0].to_string())); // text before the command given default color
            styled_text.push((Style::new().fg(match_color), longest_match)); // command is styled with the match color (green)
            styled_text.push((Style::new().fg(color), buffer_split[1].to_string()));
            // text after the command is styled with the default color again
        } else {
            // if no REPL commands are found in the line, we style the entire line with the default color
            styled_text.push((Style::new().fg(color), line.to_string()));
        }

        // returning the stylized text
        styled_text
    }
}
