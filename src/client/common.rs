// This file contains all the common utility functions to be used inside other files
// for managing client configurations, sending messages, and handling configurations
use super::{openai::OpenAIConfig, ClientConfig, Message, MessageContent, Model};

use crate::{
    config::{GlobalConfig, Input},
    render::ReplyHandler,
    utils::{
        init_tokio_runtime, prompt_input_integer, prompt_input_string, tokenize, AbortSignal,
        PromptKind,
    },
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::{Client as ReqwestClient, ClientBuilder, Proxy, RequestBuilder};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{env, future::Future, time::Duration};
use tokio::time::sleep;

// a macro for registering client configurations
#[macro_export]
macro_rules! register_client {
    (
        // for each tuple in input, it creates a module with the name specified in the tuple's first element
        // and imports the configuration struct specified in the tuple's third element
        $(($module:ident, $name:literal, $config:ident, $client:ident),)+
    ) => {
        $(
            mod $module;
        )+
        $(
            use self::$module::$config;
        )+

        // an enum for representing different client configurations
        // each variantation of this enum corresponds to a different client config
        #[derive(Debug, Clone, serde::Deserialize)]
        #[serde(tag = "type")] // this 'tag', enables polymorphic deserialization based on the type field
        pub enum ClientConfig {
            $(
                #[serde(rename = $name)]
                $config($config),
            )+
            #[serde(other)]
            Unknown,
        }


        $(
            #[derive(Debug)]
            pub struct $client {
                global_config: $crate::config::GlobalConfig,
                config: $config,
                model: $crate::client::Model,
            }

            impl $client {
                pub const NAME: &'static str = $name;

                // constructor for client, based on the global configurations
                pub fn init(global_config: &$crate::config::GlobalConfig) -> Option<Box<dyn Client>> {
                    let model = global_config.read().model.clone();
                    // iterates over client configs in the global configs 
                    // and tries to find a matching client based on the model's name
                    let config = global_config.read().clients.iter().find_map(|client_config| {
                        if let ClientConfig::$config(c) = client_config {
                            if Self::name(c) == &model.client_name {
                                return Some(c.clone())
                            }
                        }
                        None
                    })?;
                    
                    Some(Box::new(Self {
                        global_config: global_config.clone(),
                        config,
                        model,
                    }))
                }
                
                pub fn name(config: &$config) -> &str {
                    config.name.as_deref().unwrap_or(Self::NAME)
                }
            }
            
        )+
        
        // initializes a client instance based on the global configurations
        pub fn init_client(config: &$crate::config::GlobalConfig) -> anyhow::Result<Box<dyn Client>> {
            None
            $(.or_else(|| $client::init(config)))+
            .ok_or_else(|| {
                let model = config.read().model.clone();
                anyhow::anyhow!("Unknown client '{}'", &model.client_name)
            })
        }

        // This is function to ensure that the current model has the required capabilities of the user
        pub fn ensure_model_capabilities(client: &mut dyn Client, capabilities: $crate::client::ModelCapabilities) -> anyhow::Result<()> {
            if !client.model().capabilities.contains(capabilities) {
                let models = client.models();
                if let Some(model) = models.into_iter().find(|v| v.capabilities.contains(capabilities)) {
                    client.set_model(model);
                } else {
                    anyhow::bail!(
                        "The current model lacks the corresponding capability."
                    );
                }
            }
            Ok(())
        }

        // utility functions for listing client types
        pub fn list_client_types() -> Vec<&'static str> {
            vec![$($client::NAME,)+]
        }
        
        // utility functions for creating client configurations
        pub fn create_client_config(client: &str) -> anyhow::Result<serde_json::Value> {
            $(
                if client == $client::NAME {
                    return create_config(&$client::PROMPTS, $client::NAME)
                }
            )+
            anyhow::bail!("Unknown client {}", client)
        }
        
        // utility functions for listing available models
        pub fn list_models(config: &$crate::config::Config) -> Vec<$crate::client::Model> {
            config
                .clients
                .iter()
                .flat_map(|v| match v {
                    $(ClientConfig::$config(c) => $client::list_models(c),)+
                    ClientConfig::Unknown => vec![],
                })
                .collect()
        }

    };
}

// macro for defining common client functions
// it takes no input
#[macro_export]
macro_rules! client_common_fns {
    // expands to a set of function, common to few client implementation
    () => {
        // method for returning a tuple containing references to the global configuration
        fn config(
            &self,
        ) -> (
            &$crate::config::GlobalConfig,
            &Option<$crate::client::ExtraConfig>,
        ) {
            (&self.global_config, &self.config.extra)
        }

        // method for returning a vector of models of the client configuration
        fn models(&self) -> Vec<Model> {
            Self::list_models(&self.config)
        }

        // method that returns a reference to the current model
        fn model(&self) -> &Model {
            &self.model
        }

        // method for setting a model for the client
        fn set_model(&mut self, model: Model) {
            self.model = model;
        }
    };
}

// macro for defining clients compatible with openAI
// it takes 'client' as input, which is the name of the client
#[macro_export]
macro_rules! openai_compatible_client {
    // expands to an implementation of the client trait
    ($client:ident) => {
        #[async_trait] // tells us that this is an asynchronous trait
        impl $crate::client::Client for $crate::client::$client {
            client_common_fns!(); // for including common client functions

            // this is an asynchronous method is responsible for sending a message
            async fn send_message_inner(
                &self,
                client: &reqwest::Client,
                data: $crate::client::SendData,
            ) -> anyhow::Result<String> {
                // making a request builder
                let builder = self.request_builder(client, data)?;
                // calling 'openai_send_message' from the openai module, using the request builder, and await
                $crate::client::openai::openai_send_message(builder).await
            }
            
            // this is an asynchronous method is responsible for sending a message in a streaming fashion
            async fn send_message_streaming_inner(
                &self,
                client: &reqwest::Client,
                handler: &mut $crate::render::ReplyHandler,
                data: $crate::client::SendData,
            ) -> Result<()> {
                // making a request builder
                let builder = self.request_builder(client, data)?;
                // calling 'openai_send_message_streaming' from the openai module, using the request builder, and await
                $crate::client::openai::openai_send_message_streaming(builder, handler).await
            }
        }
    };
}

// macro for defining functions to get configuration values
#[macro_export]
macro_rules! config_get_fn {
    ($field_name:ident, $fn_name:ident) => {
        // it creates a new function with the name specified by fn_name
        fn $fn_name(&self) -> anyhow::Result<String> {
            // configuration value is retrieved from field_name
            let api_key = self.config.$field_name.clone();
            // configuration value is None, attempts to retrieve it from an environment variable
            api_key
                .or_else(|| {
                    let env_prefix = Self::name(&self.config);
                    let env_name =
                        format!("{}_{}", env_prefix, stringify!($field_name)).to_ascii_uppercase();
                    std::env::var(&env_name).ok()
                })
                .ok_or_else(|| anyhow::anyhow!("Miss {}", stringify!($field_name)))
        }
    };
}

// trait for defining common client functionality
#[async_trait]
pub trait Client {
    // We just declare these and use the above macro to define them later
    fn config(&self) -> (&GlobalConfig, &Option<ExtraConfig>);

    fn models(&self) -> Vec<Model>;

    fn model(&self) -> &Model;

    fn set_model(&mut self, model: Model);

    // This function builds and returns a Reqwest client, based on the client's configuration
    fn build_client(&self) -> Result<ReqwestClient> {
        let mut builder = ReqwestClient::builder();
        let options = self.config().1;
        let timeout = options
            .as_ref()
            .and_then(|v| v.connect_timeout)
            .unwrap_or(10); // This sets connection timeout based on the provided configuration or defaults to 10 seconds
        let proxy = options.as_ref().and_then(|v| v.proxy.clone());
        // sets up any proxy configuration if provided
        builder = set_proxy(builder, &proxy)?;
        let client = builder
            .connect_timeout(Duration::from_secs(timeout))
            .build()
            .with_context(|| "Failed to build client")?;
        Ok(client)
    }

    // this function sends a message asynchronously and returns the response as a string
    fn send_message(&self, input: Input) -> Result<String> {
        // We use tokio, initialized lazily for using async/await
        init_tokio_runtime()?.block_on(async {
            let global_config = self.config().0;
            if global_config.read().dry_run {
                let content = global_config.read().echo_messages(&input);
                return Ok(content);
            }
            let client = self.build_client()?;
            let data = global_config.read().prepare_send_data(&input, false)?;
            self.send_message_inner(&client, data)
                .await
                .with_context(|| "Failed to get answer")
        })
    }

    // function for sending a message in stream mode
    fn send_message_streaming(&self, input: &Input, handler: &mut ReplyHandler) -> Result<()> {
        // listening for an abort signal, stops sending messages if it is received.
        async fn watch_abort(abort: AbortSignal) {
            loop {
                if abort.aborted() {
                    break;
                }
                sleep(Duration::from_millis(100)).await;
            }
        }
        let abort = handler.get_abort();
        let input = input.clone();
        init_tokio_runtime()?.block_on(async move {
            tokio::select! {
                ret = async {
                    let global_config = self.config().0;
                    if global_config.read().dry_run {
                        let content = global_config.read().echo_messages(&input);
                        let tokens = tokenize(&content);
                        for token in tokens {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            handler.text(&token)?;
                        }
                        return Ok(());
                    }
                    let client = self.build_client()?;
                    let data = global_config.read().prepare_send_data(&input, true)?;
                    self.send_message_streaming_inner(&client, handler, data).await
                } => {
                    handler.done()?;
                    ret.with_context(|| "Failed to get answer")
                }
                _ = watch_abort(abort.clone()) => {
                    handler.done()?;
                    Ok(())
                 },
            }
        })
    }

    // functions responsible for sending messages using the Reqwest
    // takes in a data payload, and a reply handler as input and returns a result
    async fn send_message_inner(&self, client: &ReqwestClient, data: SendData) -> Result<String>;

    // functions responsible for sending messages using the Reqwest
    // takes in a data payload, and a reply handler as input and returns a result
    async fn send_message_streaming_inner(
        &self,
        client: &ReqwestClient,
        handler: &mut ReplyHandler,
        data: SendData,
    ) -> Result<()>;
}

// Default implementation for ClientConfig
impl Default for ClientConfig {
    fn default() -> Self {
        Self::OpenAIConfig(OpenAIConfig::default()) // sets openAI config with its default values
    }
}

// struct for storing extra configuration options, all the elements in this struct are optional
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExtraConfig {
    pub proxy: Option<String>, // holds the proxy configuration
    pub connect_timeout: Option<u64>, // tells the connection timeout duration
}

// struct represents the data to be sent over the client
#[derive(Debug)]
pub struct SendData {
    pub messages: Vec<Message>, // vector of messages, which holds the content of the messages
    pub temperature: Option<f64>, // this determines the creativity and randomness of generated responses
    pub stream: bool, // indicates whether the message should be sent as streaming
}

// Represents a tuple containing prompt related info 
// like prompt name, description, requirement status, and prompt kind
pub type PromptType<'a> = (&'a str, &'a str, bool, PromptKind);

// function for generating a configuration object based on a list of input parameters
pub fn create_config(list: &[PromptType], client: &str) -> Result<Value> {
    let mut config = json!({
        "type": client,
    });
    for (path, desc, required, kind) in list {
        match kind {
            PromptKind::String => {
                let value = prompt_input_string(desc, *required)?;
                set_config_value(&mut config, path, kind, &value);
            }
            PromptKind::Integer => {
                let value = prompt_input_integer(desc, *required)?;
                set_config_value(&mut config, path, kind, &value);
            }
        }
    }

    let clients = json!(vec![config]);
    Ok(clients)
}

// function to send message as stream
#[allow(unused)]
pub async fn send_message_as_streaming<F, Fut>(
    builder: RequestBuilder,
    handler: &mut ReplyHandler,
    f: F,
) -> Result<()>
where
    F: FnOnce(RequestBuilder) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let text = f(builder).await?;
    handler.text(&text)?;
    handler.done()?;

    Ok(())
}

// function to patch system message
pub fn patch_system_message(messages: &mut Vec<Message>) {
    if messages[0].role.is_system() {
        let system_message = messages.remove(0);
        if let (Some(message), MessageContent::Text(system_text)) =
            (messages.get_mut(0), system_message.content)
        {
            if let MessageContent::Text(text) = message.content.clone() {
                message.content = MessageContent::Text(format!("{}\n\n{}", system_text, text))
            }
        }
    }
}

// function to set configuration value
fn set_config_value(json: &mut Value, path: &str, kind: &PromptKind, value: &str) {
    let segs: Vec<&str> = path.split('.').collect();
    match segs.as_slice() {
        [name] => json[name] = to_json(kind, value),
        [scope, name] => match scope.split_once('[') {
            None => {
                if json.get(scope).is_none() {
                    let mut obj = json!({});
                    obj[name] = to_json(kind, value);
                    json[scope] = obj;
                } else {
                    json[scope][name] = to_json(kind, value);
                }
            }
            Some((scope, _)) => {
                if json.get(scope).is_none() {
                    let mut obj = json!({});
                    obj[name] = to_json(kind, value);
                    json[scope] = json!([obj]);
                } else {
                    json[scope][0][name] = to_json(kind, value);
                }
            }
        },
        _ => {}
    }
}

// Function to PromptKind to json
fn to_json(kind: &PromptKind, value: &str) -> Value {
    if value.is_empty() {
        return Value::Null;
    }
    match kind {
        PromptKind::String => value.into(),
        PromptKind::Integer => match value.parse::<i32>() {
            Ok(value) => value.into(),
            Err(_) => value.into(),
        },
    }
}

// functiion to set a proxy for our client
fn set_proxy(builder: ClientBuilder, proxy: &Option<String>) -> Result<ClientBuilder> {
    let proxy = if let Some(proxy) = proxy {
        if proxy.is_empty() || proxy == "false" || proxy == "-" {
            return Ok(builder);
        }
        proxy.clone()
    } else if let Ok(proxy) = env::var("HTTPS_PROXY").or_else(|_| env::var("ALL_PROXY")) {
        proxy
    } else {
        return Ok(builder);
    };
    let builder =
        builder.proxy(Proxy::all(&proxy).with_context(|| format!("Invalid proxy `{proxy}`"))?);
    Ok(builder)
}
