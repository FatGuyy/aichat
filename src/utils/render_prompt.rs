use std::collections::HashMap;

/// Render REPL prompt
///
/// The template comprises plain text and `{...}`.
///
/// The syntax of `{...}`:
/// - `{var}` - When `var` has a value, replace `var` with the value and eval `template`
/// - `{?var <template>}` - Eval `template` when `var` is evaluated as true
/// - `{!var <template>}` - Eval `template` when `var` is evaluated as false

// this function takes a template string and a hashmap of variables and renders them
pub fn render_prompt(template: &str, variables: &HashMap<&str, String>) -> String {
    // we first parse the template
    let exprs = parse_template(template);
    // the we return the rendered string
    eval_exprs(&exprs, variables)
}

// This function parses the template string
fn parse_template(template: &str) -> Vec<Expr> {
    let chars: Vec<char> = template.chars().collect();
    let mut exprs = vec![];
    let mut current = vec![];
    let mut balances = vec![];
    // iterating over each character in the template string
    for ch in chars.iter().cloned() {
        if !balances.is_empty() {
            // if we find a matching closing brace, we start to pop the balances and parse the characters
            if ch == '}' {
                balances.pop();
                if balances.is_empty() {
                    if !current.is_empty() {
                        let block = parse_block(&mut current);
                        exprs.push(block)
                    }
                } else {
                    current.push(ch);
                }
            }
            // If we encounter an opening brace
            else if ch == '{' {
                // we start collecting characters
                balances.push(ch);
                current.push(ch);
            } else {
                // else we just keep pushing
                current.push(ch);
            }
        } else if ch == '{' {
            balances.push(ch);
            add_text(&mut exprs, &mut current);
        } else {
            current.push(ch)
        }
    }
    add_text(&mut exprs, &mut current);
    exprs
}

// this function parses a block of text
fn parse_block(current: &mut Vec<char>) -> Expr {
    let value: String = current.drain(..).collect();
    // spliting the block into a name and the rest of the text
    match value.split_once(' ') {
        Some((name, tail)) => {
            // If the name starts with ?, it is a conditional block with a positive condition
            if let Some(name) = name.strip_prefix('?') {
                // it parses the rest of the text using parse_template
                let block_exprs = parse_template(tail);
                // creating an Expr::Block variant with a positive condition
                Expr::Block(BlockType::Yes, name.to_string(), block_exprs)
            }
            // if it starts with !, it is a conditional block with a negative condition
            else if let Some(name) = name.strip_prefix('!') {
                let block_exprs = parse_template(tail);
                // creating an Expr::Block variant with negaive condition
                Expr::Block(BlockType::No, name.to_string(), block_exprs)
            } else {
                Expr::Text(format!("{{{value}}}"))
            }
        }
        None => Expr::Variable(value),
    }
}

// this function returns the rendered string from a vector
fn eval_exprs(exprs: &[Expr], variables: &HashMap<&str, String>) -> String {
    let mut output = String::new();
    // iterating over each expr, for each variant ie Text, Variable, or Block
    for part in exprs {
        match part {
            // for text variant, we append the text to the output string
            Expr::Text(text) => output.push_str(text),
            // for variable variant, we retrieve the value from the hashmap and append it to the output string
            Expr::Variable(variable) => {
                let value = variables
                    .get(variable.as_str())
                    .cloned()
                    .unwrap_or_default();
                // push it on the output
                output.push_str(&value);
            }
            // for block variant, we evaluate the condition based on the variable's value and evaluate the inner expressions if the condition is met
            Expr::Block(typ, variable, block_exprs) => {
                let value = variables
                    .get(variable.as_str())
                    .cloned()
                    .unwrap_or_default();
                match typ {
                    BlockType::Yes => {
                        if truly(&value) {
                            let block_output = eval_exprs(block_exprs, variables);
                            // push the smaller block on the output
                            output.push_str(&block_output)
                        }
                    }
                    BlockType::No => {
                        if !truly(&value) {
                            let block_output = eval_exprs(block_exprs, variables);
                            // push the smaller block on the output
                            output.push_str(&block_output)
                        }
                    }
                }
            }
        }
    }
    // return the output
    output
}

// this function adds a text expression to the vector of expressions
// to handle consecutive text blocks in the template
fn add_text(exprs: &mut Vec<Expr>, current: &mut Vec<char>) {
    if current.is_empty() {
        return;
    }
    let value: String = current.drain(..).collect();
    exprs.push(Expr::Text(value));
}

// this function determines whether a string value is "true"
fn truly(value: &str) -> bool {
    // returning true if the string is not empty and not equal to "0" or "false"
    !(value.is_empty() || value == "0" || value == "false")
}

// this enum represents different types of expressions that can occur within a template
#[derive(Debug)]
enum Expr {
    Text(String),
    Variable(String),
    Block(BlockType, String, Vec<Expr>),
}

// this enum represents the type of a conditional block
#[derive(Debug)]
enum BlockType {
    Yes,
    No,
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! assert_render {
        ($template:expr, [$(($key:literal, $value:literal),)*], $expect:literal) => {
            let data = HashMap::from([
                $(($key, $value.into()),)*
            ]);
            assert_eq!(render_prompt($template, &data), $expect);
        };
    }

    #[test]
    fn test_render() {
        let prompt = "{?session {session}{?role /}}{role}{?session )}{!session >}";
        assert_render!(prompt, [], ">");
        assert_render!(prompt, [("role", "coder"),], "coder>");
        assert_render!(prompt, [("session", "temp"),], "temp)");
        assert_render!(
            prompt,
            [("session", "temp"), ("role", "coder"),],
            "temp/coder)"
        );
    }
}
