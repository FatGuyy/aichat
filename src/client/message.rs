// This code defines a module responsible for handling messages
use crate::config::Input;

use serde::{Deserialize, Serialize};

// This struct represents a message to be sent to user
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Message {
    pub role: MessageRole, // MessageRole is an enum we use to specify the role of sender
    pub content: MessageContent, // This is the actual contents of the message
}

// This is a basic constructor for the struct - Message
impl Message {
    pub fn new(input: &Input) -> Self {
        Self {
            role: MessageRole::User,
            content: input.to_message_content(),
        }
    }
}

//This is an enum for telling the roles inside a message
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    Assistant,
    User,
}

// utility methods of MessageRole to check if the role is system, assistant, or user
#[allow(dead_code)]
impl MessageRole {
    // returns True of the MessageRole is System
    pub fn is_system(&self) -> bool {
        matches!(self, MessageRole::System)
    }

    // returns True of the MessageRole is User
    pub fn is_user(&self) -> bool {
        matches!(self, MessageRole::User)
    }

    // returns True of the MessageRole is Assitant
    pub fn is_assistant(&self) -> bool {
        matches!(self, MessageRole::Assistant)
    }
}

// This is the Struct which represents the contents inside a message
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String), // The actual response text string
    Array(Vec<MessageContentPart>),
}

// methods for MessageContent to render input (including resolving image URLs) and merge prompts
impl MessageContent {
    pub fn render_input(&self, resolve_url_fn: impl Fn(&str) -> String) -> String {
        match self {
            MessageContent::Text(text) => text.to_string(),
            MessageContent::Array(list) => {
                let (mut concated_text, mut files) = (String::new(), vec![]);
                for item in list {
                    match item {
                        MessageContentPart::Text { text } => {
                            concated_text = format!("{concated_text} {text}")
                        }
                        MessageContentPart::ImageUrl { image_url } => {
                            files.push(resolve_url_fn(&image_url.url))
                        }
                    }
                }
                if !concated_text.is_empty() {
                    concated_text = format!(" -- {concated_text}")
                }
                format!(".file {}{}", files.join(" "), concated_text)
            }
        }
    }

    // Function to merge the whole prompt into one
    pub fn merge_prompt(&mut self, replace_fn: impl Fn(&str) -> String) {
        match self {
            MessageContent::Text(text) => *text = replace_fn(text),
            MessageContent::Array(list) => {
                if list.is_empty() {
                    list.push(MessageContentPart::Text {
                        text: replace_fn(""),
                    })
                } else if let Some(MessageContentPart::Text { text }) = list.get_mut(0) {
                    *text = replace_fn(text)
                }
            }
        }
    }
}

// Enum for a part of Message content
// It can either be text or Image
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

// Struct to represent the url of an Image
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImageUrl {
    pub url: String,
}

// This is a test to check if the message is beign made as we want
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serde() {
        assert_eq!(
            // Make a new Message and deserialize it to a string
            // Then check if it's equal to the expected string
            serde_json::to_string(&Message::new(&Input::from_str("Hello World"))).unwrap(),
            "{\"role\":\"user\",\"content\":\"Hello World\"}"
        );
    }
}
