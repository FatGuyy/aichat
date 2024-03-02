use inquire::{required, validator::Validation, Text};

const MSG_REQUIRED: &str = "This field is required"; // message indicating that a field is required
const MSG_OPTIONAL: &str = "Optional field - Press ↵ to skip"; // message indicating that a field is optional and can be skipped by pressing Enter

// this function prompts the user to input a string
pub fn prompt_input_string(desc: &str, required: bool) -> anyhow::Result<String> {
    // creating a new Text prompt with the provided description
    let mut text = Text::new(desc);
    // input is required
    if required {
        // we add a validator to check if the input is empty
        text = text.with_validator(required!(MSG_REQUIRED))
    } else {
        // else, we add a help message indicating that the input is optional
        text = text.with_help_message(MSG_OPTIONAL)
    }
    // return the text
    text.prompt().map_err(prompt_op_err)
}

// this function is similar to the above but for integer input
pub fn prompt_input_integer(desc: &str, required: bool) -> anyhow::Result<String> {
    // creating a new Text prompt with the provided description
    let mut text = Text::new(desc);
    if required {
        // the  validator checks if the input is empty and returns an appropriate validation result
        text = text.with_validator(|text: &str| {
            let out = if text.is_empty() {
                Validation::Invalid(MSG_REQUIRED.into())
            } else {
                validate_integer(text)
            };
            Ok(out)
        })
    } else {
        text = text
            .with_validator(|text: &str| {
                let out = if text.is_empty() {
                    Validation::Valid
                } else {
                    validate_integer(text)
                };
                Ok(out)
            })
            .with_help_message(MSG_OPTIONAL)
    }
    text.prompt().map_err(prompt_op_err)
}

//This function is a error handler used for mapping any error to a custom error message
pub fn prompt_op_err<T>(_: T) -> anyhow::Error {
    anyhow::anyhow!("Not finish questionnaire, try again later!")
}
// This enum represents the kind of prompt being used
#[derive(Debug, Clone, Copy)]
pub enum PromptKind {
    String,
    Integer,
}

// this function validates whether a given string can be parsed as an integer
fn validate_integer(text: &str) -> Validation {
    if text.parse::<i32>().is_err() {
        Validation::Invalid("Must be a integer".into())
    } else {
        Validation::Valid
    }
}
