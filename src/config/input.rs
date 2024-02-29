use crate::client::{ImageUrl, MessageContent, MessageContentPart, ModelCapabilities};
use crate::utils::sha256sum;

use anyhow::{bail, Context, Result};
use base64::{self, engine::general_purpose::STANDARD, Engine};
use fancy_regex::Regex;
use lazy_static::lazy_static;
use mime_guess::from_path;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

// array of strings representing common image file extensions
const IMAGE_EXTS: [&str; 5] = ["png", "jpeg", "jpg", "webp", "gif"];

lazy_static! {
    // regex pattern for matching URLs
    static ref URL_RE: Regex = Regex::new(r"^[A-Za-z0-9_-]{2,}:/").unwrap();
}

// this struct represents input data consisting of text and media files
#[derive(Debug, Clone)]
pub struct Input {
    text: String,
    medias: Vec<String>,
    data_urls: HashMap<String, String>,
}

impl Input {
    // constructor method that creates an Input instance from a str
    pub fn from_str(text: &str) -> Self {
        Self {
            text: text.to_string(),
            medias: Default::default(),
            data_urls: Default::default(),
        }
    }

    // another constructor that creates an Input instance from a string and files, using file path
    pub fn new(text: &str, files: Vec<String>) -> Result<Self> {
        let mut texts = vec![text.to_string()];
        let mut medias = vec![];
        let mut data_urls = HashMap::new();
        for file_item in files.into_iter() {
            match resolve_path(&file_item) {
                Some(file_path) => {
                    let file_path = fs::canonicalize(file_path)
                        .with_context(|| format!("Unable to use file '{file_item}"))?;
                    if is_image_ext(&file_path) {
                        let data_url = read_media_to_data_url(&file_path)?;
                        data_urls.insert(sha256sum(&data_url), file_path.display().to_string());
                        medias.push(data_url)
                    } else {
                        let mut text = String::new();
                        let mut file = File::open(&file_path)
                            .with_context(|| format!("Unable to open file '{file_item}'"))?;
                        file.read_to_string(&mut text)
                            .with_context(|| format!("Unable to read file '{file_item}'"))?;
                        texts.push(text);
                    }
                }
                None => {
                } else {
                    if is_image_ext(Path::new(&file_item)) {
                        medias.push(file_item)
                        bail!("Unable to use file '{file_item}");
                    }
                }
            }
        }

        Ok(Self {
            text: texts.join("\n"),
            medias,
            data_urls,
        })
    }

    // returns a clone of the data urls stored in the input
    pub fn data_urls(&self) -> HashMap<String, String> {
        self.data_urls.clone()
    }

    // renders the input into a format of text and media files
    pub fn render(&self) -> String {
        if self.medias.is_empty() {
            return self.text.clone();
        }
        let text = if self.text.is_empty() {
            self.text.to_string()
        } else {
            format!(" -- {}", self.text)
        };
        let files: Vec<String> = self
            .medias
            .iter()
            .cloned()
            .map(|url| resolve_data_url(&self.data_urls, url))
            .collect();
        format!(".file {}{}", files.join(" "), text)
    }

    // Converts the input data into a MessageContent enum variant
    pub fn to_message_content(&self) -> MessageContent {
        if self.medias.is_empty() {
            MessageContent::Text(self.text.clone())
        } else {
            let mut list: Vec<MessageContentPart> = self
                .medias
                .iter()
                .cloned()
                .map(|url| MessageContentPart::ImageUrl {
                    image_url: ImageUrl { url },
                })
                .collect();
            if !self.text.is_empty() {
                list.insert(
                    0,
                    MessageContentPart::Text {
                        text: self.text.clone(),
                    },
                );
            }
            MessageContent::Array(list)
        }
    }

    // determines the required capabilities based on the presence of media files
    pub fn required_capabilities(&self) -> ModelCapabilities {
        if !self.medias.is_empty() {
            ModelCapabilities::Vision
        } else {
            ModelCapabilities::Text
        }
    }
}
// this function formats the url
pub fn resolve_data_url(data_urls: &HashMap<String, String>, data_url: String) -> String {
    // If the data_url starts with "data:" 
    if data_url.starts_with("data:") {
        // we calculate the SHA-256 hash of the url
        let hash = sha256sum(&data_url);
        // check if it exists in the data_urls map
        if let Some(path) = data_urls.get(&hash) {
            // returning the corresponding path otherwise
            return path.to_string();
        }
        // we return the data_url
        data_url
    } else {
        // else we just return the data_url
        data_url
    }
}

// making the path to work with
fn resolve_path(file: &str) -> Option<PathBuf> {
    if let Ok(true) = URL_RE.is_match(file) {
        return None;
    }
    // checking if the file path starts with "~" (indicating a home directory shortcut) and expanding it to the home directory if possible
    let path = if let (Some(file), Some(home)) = (file.strip_prefix('~'), dirs::home_dir()) {
        home.join(file)
    } else {
        // Otherwise, we resolve the file path relative to the current directory
        std::env::current_dir().ok()?.join(file)
    };
    // returning an option, either containing the resolved file path or None if the input path is an url
    Some(path)
}

// checks if its extension matches the image file extensions defined in IMAGE_EXTS
fn is_image_ext(path: &Path) -> bool {
    // extracting the extension using path.extension()
    path.extension()
        .map(|v| {
            IMAGE_EXTS
                .iter()
                // converting it to lowercase for case-insensitive comparison with the image extensions
                .any(|ext| *ext == v.to_string_lossy().to_lowercase())
        })
        .unwrap_or_default()
        // we return true if the extension matches any of the image extensions
        // else we return false
}

// this function reads an image file from the given path and encodes it into a url string
fn read_media_to_data_url<P: AsRef<Path>>(image_path: P) -> Result<String> {
    // determining the MIME type of the image file
    let mime_type = from_path(&image_path).first_or_octet_stream().to_string();

    // we open the image file at the path and read its content into a buffer
    let mut file = File::open(image_path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;

    // then, we encode the content of the image file using Base64(STANDARD) encoding 
    // This encoded data will be included in the data URL
    let encoded_image = STANDARD.encode(buffer);
    // constructing the data url string using the MIME type and the Base64-encoded image data
    let data_url = format!("data:{};base64,{}", mime_type, encoded_image);

    // returning the data_url inside Ok, so it gets wrapped in a Result
    Ok(data_url)
}
