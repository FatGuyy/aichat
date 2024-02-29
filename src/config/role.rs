use crate::client::{Message, MessageContent, MessageRole};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::Input;

// a constant string used as a placeholder for input within the role's prompt
const INPUT_PLACEHOLDER: &str = "__INPUT__";

// struct representing the role of the user
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Role {
    /// Role name
    pub name: String,
    /// Prompt text
    pub prompt: String,
    /// What sampling temperature to use, between 0 and 2
    pub temperature: Option<f64>,
}

impl Role {
    // this is function for Serializing the role struct into a yaml string and trims whitespace
    pub fn info(&self) -> Result<String> {
        let output = serde_yaml::to_string(&self)
            .with_context(|| format!("Unable to show info about role {}", &self.name))?;
        // Returning a Result<String> containing the YAML representation of the role
        Ok(output.trim_end().to_string())
    }

    // this function checks if the prompt contains the INPUT_PLACEHOLDER
    pub fn embedded(&self) -> bool {
        // returning a boolean indicating whether the prompt is embedded
        self.prompt.contains(INPUT_PLACEHOLDER)
    }

    // this funciton replaces placeholder arguments in the prompt with actual values derived from the role's name
    pub fn complete_prompt_args(&mut self, name: &str) {
        self.name = name.to_string();
        self.prompt = complete_prompt_args(&self.prompt, &self.name);
    }

    // this function compares the role's name with the provided name
    // returns true if the names match, false otherwise
    pub fn match_name(&self, name: &str) -> bool {
        // Handling cases where the role name contains arguments separated by colons
        if self.name.contains(':') {
            let role_name_parts: Vec<&str> = self.name.split(':').collect();
            let name_parts: Vec<&str> = name.split(':').collect();
            role_name_parts[0] == name_parts[0] && role_name_parts.len() == name_parts.len()
        } else {
            self.name == name
        }
    }

    // this funciton renders the input and replaces the input placeholder in the role's prompt with the rendered input
    pub fn echo_messages(&self, input: &Input) -> String {
        let input_markdown = input.render();
        if self.embedded() {
            // a string representing the echoed messages
            self.prompt.replace(INPUT_PLACEHOLDER, &input_markdown)
        } else {
            format!("{}\n\n{}", self.prompt, input.render())
        }
    }

    // this function constructs a messages based on the role's prompt and the provided input
    pub fn build_messages(&self, input: &Input) -> Vec<Message> {
        let mut content = input.to_message_content();

        // handling cases where the prompt is embedded with the input placeholder
        if self.embedded() {
            content.merge_prompt(|v: &str| self.prompt.replace(INPUT_PLACEHOLDER, v));
            // Returning a vector of Message structs
            vec![Message {
                role: MessageRole::User,
                content,
            }]
        } else {
            // Returning a vector of Message structs
            vec![
                Message {
                    role: MessageRole::System,
                    content: MessageContent::Text(self.prompt.clone()),
                },
                Message {
                    role: MessageRole::User,
                    content,
                },
            ]
        }
    }
}

// this function replaces placeholder arguments in the prompt with actual values derived from the provided name
fn complete_prompt_args(prompt: &str, name: &str) -> String {
    let mut prompt = prompt.trim().to_string();
    for (i, arg) in name.split(':').skip(1).enumerate() {
        prompt = prompt.replace(&format!("__ARG{}__", i + 1), arg);
    }
    // returning the modified prompt as a string
    prompt
}

// unit tests for the "complete_prompt_args" function
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_prompt_name() {
        assert_eq!(
            complete_prompt_args("convert __ARG1__", "convert:foo"),
            "convert foo"
        );
        assert_eq!(
            complete_prompt_args("convert __ARG1__ to __ARG2__", "convert:foo:bar"),
            "convert foo to bar"
        );
    }
}
