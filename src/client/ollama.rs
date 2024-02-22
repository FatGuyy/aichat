use super::{
    message::*, patch_system_message, Client, ExtraConfig, Model, ModelConfig, OllamaClient,
    PromptType, SendData, TokensCountFactors,
};

use crate::{render::ReplyHandler, utils::PromptKind};

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{Client as ReqwestClient, RequestBuilder};
use serde::Deserialize;
use serde_json::{json, Value};

// represents factors affecting token count
const TOKENS_COUNT_FACTORS: TokensCountFactors = (5, 2);

// struct representing the configuration for the Ollama client
#[derive(Debug, Clone, Deserialize, Default)]
pub struct OllamaConfig {
    pub name: Option<String>, // name of model
    pub api_base: String, // base url for the Ollama api
    pub api_key: Option<String>, // api key for Ollama 
    pub chat_endpoint: Option<String>, // endpoint for chat operations
    pub models: Vec<ModelConfig>, // configurations for different models
    pub extra: Option<ExtraConfig>, // extra and optional configurations
}

// Client trait is implemented for the Ollama client struct
#[async_trait]
impl Client for OllamaClient {
    client_common_fns!();

    // this function sends a message using the provided Reqwest client and message
    async fn send_message_inner(&self, client: &ReqwestClient, data: SendData) -> Result<String> {
        let builder = self.request_builder(client, data)?;
        send_message(builder).await
    }

    // this function streams messages using the provided Reqwest client, reply handler and message
    async fn send_message_streaming_inner(
        &self,
        client: &ReqwestClient,
        handler: &mut ReplyHandler,
        data: SendData,
    ) -> Result<()> {
        let builder = self.request_builder(client, data)?;
        send_message_streaming(builder, handler).await
    }
}

impl OllamaClient {
    // this function generates a function named get_api_key that retrieves the api key from the configurations
    config_get_fn!(api_key, get_api_key);

    // This array contains prompts for configuration values
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

    // 
    pub fn list_models(local_config: &OllamaConfig) -> Vec<Model> {
        // obtaining the client name from the config
        let client_name = Self::name(local_config);

        // iterate over the models defined in the config, and map them to Model object
        // for each new Model, we create a new Model instance
        local_config
            .models
            .iter()
            .map(|v| {
                Model::new(client_name, &v.name)
                    .set_capabilities(v.capabilities)
                    .set_max_tokens(v.max_tokens)
                    .set_tokens_count_factors(TOKENS_COUNT_FACTORS)
            })
            .collect()
    }

    //  this function constructs a request builder for sending requests to the Ollama api
    fn request_builder(&self, client: &ReqwestClient, data: SendData) -> Result<RequestBuilder> {
        // retrieving the API key from the client's configuration
        let api_key = self.get_api_key().ok();

        // constructing the request body 
        let body = build_body(data, self.model.name.clone())?;

        let chat_endpoint = self.config.chat_endpoint.as_deref().unwrap_or("/api/chat");

        let url = format!("{}{chat_endpoint}", self.config.api_base);

        // logging the constructed request url and body
        debug!("Ollama Request: {url} {body}");

        // creates a POST request using the request builder
        let mut builder = client.post(url).json(&body);
        if let Some(api_key) = api_key {
            builder = builder.header("Authorization", api_key)
        }

        // returns RequestBuilder wrapped in a Result
        Ok(builder)
    }
}

// for sending to the client 
async fn send_message(builder: RequestBuilder) -> Result<String> {
    // sends the request using the send method of builder
    let res = builder.send().await?;
    // retrieving the HTTP status code
    let status = res.status();
    if status != 200 {
        // reading the response body as text to extract the error message
        let text = res.text().await?;
        bail!("{status}, {text}");
    }
    // parsing the response body as json into a Value object
    let data: Value = res.json().await?;
    // extracting the content of the message
    let output = data["message"]["content"]
    .as_str()
    .ok_or_else(|| anyhow!("Invalid response data: {data}"))?;
Ok(output.to_string())
}

// similar to above function but is intended for streaming responses
async fn send_message_streaming(builder: RequestBuilder, handler: &mut ReplyHandler) -> Result<()> {
    // sends the request using the send method of builder
    let res = builder.send().await?;
    // retrieving the HTTP status code
    let status = res.status();
    if status != 200 {
        // reading the response body as text to extract the error message
        let text = res.text().await?;
        bail!("{status}, {text}");
    } else {
        // initializing a byte stream from the response
        let mut stream = res.bytes_stream();
        // iterating over the stream, processing each chunk asynchronously
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            // For each chunk, we deserialize the json data into a Value object
            let data: Value = serde_json::from_slice(&chunk)?;
            if data["done"].is_boolean() {
                if let Some(text) = data["message"]["content"].as_str() {
                    handler.text(text)?;
                }
            } else {
                bail!("Invalid response data: {data}")
            }
        }
    }
    Ok(())
}

// This function constructs the json body for the request based on the provided data and model
fn build_body(data: SendData, model: String) -> Result<Value> {
    // destructuring the data object to extract messages, temperature, and stream information
    let SendData {
        mut messages,
        temperature,
        stream,
    } = data;

    patch_system_message(&mut messages);
    
    // initializing vector to store network image urls
    let mut network_image_urls = vec![];
    // constructing the json representation of each message
    let messages: Vec<Value> = messages
        .into_iter()
        .map(|message| {
            let role = message.role;
            match message.content {
                // constructing the json body object containing the model name, messages and stream information
                MessageContent::Text(text) => json!({
                    "role": role,
                    "content": text,
                }),
                MessageContent::Array(list) => {
                    let mut content = vec![];
                    let mut images = vec![];
                    for item in list {
                        match item {
                            MessageContentPart::Text { text } => {
                                content.push(text);
                            }
                            MessageContentPart::ImageUrl {
                                image_url: ImageUrl { url },
                            } => {
                                if let Some((_, data)) = url
                                    .strip_prefix("data:")
                                    .and_then(|v| v.split_once(";base64,"))
                                {
                                    images.push(data.to_string());
                                } else {
                                    network_image_urls.push(url.clone());
                                }
                            }
                        }
                    }
                    let content = content.join("\n\n");
                    json!({ "role": role, "content": content, "images": images })
                }
            }
        })
        .collect();

    if !network_image_urls.is_empty() {
        bail!(
            "The model does not support network images: {:?}",
            network_image_urls
        );
    }

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": stream,
    });

    // If temperature value is provided, we add an options field to the body json object
    if let Some(temperature) = temperature {
        body["options"] = json!({
            "temperature": temperature,
        });
    }

    // returning the constructed json wrapped in a Result
    Ok(body)
}
