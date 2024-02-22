use super::message::{Message, MessageContent};

use crate::utils::count_tokens;

use anyhow::{bail, Result};
use serde::{Deserialize, Deserializer};

pub type TokensCountFactors = (usize, usize); // (per-messages, bias)

// this struct represents a llm
#[derive(Debug, Clone)]
pub struct Model {
    pub client_name: String,                      // name of the client
    pub name: String,                             // name of model
    pub max_tokens: Option<usize>, // maximum number of tokens allowed for text generation
    pub tokens_count_factors: TokensCountFactors, // factors affecting token count, such as tokens per message
    pub capabilities: ModelCapabilities,          // enum indicating the capabilities of the model
}

// defalult implementations for model
impl Default for Model {
    fn default() -> Self {
        Model::new("", "")
    }
}

impl Model {
    // Constructor function for Model instance
    pub fn new(client_name: &str, name: &str) -> Self {
        Self {
            client_name: client_name.into(),
            name: name.into(),
            max_tokens: None,
            tokens_count_factors: Default::default(),
            capabilities: ModelCapabilities::Text,
        }
    }

    // this function finds a model from a list of models based on a given configurations
    pub fn find(models: &[Self], value: &str) -> Option<Self> {
        let mut model = None;
        let (client_name, model_name) = match value.split_once(':') {
            Some((client_name, model_name)) => {
                if model_name.is_empty() {
                    (client_name, None)
                } else {
                    (client_name, Some(model_name))
                }
            }
            None => (value, None),
        };
        match model_name {
            Some(model_name) => {
                if let Some(found) = models.iter().find(|v| v.id() == value) {
                    model = Some(found.clone());
                } else if let Some(found) = models.iter().find(|v| v.client_name == client_name) {
                    let mut found = found.clone();
                    found.name = model_name.to_string();
                    model = Some(found)
                }
            }
            None => {
                if let Some(found) = models.iter().find(|v| v.client_name == client_name) {
                    model = Some(found.clone());
                }
            }
        }
        model
    }

    // this function generates an identifier string for model and returns self
    pub fn id(&self) -> String {
        format!("{}:{}", self.client_name, self.name)
    }

    // this function sets the capabilities of model and returns self
    pub fn set_capabilities(mut self, capabilities: ModelCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    // this function sets the maximum number of tokens for model and returns self
    pub fn set_max_tokens(mut self, max_tokens: Option<usize>) -> Self {
        match max_tokens {
            None | Some(0) => self.max_tokens = None,
            _ => self.max_tokens = max_tokens,
        }
        self
    }

    // this function sets the factors affecting token count for model and returns self
    pub fn set_tokens_count_factors(mut self, tokens_count_factors: TokensCountFactors) -> Self {
        self.tokens_count_factors = tokens_count_factors;
        self
    }

    // this function calculates the total number of tokens in the given message
    pub fn messages_tokens(&self, messages: &[Message]) -> usize {
        messages
            .iter()
            .map(|v| {
                match &v.content {
                    MessageContent::Text(text) => count_tokens(text),
                    MessageContent::Array(_) => 0, // TODO
                }
            })
            .sum()
    }

    // this function calculates the total number of tokens considering messages and token count factors
    pub fn total_tokens(&self, messages: &[Message]) -> usize {
        if messages.is_empty() {
            return 0;
        }
        let num_messages = messages.len();
        let message_tokens = self.messages_tokens(messages);
        let (per_messages, _) = self.tokens_count_factors;
        if messages[num_messages - 1].role.is_user() {
            num_messages * per_messages + message_tokens
        } else {
            (num_messages - 1) * per_messages + message_tokens
        }
    }

    // this funciton checks if the total tokens exceed the maximum token limit
    pub fn max_tokens_limit(&self, messages: &[Message]) -> Result<()> {
        let (_, bias) = self.tokens_count_factors;
        let total_tokens = self.total_tokens(messages) + bias;
        if let Some(max_tokens) = self.max_tokens {
            if total_tokens >= max_tokens {
                bail!("Exceed max tokens limit")
            }
        }
        Ok(())
    }
}

// this struct represents the configuration for the model
#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    pub name: String,              // name of the model
    pub max_tokens: Option<usize>, // maximum number of tokens allowed per generations
    #[serde(deserialize_with = "deserialize_capabilities")]
    #[serde(default = "default_capabilities")]
    pub capabilities: ModelCapabilities, // the capabilities of model
}

// bitflags enum representing the capabilities of a model
bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct ModelCapabilities: u32 {
        const Text = 0b00000001;
        const Vision = 0b00000010;
    }
}

// implementation for converting a string slice into ModelCapabilities
impl From<&str> for ModelCapabilities {
    fn from(value: &str) -> Self {
        let value = if value.is_empty() { "text" } else { value };
        let mut output = ModelCapabilities::empty();
        if value.contains("text") {
            output |= ModelCapabilities::Text;
        }
        if value.contains("vision") {
            output |= ModelCapabilities::Vision;
        }
        output
    }
}

// this function deserializes model capabilities from a string
fn deserialize_capabilities<'de, D>(deserializer: D) -> Result<ModelCapabilities, D::Error>
where
    D: Deserializer<'de>,
{
    let value: String = Deserialize::deserialize(deserializer)?;
    Ok(value.as_str().into())
}

// this function provides a default value for model capabilities
fn default_capabilities() -> ModelCapabilities {
    ModelCapabilities::Text
}
