use super::{ExtraConfig, Model, OpenAIClient, PromptType, SendData, TokensCountFactors};

use crate::{render::ReplyHandler, utils::PromptKind};

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{Client as ReqwestClient, RequestBuilder};
use reqwest_eventsource::{Error as EventSourceError, Event, RequestBuilderExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::env;

// defining the base url
const API_BASE: &str = "https://api.openai.com/v1";

// Array holding all the model names, token count, type of model
const MODELS: [(&str, usize, &str); 7] = [
    ("gpt-3.5-turbo", 4096, "text"),
    ("gpt-3.5-turbo-16k", 16385, "text"),
    ("gpt-3.5-turbo-1106", 16385, "text"),
    ("gpt-4", 8192, "text"),
    ("gpt-4-32k", 32768, "text"),
    ("gpt-4-1106-preview", 128000, "text"),
    ("gpt-4-vision-preview", 128000, "text,vision"),
];

// defining the token count factors
pub const OPENAI_TOKENS_COUNT_FACTORS: TokensCountFactors = (5, 2);

// struct representing the configuration for the openAI client
#[derive(Debug, Clone, Deserialize, Default)]
pub struct OpenAIConfig {
    pub name: Option<String>,
    pub api_key: Option<String>,
    pub organization_id: Option<String>,
    pub extra: Option<ExtraConfig>,
}

// this macro generates the necessary code to make OpenAIClient compatible with the API
openai_compatible_client!(OpenAIClient);

impl OpenAIClient {
    // macro for generating function named get_api_key to retrieve the api key from the configs
    config_get_fn!(api_key, get_api_key);

    // constant defining an array of prompts used for configuration input
    pub const PROMPTS: [PromptType<'static>; 1] =
        [("api_key", "API Key:", true, PromptKind::String)];

    // initializes models based on the variable MODELS and the configuration
    pub fn list_models(local_config: &OpenAIConfig) -> Vec<Model> {
        let client_name = Self::name(local_config);
        MODELS
            .into_iter()
            .map(|(name, max_tokens, capabilities)| {
                // constructing with capabilities, maximum tokens, and token count factors
                Model::new(client_name, name)
                    .set_capabilities(capabilities.into())
                    .set_max_tokens(Some(max_tokens))
                    .set_tokens_count_factors(OPENAI_TOKENS_COUNT_FACTORS)
            })
            .collect()
    }

    // this funciton constructs a request builder for sending requests to the OpenAI api
    fn request_builder(&self, client: &ReqwestClient, data: SendData) -> Result<RequestBuilder> {
        // retrieving the api key from the client configuration
        let api_key = self.get_api_key()?;

        // building the request body
        let body = openai_build_body(data, self.model.name.clone());

        let env_prefix = Self::name(&self.config).to_uppercase();
        // constructing the url for the request based on base url, obtained from the environment variables or a default value
        let api_base = env::var(format!("{env_prefix}_API_BASE"))
            .ok()
            .unwrap_or_else(|| API_BASE.to_string());

        let url = format!("{api_base}/chat/completions");

        debug!("OpenAI Request: {url} {body}");

        // sets up the request with the necessary authentication headers and body
        let mut builder = client.post(url).bearer_auth(api_key).json(&body);

        if let Some(organization_id) = &self.config.organization_id {
            builder = builder.header("OpenAI-Organization", organization_id);
        }

        // returning the builder wrapped in Result
        Ok(builder)
    }
}

// this function sends the request and parses the json into a Value
pub async fn openai_send_message(builder: RequestBuilder) -> Result<String> {
    let data: Value = builder.send().await?.json().await?;
    // checking if there's an error message in the response. If there is, return an error
    if let Some(err_msg) = data["error"]["message"].as_str() {
        bail!("{err_msg}");
    }

    // extracting the message content from the response
    let output = data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow!("Invalid response data: {data}"))?;

    // return it as a string
    Ok(output.to_string())
}

// 
pub async fn openai_send_message_streaming(
    builder: RequestBuilder,
    handler: &mut ReplyHandler,
) -> Result<()> {
    let mut es = builder.eventsource()?;
    // it enters a loop to process events received from the event source
    while let Some(event) = es.next().await {
        match event {
            // if the event is an open event (indicating the start of the stream), it continues to process it
            Ok(Event::Open) => {}
            Ok(Event::Message(message)) => {
                // checking if the message data is "[DONE]"
                if message.data == "[DONE]" {
                    // if yes, break out of the loop
                    break;
                }
                // serialize the message content
                let data: Value = serde_json::from_str(&message.data)?;
                if let Some(text) = data["choices"][0]["delta"]["content"].as_str() {
                    handler.text(text)?;
                }
            }
            // if there is an error, we classify the error and exit using the bail macro
            Err(err) => {
                match err {
                    EventSourceError::InvalidStatusCode(_, res) => {
                        let data: Value = res.json().await?;
                        if let Some(err_msg) = data["error"]["message"].as_str() {
                            bail!("{err_msg}");
                        }
                        bail!("Request failed");
                    }
                    EventSourceError::StreamEnded => {}
                    _ => {
                        bail!("{}", err);
                    }
                }
                // closing the event source
                es.close();
            }
        }
    }

    // returing a Ok(())
    Ok(())
}

// this function constructs the request body for sending messages to OpenAI api
pub fn openai_build_body(data: SendData, model: String) -> Value {
    // destructuring the data object to extract messages, temperature, and stream information
    let SendData {
        messages,
        temperature,
        stream,
    } = data;

    // constructing the body
    let mut body = json!({
        "model": model, // model to be used
        "messages": messages, // vector of messages to be processed
    });

    // The default max_tokens of gpt-4-vision-preview is only 16, we need to make it larger
    if model == "gpt-4-vision-preview" {
        body["max_tokens"] = json!(4096);
    }

    // if the temperature is provided, we add it to the body
    if let Some(v) = temperature {
        body["temperature"] = v.into();
    }
    // if stream is true, we add it to the body
    if stream {
        body["stream"] = true.into();
    }
    // returning the body
    body
}
