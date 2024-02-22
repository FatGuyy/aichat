// this file has a modular design where different components such as
// common utilities, message handling, model definitions, and client implementations
// are organized into separate modules for better organization and maintainability
#[macro_use]
mod common;
mod message;
mod model;

pub use common::*;
pub use message::*;
pub use model::*;

register_client!(
    (openai, "openai", OpenAIConfig, OpenAIClient),
    (gemini, "gemini", GeminiConfig, GeminiClient),
    (localai, "localai", LocalAIConfig, LocalAIClient),
    (ollama, "ollama", OllamaConfig, OllamaClient),
    (
        azure_openai,
        "azure-openai",
        AzureOpenAIConfig,
        AzureOpenAIClient
    ),
    (ernie, "ernie", ErnieConfig, ErnieClient),
    (qianwen, "qianwen", QianwenConfig, QianwenClient),
);
