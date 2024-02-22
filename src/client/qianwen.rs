use super::{message::*, Client, ExtraConfig, Model, PromptType, QianwenClient, SendData};

use crate::{
    render::ReplyHandler,
    utils::{sha256sum, PromptKind},
};

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::StreamExt;
use reqwest::{
    multipart::{Form, Part},
    Client as ReqwestClient, RequestBuilder,
};
use reqwest_eventsource::{Error as EventSourceError, Event, RequestBuilderExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::borrow::BorrowMut;

// the base api url
const API_URL: &str =
    "https://dashscope.aliyuncs.com/api/v1/services/aigc/text-generation/generation";

// api url for multiple models
const API_URL_VL: &str =
    "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation";

// an array containing the name and token size of all the models available
const MODELS: [(&str, usize, &str); 5] = [
    ("qwen-turbo", 8192, "text"),
    ("qwen-plus", 32768, "text"),
    ("qwen-max", 8192, "text"),
    ("qwen-max-longcontext", 30720, "text"),
    ("qwen-vl-plus", 0, "text,vision"),
];

// struct that holds configuration parameters for the Qianwen client
#[derive(Debug, Clone, Deserialize, Default)]
pub struct QianwenConfig {
    pub name: Option<String>,
    pub api_key: Option<String>,
    pub extra: Option<ExtraConfig>,
}

// implementing Client trait for the QianwenClient
#[async_trait]
impl Client for QianwenClient {
    // macro for getting the common client function
    client_common_fns!();

    // This function is responsible for sending a message
    async fn send_message_inner(
        &self,
        client: &ReqwestClient,
        mut data: SendData,
    ) -> Result<String> {
        // retrieving the api key
        let api_key = self.get_api_key()?;
        // patching the messages with the model name and api key
        patch_messages(&self.model.name, &api_key, &mut data.messages).await?;
        // constructing the request builder
        let builder = self.request_builder(client, data)?;
        // sending the message using send_message function
        send_message(builder, self.is_vl()).await
    }
    
    // this funciton is responsible for sending messages in streaming mode
    async fn send_message_streaming_inner(
        &self,
        client: &ReqwestClient,
        handler: &mut ReplyHandler,
        mut data: SendData,
    ) -> Result<()> {
        // retrieving the api key
        let api_key = self.get_api_key()?;
        // patching the messages with the model name and api key
        patch_messages(&self.model.name, &api_key, &mut data.messages).await?;
        // constructing the request builder
        let builder = self.request_builder(client, data)?;
        // sending the message using send_message_streaming function
        send_message_streaming(builder, handler, self.is_vl()).await
    }
}

// 
impl QianwenClient {
    // this generates a get_api_key function for retrieving the api key
    config_get_fn!(api_key, get_api_key);

    // constant array containing one element, which is a tuple representing a prompt 
    pub const PROMPTS: [PromptType<'static>; 1] =
        [("api_key", "API Key:", true, PromptKind::String)];

    pub fn list_models(local_config: &QianwenConfig) -> Vec<Model> {
        // Obtains the client name from the config
        let client_name = Self::name(local_config);
        // iterates over predefined models, creating Model instances with the 
        // client name, model name, capabilities, and maximum tokens
        MODELS
            .into_iter()
            .map(|(name, max_tokens, capabilities)| {
                Model::new(client_name, name)
                    .set_capabilities(capabilities.into())
                    .set_max_tokens(Some(max_tokens))
            })
            .collect()
    }

    // this function constructs a RequestBuilder for making requests to the API
    fn request_builder(&self, client: &ReqwestClient, data: SendData) -> Result<RequestBuilder> {
        // retrieving the api key from the Client
        let api_key = self.get_api_key()?;

        let stream = data.stream;

        let is_vl = self.is_vl();
        // selects the api url based on whether the model is for vision and language
        let url = match is_vl {
            true => API_URL_VL,
            false => API_URL,
        };
        let (body, has_upload) = build_body(data, self.model.name.clone(), is_vl)?;

        // a debug message indicating the request url and body
        debug!("Qianwen Request: {url} {body}");

        // constructing the RequestBuilder with the appropriate method, authentication, and json
        let mut builder = client.post(url).bearer_auth(api_key).json(&body);
        if stream {
            builder = builder.header("X-DashScope-SSE", "enable");
        }
        if has_upload {
            builder = builder.header("X-DashScope-OssResourceResolve", "enable");
        }

        // returning builder wrapped in result
        Ok(builder)
    }

    // This method determines whether the model name of the client 
    // starts with "qwen-vl", indicating it is a vision and language model
    fn is_vl(&self) -> bool {
        self.model.name.starts_with("qwen-vl")
    }
}

// this function handles sending a message with a single response
// it sends the request,
async fn send_message(builder: RequestBuilder, is_vl: bool) -> Result<String> {
    let data: Value = builder.send().await?.json().await?;
    check_error(&data)?;

    // processes the response json, extracts the message content as output
    let output = if is_vl {
        data["output"]["choices"][0]["message"]["content"][0]["text"].as_str()
    } else {
        data["output"]["text"].as_str()
    };

    let output = output.ok_or_else(|| anyhow!("Unexpected response {data}"))?;

    // returning the output as string
    Ok(output.to_string())
}

// this function handles sending a message with streaming responses
async fn send_message_streaming(
    builder: RequestBuilder,
    handler: &mut ReplyHandler,
    is_vl: bool,
) -> Result<()> {
    let mut es = builder.eventsource()?;
    let mut offset = 0;
    
    // it enters a loop to process events received from the event source
    while let Some(event) = es.next().await {
        match event {
            // if the event is an open event (indicating the start of the stream), it continues to process it
            Ok(Event::Open) => {}
            Ok(Event::Message(message)) => {
                // serializing the message data as json
                let data: Value = serde_json::from_str(&message.data)?;
                check_error(&data)?;
                if is_vl {
                    let text =
                        data["output"]["choices"][0]["message"]["content"][0]["text"].as_str();
                    if let Some(text) = text {
                        let text = &text[offset..];
                        handler.text(text)?;
                        offset += text.len();
                    }
                } else if let Some(text) = data["output"]["text"].as_str() {
                    handler.text(text)?;
                }
            }
            // checking for errors
            Err(err) => {
                match err {
                    EventSourceError::StreamEnded => {}
                    _ => {
                        bail!("{}", err);
                    }
                }
                // closing es before closing
                es.close();
            }
        }
    }

    Ok(())
}

// This function is a utility function for seeing the error
fn check_error(data: &Value) -> Result<()> {
    if let (Some(code), Some(message)) = (data["code"].as_str(), data["message"].as_str()) {
        bail!("{code}: {message}");
    }
    // return Ok(())
    Ok(())
}

// this function constructs the request body for sending data to the api
fn build_body(data: SendData, model: String, is_vl: bool) -> Result<(Value, bool)> {
    // Retriving the data from the data variable
    let SendData {
        messages,
        temperature,
        stream,
    } = data;

    let mut has_upload = false;
    // constructing different inputs and parameters object, depending on is_vl
    let (input, parameters) = if is_vl {
        // iterating over each message for constructing json objects representing the message content
        let messages: Vec<Value> = messages
            .into_iter()
            .map(|message| {
                let role = message.role;
                let content = match message.content {
                    MessageContent::Text(text) => vec![json!({"text": text})],
                    MessageContent::Array(list) => list
                        .into_iter()
                        .map(|item| match item {
                            // For text messages, we create an object with a text field, and for image messages
                            MessageContentPart::Text { text } => json!({"text": text}),
                            MessageContentPart::ImageUrl {
                                image_url: ImageUrl { url },
                            } => {
                                if url.starts_with("oss:") {
                                    has_upload = true;
                                }
                                json!({"image": url})
                            }
                        })
                        .collect(),
                };
                json!({ "role": role, "content": content })
            })
            .collect();

        let input = json!({
            "messages": messages,
        });

        let mut parameters = json!({});
        if let Some(v) = temperature {
            parameters["top_k"] = ((v * 50.0).round() as usize).into();
        }
        (input, parameters)
    } else {
        // processing the data for other types of models
        let input = json!({
            // constructing the input json object directly from messages
            "messages": messages,
        });

        let mut parameters = json!({});
        if stream {
            parameters["incremental_output"] = true.into();
        }

        if let Some(v) = temperature {
            parameters["temperature"] = v.into();
        }
        (input, parameters)
    };

    // constructing the overall request json containing the model, input, and parameters
    let body = json!({
        "model": model,
        "input": input,
        "parameters": parameters
    });
    // returning a Result containing the request body and the has_upload
    Ok((body, has_upload))
}

// This function patches the messages to replace embedded image urls with urls pointing to uploaded images
async fn patch_messages(model: &str, api_key: &str, messages: &mut Vec<Message>) -> Result<()> {
    // iterating over each Message in the messages vector
    for message in messages {
        // if message contains an array of MessageContent, it iterates the message
        if let MessageContent::Array(list) = message.content.borrow_mut() {
            for item in list {
                if let MessageContentPart::ImageUrl {
                    image_url: ImageUrl { url },
                } = item
                {
                    // If a part is an ImageUrl and its URL starts with "data:"
                    if url.starts_with("data:") {
                        // uploading the embedded image to an Object Storage Service using the upload function
                        *url = upload(model, api_key, url)
                            .await
                            .with_context(|| "Failed to upload embedded image to oss")?;
                    }
                }
            }
        }
    }
    // returning a Result indicating success or failure
    Ok(())
}

// struct representing a policy received from an api response
#[derive(Debug, Deserialize)]
struct Policy {
    data: PolicyData,
}

// struct representing the data part of a policy received from an api response
#[derive(Debug, Deserialize)]
struct PolicyData {
    policy: String, // string representing the policy
    signature: String, // string representing the signature
    upload_dir: String, // string representing the upload directory
    upload_host: String, // string representing the upload host
    oss_access_key_id: String, // string representing the OSS access key ID
    x_oss_object_acl: String, // string representing the OSS object ACL
    x_oss_forbid_overwrite: String, // string representing whether to forbid overwrite
}

/// Upload image to dashscope
// The function processes the url to extract the mime type and .base64 data of the image
async fn upload(model: &str, api_key: &str, url: &str) -> Result<String> {
    let (mime_type, data) = url
        .strip_prefix("data:")
        .and_then(|v| v.split_once(";base64,"))
        .ok_or_else(|| anyhow!("Invalid image url"))?;
    let mut name = sha256sum(data);
    if let Some(ext) = mime_type.strip_prefix("image/") {
        name.push('.');
        name.push_str(ext);
    }
    let data = STANDARD.decode(data)?;

    let client = reqwest::Client::new();
    let policy: Policy = client
        .get(format!(
            "https://dashscope.aliyuncs.com/api/v1/uploads?action=getPolicy&model={model}"
        ))
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await?
        .json()
        .await?;
    let PolicyData {
        policy,
        signature,
        upload_dir,
        upload_host,
        oss_access_key_id,
        x_oss_object_acl,
        x_oss_forbid_overwrite,
        ..
    } = policy.data;

    let key = format!("{upload_dir}/{name}");
    let file = Part::bytes(data).file_name(name).mime_str(mime_type)?;
    let form = Form::new()
        .text("OSSAccessKeyId", oss_access_key_id)
        .text("Signature", signature)
        .text("policy", policy)
        .text("key", key.clone())
        .text("x-oss-object-acl", x_oss_object_acl)
        .text("x-oss-forbid-overwrite", x_oss_forbid_overwrite)
        .text("success_action_status", "200")
        .text("x-oss-content-type", mime_type.to_string())
        .part("file", file);

    let res = client.post(upload_host).multipart(form).send().await?;

    let status = res.status();
    if res.status() != 200 {
        let text = res.text().await?;
        bail!("{status}, {text}")
    }
    Ok(format!("oss://{key}"))
}
