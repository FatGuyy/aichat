use super::{patch_system_message, Client, ErnieClient, ExtraConfig, Model, PromptType, SendData};

use crate::{render::ReplyHandler, utils::PromptKind};

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{Client as ReqwestClient, RequestBuilder};
use reqwest_eventsource::{Error as EventSourceError, Event, RequestBuilderExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::env;

const API_BASE: &str = "https://aip.baidubce.com/rpc/2.0/ai_custom/v1"; // base URL for API requests to the Baidu AI platform
const ACCESS_TOKEN_URL: &str = "https://aip.baidubce.com/oauth/2.0/token"; // URL for obtaining an access token

// array of tuples mapping model names to their endpoints
const MODELS: [(&str, &str); 4] = [
    ("ernie-bot", "/wenxinworkshop/chat/completions"),
    ("ernie-bot-4", "/wenxinworkshop/chat/completions_pro"),
    ("ernie-bot-8k", "/wenxinworkshop/chat/ernie_bot_8k"),
    ("ernie-bot-turbo", "/wenxinworkshop/chat/eb-instant"),
];

// static mutable string used to store the access token
static mut ACCESS_TOKEN: String = String::new(); // safe under linear operation

// struct for represents the configuration options for ErnieClient
// all of the variables are optional in this struct
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ErnieConfig {
    pub name: Option<String>,
    pub api_key: Option<String>,
    pub secret_key: Option<String>,
    pub extra: Option<ExtraConfig>,
}

// trait implementation defines methods required by the Client trait
#[async_trait]
impl Client for ErnieClient {
    client_common_fns!();

    async fn send_message_inner(&self, client: &ReqwestClient, data: SendData) -> Result<String> {
        // function to ensure that an access token is available before making requests
        self.prepare_access_token().await?;
        // we call the 'request_builder' function to construct the request
        let builder = self.request_builder(client, data)?;
        // we send it using the following funciton
        send_message(builder).await
    }

    // This is bascially the above function but it calls the streaming type of function to send message
    async fn send_message_streaming_inner(
        &self,
        client: &ReqwestClient,
        handler: &mut ReplyHandler,
        data: SendData,
    ) -> Result<()> {
        self.prepare_access_token().await?;
        let builder = self.request_builder(client, data)?;
        send_message_streaming(builder, handler).await
    }
}

// this contains additional functions and implementations for ErnieClient 
impl ErnieClient {
    // this prompts for configuring API key and secret key
    pub const PROMPTS: [PromptType<'static>; 2] = [
        ("api_key", "API Key:", true, PromptKind::String),
        ("secret_key", "Secret Key:", true, PromptKind::String),
    ];

    // this function constructs a list of supported models based on the provided configurations
    pub fn list_models(local_config: &ErnieConfig) -> Vec<Model> {
        let client_name = Self::name(local_config);
        MODELS
            .into_iter()
            .map(|(name, _)| Model::new(client_name, name))
            .collect()
    }

    // function constructs an request builder with 
    // necessary parameters for sending messages to ernie API
    fn request_builder(&self, client: &ReqwestClient, data: SendData) -> Result<RequestBuilder> {
        let body = build_body(data, self.model.name.clone());
        
        let model = self.model.name.clone();
        let (_, chat_endpoint) = MODELS
        .iter()
        .find(|(v, _)| v == &model)
        .ok_or_else(|| anyhow!("Miss Model '{}'", self.model.id()))?;
    
        // constructs the URL using the base URL, model endpoint, and access token 
        let url = format!("{API_BASE}{chat_endpoint}?access_token={}", unsafe {
            &ACCESS_TOKEN
        });

        debug!("Ernie Request: {url} {body}");

        let builder = client.post(url).json(&body);

        Ok(builder)
    }

    // this function ensures that an access token is available for making requests
    // If the access token is empty, it fetches the api and secret key
    // from the configuration or environment variables
    async fn prepare_access_token(&self) -> Result<()> {
        if unsafe { ACCESS_TOKEN.is_empty() } {
            // Note: cannot use config_get_fn!
            let env_prefix = Self::name(&self.config).to_uppercase();
            let api_key = self.config.api_key.clone();
            let api_key = api_key
                .or_else(|| env::var(format!("{env_prefix}_API_KEY")).ok())
                .ok_or_else(|| anyhow!("Miss api_key"))?;

            let secret_key = self.config.secret_key.clone();
            let secret_key = secret_key
                .or_else(|| env::var(format!("{env_prefix}_SECRET_KEY")).ok())
                .ok_or_else(|| anyhow!("Miss secret_key"))?;

            let token = fetch_access_token(&api_key, &secret_key)
                .await
                .with_context(|| "Failed to fetch access token")?;
            unsafe { ACCESS_TOKEN = token };
        }
        Ok(())
    }
}

// this function sends a message using RequestBuilder
async fn send_message(builder: RequestBuilder) -> Result<String> {
    // the request is sent asynchronously and wait for response
    // response is parsed as json
    let data: Value = builder.send().await?.json().await?;
    check_error(&data)?;

    // here, we extract the result from the json data
    let output = data["result"]
        .as_str()
        .ok_or_else(|| anyhow!("Unexpected response {data}"))?;

    // returning the extracted output as string
    Ok(output.to_string())
}

// this function sends a message in streaming mode using RequestBuilder and ReplyHandler
// this function does the same thing as above but in a stream fashion 
async fn send_message_streaming(builder: RequestBuilder, handler: &mut ReplyHandler) -> Result<()> {
    // establishing a connection to server and listening for incoming messages
    let mut es = builder.eventsource()?;
    while let Some(event) = es.next().await {
        match event {
            // when a message is received, we parse the json and extracts the result field
            Ok(Event::Open) => {}
            Ok(Event::Message(message)) => {
                // extracting the result string
                let data: Value = serde_json::from_str(&message.data)?;
                if let Some(text) = data["result"].as_str() {
                    // If successful, we send the extracted text to ReplyHandler
                    handler.text(text)?;
                }
            }
            // handling different types of errors, as invalid content type, stream ending, or general errors
            Err(err) => {
                match err {
                    EventSourceError::InvalidContentType(header_value, res) => {
                        let content_type = header_value
                            .to_str()
                            .map_err(|_| anyhow!("Invalid response header"))?;
                        if content_type.contains("application/json") {
                            let data: Value = res.json().await?;
                            check_error(&data)?;
                            bail!("Request failed");
                        } else {
                            let text = res.text().await?;
                            if let Some(text) = text.strip_prefix("data: ") {
                                let data: Value = serde_json::from_str(text)?;
                                if let Some(text) = data["result"].as_str() {
                                    handler.text(text)?;
                                }
                            } else {
                                // if any errors occur during the process, we returns an error wrapped in a Result
                                bail!("Invalid response data: {text}")
                            }
                        }
                    }
                    EventSourceError::StreamEnded => {}
                    _ => {
                        // if any errors occur during the process, we returns an error wrapped in a Result
                        bail!("{}", err);
                    }
                }
                // we close the builder eventsource before ending the funciton 
                es.close();
            }
        }
    }

    Ok(())
}

// function to check the errors in the response data
// it inspects the json data for error messages and error codes
fn check_error(data: &Value) -> Result<()> {
    if let Some(err_msg) = data["error_msg"].as_str() {
        if let Some(code) = data["error_code"].as_number().and_then(|v| v.as_u64()) {
            if code == 110 {
                // if an error is detected, it returns an error in a Result
                unsafe { ACCESS_TOKEN = String::new() }
            }
            bail!("{err_msg}. err_code: {code}");
        } else {
            bail!("{err_msg}");
        }
    }
    // if no errors are found, we return Ok(())
    Ok(())
}

// this function constructs the body of the request using the provided data
fn build_body(data: SendData, _model: String) -> Value {
    // we prepare the message data, adjust temperature if provided, and set the streaming flag if required
    let SendData {
        mut messages,
        temperature,
        stream,
    } = data;

    patch_system_message(&mut messages);

    // we make the body as a json
    let mut body = json!({
        "messages": messages,
    });

    // checking the temperature and stream boolean
    if let Some(temperature) = temperature {
        body["temperature"] = (temperature / 2.0).into();
    }
    if stream {
        body["stream"] = true.into();
    }

    // we return the constructed JSON body as a Value
    body
}

// function for fetching the access token from the baidu api
async fn fetch_access_token(api_key: &str, secret_key: &str) -> Result<String> {
    // we construct the URL with the provided api and secret key
    let url = format!("{ACCESS_TOKEN_URL}?grant_type=client_credentials&client_id={api_key}&client_secret={secret_key}");
    // we send a request to the baidu api endpoint, wait for the response
    let value: Value = reqwest::get(&url).await?.json().await?;
    // we parse the recieved json data to extract the access token from the response
    let result = value["access_token"].as_str().ok_or_else(|| {
        if let Some(err_msg) = value["error_description"].as_str() {
            anyhow!("{err_msg}")
        } else {
            anyhow!("Invalid response data")
        }
    })?;
    // if successful, we return the access token as a string in a Result
    Ok(result.to_string())
}
