use super::openai::{openai_build_body, OPENAI_TOKENS_COUNT_FACTORS};
use super::{ExtraConfig, LocalAIClient, Model, ModelConfig, PromptType, SendData};

use crate::utils::PromptKind;

use anyhow::Result;
use async_trait::async_trait;
use reqwest::{Client as ReqwestClient, RequestBuilder};
use serde::Deserialize;

// struct representing the configuration for a local-ai
#[derive(Debug, Clone, Deserialize)]
pub struct LocalAIConfig {
    pub name: Option<String>,
    pub api_base: String,              // base URL for api endpoints
    pub api_key: Option<String>,       // api key used for authentication
    pub chat_endpoint: Option<String>, // optional endpoint for chat
    pub models: Vec<ModelConfig>, // vector of structs representing different models supported by the local-ai
    pub extra: Option<ExtraConfig>, // Optional extra configurations
}

// macro invocation generates an implementation of the Client trait for LocalAIClient
openai_compatible_client!(LocalAIClient);

// this includes the implementation of functions required by the Client trait
impl LocalAIClient {
    // this macro invocation generates a function named get_api_key for retrieving the api key from the config
    config_get_fn!(api_key, get_api_key);

    // constant array defines prompts for collecting user input
    // each prompt consists of a field path, a description,
    // a flag indicating whether the field is required, and the kind of prompt
    pub const PROMPTS: [PromptType<'static>; 4] = [
        ("api_base", "API Base:", true, PromptKind::String),
        ("api_key", "API Key:", false, PromptKind::String),
        ("models[].name", "Model Name:", true, PromptKind::String),
        (
            "models[].max_tokens",
            "Max Tokens:",
            false,
            PromptKind::Integer,
        ),
    ];

    // this function generates a list of Model instances based on the provided configurations
    pub fn list_models(local_config: &LocalAIConfig) -> Vec<Model> {
        let client_name = Self::name(local_config);

        // this extracts information from the models field of the configurations and constructs Model instances accordingly
        local_config
            .models
            .iter()
            .map(|v| {
                Model::new(client_name, &v.name)
                    .set_capabilities(v.capabilities)
                    .set_max_tokens(v.max_tokens)
                    .set_tokens_count_factors(OPENAI_TOKENS_COUNT_FACTORS)
            })
            .collect()
    }

    // this function constructs a request builder for making api requests to local-AI
    fn request_builder(&self, client: &ReqwestClient, data: SendData) -> Result<RequestBuilder> {
        let api_key = self.get_api_key().ok();

        let body = openai_build_body(data, self.model.name.clone());

        let chat_endpoint = self
            .config
            .chat_endpoint
            .as_deref()
            .unwrap_or("/chat/completions");

        // this retrieves the api key from the configuration and constructs
        // the request body using openai_build_body and prepares the request url
        let url = format!("{}{chat_endpoint}", self.config.api_base);

        debug!("LocalAI Request: {url} {body}");

        let mut builder = client.post(url).json(&body);
        if let Some(api_key) = api_key {
            builder = builder.bearer_auth(api_key);
        }

        // finally, we return a RequestBuilder instance to be used for making the api calls
        Ok(builder)
    }
}
