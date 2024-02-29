// completer for our Read-Evaluate-Print Loop (REPL)
// used for autocompleting commands and suggestions based on user input
use super::{ReplCommand, REPL_COMMANDS};

use crate::config::GlobalConfig;

use reedline::{Completer, Span, Suggestion};
use std::collections::HashMap;

// Here we implement the Completer trait for ReplCompleter
// It generates suggestions based on user input
impl Completer for ReplCompleter {
    // this function analyzes the input line,
    // determines the current command and arguments, and generates appropriate suggestions
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        // initializing an empty vector suggestions to store the generated suggestions
        let mut suggestions = vec![];
        let line = &line[0..pos];
        // splits the line into individual parts
        let mut parts = split_line(line);
        // If the parts list is empty, we return an empty vector of suggestions
        if parts.is_empty() {
            return suggestions;
        }
        // the first part of the input line starts with ":::#", we remove it
        if parts[0].0 == r#":::"# {
            parts.remove(0);
        }

        let parts_len = parts.len();
        if parts_len == 0 {
            return suggestions;
        }
        // extracts the command(cmd) and its start position(cmd_start) from the first part
        let (cmd, cmd_start) = parts[0];

        // if the command doesn't start with a dot ('.'), we return an empty vector of suggestions
        if !cmd.starts_with('.') {
            return suggestions;
        }

        let state = self.config.read().get_state();

        // This filters available repl commands based on the current state and input
        let commands: Vec<_> = self
            .commands
            .iter()
            .filter(|cmd| {
                if cmd.unavailable(&state) {
                    return false;
                }
                let line = parts
                    .iter()
                    .take(2)
                    .map(|(v, _)| *v)
                    .collect::<Vec<&str>>()
                    .join(" ");
                cmd.name.starts_with(&line)
            })
            .collect();

        if parts_len > 1 {
            let span = Span::new(parts[parts_len - 1].1, pos);
            let args: Vec<&str> = parts.iter().skip(1).map(|(v, _)| *v).collect();
            suggestions.extend(
                self.config
                    .read()
                    .repl_complete(cmd, &args)
                    .iter()
                    .map(|name| create_suggestion(name.clone(), None, span)),
            )
        }

        // if there are no suggestions generated from the arguments, we create suggestions for available repl commands
        if suggestions.is_empty() {
            let span = Span::new(cmd_start, pos);
            suggestions.extend(commands.iter().map(|cmd| {
                let name = cmd.name;
                let description = cmd.description;
                let has_group = self.groups.get(name).map(|v| *v > 1).unwrap_or_default();
                let name = if has_group {
                    name.to_string()
                } else {
                    format!("{name} ")
                };
                create_suggestion(name, Some(description.to_string()), span)
            }))
        }
        // we return the suggestions
        suggestions
    }
}

// The struct ReplCompleter
pub struct ReplCompleter {
    config: GlobalConfig,
    commands: Vec<ReplCommand>,
    groups: HashMap<&'static str, usize>,
}

impl ReplCompleter {
    // constructor for the ReplCompleter
    pub fn new(config: &GlobalConfig) -> Self {
        let mut groups = HashMap::new();

        let commands: Vec<ReplCommand> = REPL_COMMANDS.to_vec();

        for cmd in REPL_COMMANDS.iter() {
            let name = cmd.name;
            if let Some(count) = groups.get(name) {
                groups.insert(name, count + 1);
            } else {
                groups.insert(name, 1);
            }
        }

        Self {
            config: config.clone(),
            commands,
            groups,
        }
    }
}

// This function creates a suggestion object with the specified value, description, and span.
fn create_suggestion(value: String, description: Option<String>, span: Span) -> Suggestion {
    Suggestion {
        value,
        description,
        extra: None,
        span,
        append_whitespace: false,
    }
}

// This function parses the input line into individual parts, separating commands and arguments
fn split_line(line: &str) -> Vec<(&str, usize)> {
    // initializing an empty vector parts to store parts of the input
    let mut parts = vec![];
    let mut part_start = None;
    // iterates through the characters of the input line
    for (i, ch) in line.char_indices() {
        // If the character is a space, we check if there's a current part being collected
        if ch == ' ' {
            // If so, we push the substring from the start index to the current index
            if let Some(s) = part_start {
                parts.push((&line[s..i], s));
                part_start = None;
            }
        } else if part_start.is_none() {
            part_start = Some(i)
        }
    }
    // checking if there's still a part being collected
    if let Some(s) = part_start {
        // pushhing the remaining substring from the start index to the end into the vector
        parts.push((&line[s..], s));
    } else {
        // else, adding an empty part to the parts vector
        parts.push(("", line.len()))
    }
    // returning the vectors
    parts
}

#[test]
fn test_split_line() {
    assert_eq!(split_line(".role coder"), vec![(".role", 0), ("coder", 6)],);
    assert_eq!(
        split_line(" .role   coder"),
        vec![(".role", 1), ("coder", 9)],
    );
    assert_eq!(
        split_line(".set highlight "),
        vec![(".set", 0), ("highlight", 5), ("", 15)],
    );
    assert_eq!(
        split_line(".set highlight t"),
        vec![(".set", 0), ("highlight", 5), ("t", 15)],
    );
}
