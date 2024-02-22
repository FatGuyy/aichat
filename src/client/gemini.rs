use super::{
    message::*, patch_system_message, Client, ExtraConfig, GeminiClient, Model, PromptType,
    SendData, TokensCountFactors,
};

use crate::{render::ReplyHandler, utils::PromptKind};

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{Client as ReqwestClient, RequestBuilder};
use serde::Deserialize;
use serde_json::{json, Value};

// The base api url
const API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models/";

// array of all the available models
const MODELS: [(&str, usize, &str); 3] = [
    ("gemini-pro", 32768, "text"),
    ("gemini-pro-vision", 16384, "vision"),
    ("gemini-ultra", 32768, "text"),
];

const TOKENS_COUNT_FACTORS: TokensCountFactors = (5, 2);

// a struct to hold configuration for the gemini client
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GeminiConfig {
    pub name: Option<String>, // name of the model
    pub api_key: Option<String>, // the api key
    pub extra: Option<ExtraConfig>, // extra configurations
}

// implementaion of client trait for gemini client
#[async_trait]
impl Client for GeminiClient {
    client_common_fns!();

    // this function sends a message using the provided ReqwestClient and SendData
    async fn send_message_inner(&self, client: &ReqwestClient, data: SendData) -> Result<String> {
        // making a builder with request_builder funciton 
        let builder = self.request_builder(client, data)?;
        send_message(builder).await
    }

    // this function does the same thing as above one but does in a streaming way
    async fn send_message_streaming_inner(
        &self,
        client: &ReqwestClient,
        handler: &mut ReplyHandler,
        data: SendData,
    ) -> Result<()> {
        // making a builder with request_builder funciton 
        let builder = self.request_builder(client, data)?;
        send_message_streaming(builder, handler).await
    }
}

// This defines the GeminiClient struct and all its functions
impl GeminiClient {
    // we macro invoke config_get_fn! to generate a function for retrieving api key
    config_get_fn!(api_key, get_api_key);

    // array of prompts for obtaining api key
    pub const PROMPTS: [PromptType<'static>; 1] =
        [("api_key", "API Key:", true, PromptKind::String)];

    // function constructs and returns a vector of models based on the given configurations
    pub fn list_models(local_config: &GeminiConfig) -> Vec<Model> {
        let client_name = Self::name(local_config);
        MODELS
            .into_iter()
            .map(|(name, max_tokens, capabilities)| {
                Model::new(client_name, name)
                    .set_capabilities(capabilities.into())
                    .set_max_tokens(Some(max_tokens))
                    .set_tokens_count_factors(TOKENS_COUNT_FACTORS)
            })
            .collect()
    }

    // function constructs and returns a RequestBuilder for making requests to the Gemini api
    fn request_builder(&self, client: &ReqwestClient, data: SendData) -> Result<RequestBuilder> {
        // extracting the api key from the configuration
        let api_key = self.get_api_key()?;

        // determining the api endpoint function based on whether streaming is enabled or not
        let func = match data.stream {
            true => "streamGenerateContent",
            false => "generateContent",
        };

        let body = build_body(data, self.model.name.clone())?;

        let model = self.model.name.clone();

        // constructing the url and request body based on model name, api key
        let url = format!("{API_BASE}{}:{}?key={}", model, func, api_key);

        debug!("Gemini Request: {url} {body}");

        let builder = client.post(url).json(&body);

        Ok(builder)
    }
}

// function is used to construct an HTTP request for sending message
async fn send_message(builder: RequestBuilder) -> Result<String> {
    let res = builder.send().await?;
    let status = res.status();
    let data: Value = res.json().await?;
    // checking the http status code, if it's not 200, indicating an error, 
    // we parse the json response and check for any error messages
    if status != 200 {
        check_error(&data)?;
    }
    // if response is successful, we extract the content of the 
    // first candidate from the json and return it as a string
    let output = data["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .ok_or_else(|| anyhow!("Invalid response data: {data}"))?;
    Ok(output.to_string())
}

// function is similar to send_message but for handling streaming mode
// we continue reading and processing data until the stream ends
async fn send_message_streaming(builder: RequestBuilder, handler: &mut ReplyHandler) -> Result<()> {
    // sending the request
    let res = builder.send().await?;
    // response status code is not 200, we parse the json response and checks for any error messages
    if res.status() != 200 {
        let data: Value = res.json().await?;
        check_error(&data)?;
    }
    // if response is successful 
    else {
        // buffer to accumulate the received data
        let mut buffer = vec![];
        let mut cursor = 0;
        let mut start = 0;
        let mut balances = vec![];
        let mut quoting = false;
        let mut stream = res.bytes_stream();
        // enters a loop to process the streaming data
        while let Some(chunk) = stream.next().await {
            // reading chunks of data from the response stream
            let chunk = chunk?;
            // processing the chunks as utf8 string
            let chunk = std::str::from_utf8(&chunk)?;
            buffer.extend(chunk.chars());
            for i in cursor..buffer.len() {
                let ch = buffer[i];
                if quoting {
                    if ch == '"' && buffer[i - 1] != '\\' {
                        quoting = false;
                    }
                    continue;
                }
                match ch {
                    '"' => quoting = true,
                    '{' => {
                        if balances.is_empty() {
                            start = i;
                        }
                        balances.push(ch);
                    }
                    '[' => {
                        if start != 0 {
                            balances.push(ch);
                        }
                    }
                    '}' => {
                        balances.pop();
                        if balances.is_empty() {
                            let value: String = buffer[start..=i].iter().collect();
                            let value: Value = serde_json::from_str(&value)?;
                            if let Some(text) =
                                value["candidates"][0]["content"]["parts"][0]["text"].as_str()
                            {
                                handler.text(text)?;
                            } else {
                                bail!("Invalid response data: {value}")
                            }
                        }
                    }
                    ']' => {
                        balances.pop();
                    }
                    _ => {}
                }
            }
            cursor = buffer.len();
        }
    }
    // after successful completion, we return Ok(())
    Ok(())
}

// this function attempts to extract error information from the json object by checking if it contains an error
fn check_error(data: &Value) -> Result<()> {
    // the error field is present and contains both "status" and "message" fields, 
    // it extracts their values and formats an error message using these values
    if let Some((Some(status), Some(message))) = data[0]["error"].as_object().map(|v| {
        (
            v.get("status").and_then(|v| v.as_str()),
            v.get("message").and_then(|v| v.as_str()),
        )
    }) {
        bail!("{status}: {message}")
    } else {
        // if the error field is not properly structured or missing, 
        // we raise a generic error with the entire json object as context
        bail!("Error {}", data);
    }
}

// this function constructs the request body to be sent in an api request
fn build_body(data: SendData, _model: String) -> Result<Value> {
    // first extracts the messages from the SendData object and 
    // prepares them for inclusion in the request body
    let SendData {
        mut messages,
        temperature,
        ..
    } = data;

    patch_system_message(&mut messages);

    let mut network_image_urls = vec![];
    // for each message, we check content type (text or array) and constructs the json accordingly
    let contents: Vec<Value> = messages
        .into_iter()
        .map(|message| {
            let role = match message.role {
                MessageRole::User => "user",
                _ => "model",
            };
            match message.content {
                MessageContent::Text(text) => json!({
                    "role": role,
                    "parts": [{ "text": text }]
                }),
                MessageContent::Array(list) => {
                    let list: Vec<Value> = list
                        .into_iter()
                        .map(|item| match item {
                            MessageContentPart::Text { text } => json!({"text": text}),
                            // if the message has an image url, we distinguish between network images and inline data images
                            MessageContentPart::ImageUrl { image_url: ImageUrl { url } } => {
                                if let Some((mime_type, data)) = url.strip_prefix("data:").and_then(|v| v.split_once(";base64,")) {
                                    json!({ "inline_data": { "mime_type": mime_type, "data": data } })
                                } else {
                                    network_image_urls.push(url.clone());
                                    json!({ "url": url })
                                }
                            },
                        })
                        .collect();
                    json!({ "role": role, "parts": list })
                }
            }
        })
        .collect();

    // if network images are detected, we raise an error
    if !network_image_urls.is_empty() {
        bail!(
            "The model does not support network images: {:?}",
            network_image_urls
        );
    }

    // finally, we construct the main body for the request
    let mut body = json!({
        "contents": contents,
    });

    if let Some(temperature) = temperature {
        body["generationConfig"] = json!({
            "temperature": temperature,
        });
    }

    Ok(body)
}
