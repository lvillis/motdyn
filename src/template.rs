use std::env;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    UnclosedVariable,
    EmptyVariable,
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnclosedVariable => write!(f, "template variable is missing closing '}}'"),
            Self::EmptyVariable => write!(f, "template variable name is empty"),
        }
    }
}

impl std::error::Error for TemplateError {}

pub fn render_template(input: &str, env_prefix: Option<&str>) -> Result<String, TemplateError> {
    render_template_with(input, env_prefix, |key| {
        env::var_os(key).map(|value| value.to_string_lossy().into_owned())
    })
}

fn render_template_with<F>(
    input: &str,
    env_prefix: Option<&str>,
    env_lookup: F,
) -> Result<String, TemplateError>
where
    F: Fn(&str) -> Option<String>,
{
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(index) = rest.find('$') {
        output.push_str(&rest[..index]);
        rest = &rest[index + 1..];

        if let Some(after_dollar) = rest.strip_prefix('$') {
            output.push('$');
            rest = after_dollar;
            continue;
        }

        let Some(after_open) = rest.strip_prefix('{') else {
            output.push('$');
            continue;
        };

        let Some(close_index) = after_open.find('}') else {
            return Err(TemplateError::UnclosedVariable);
        };
        let expression = &after_open[..close_index];
        let (name, default) = parse_variable_expression(expression)?;
        output.push_str(&resolve_variable(name, default, env_prefix, &env_lookup));
        rest = &after_open[close_index + 1..];
    }

    output.push_str(rest);
    Ok(output)
}

fn parse_variable_expression(expression: &str) -> Result<(&str, Option<&str>), TemplateError> {
    let (name, default) = expression
        .split_once(":-")
        .map_or((expression, None), |(name, default)| (name, Some(default)));

    if name.is_empty() {
        return Err(TemplateError::EmptyVariable);
    }

    Ok((name, default))
}

fn resolve_variable<F>(
    name: &str,
    default: Option<&str>,
    env_prefix: Option<&str>,
    env_lookup: &F,
) -> String
where
    F: Fn(&str) -> Option<String>,
{
    let key = env_key(name, env_prefix);
    let value = env_lookup(&key).unwrap_or_default();

    if value.is_empty() {
        default.unwrap_or_default().to_string()
    } else {
        value
    }
}

fn env_key(name: &str, env_prefix: Option<&str>) -> String {
    match env_prefix.filter(|prefix| !prefix.is_empty()) {
        Some(prefix) if !name.starts_with(prefix) => format!("{prefix}{name}"),
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_env_variables_and_defaults() {
        let rendered = render_template_with(
            "name=${NAME} missing=${EMPTY:-fallback}",
            Some("MOTDYN_TEMPLATE_"),
            |key| match key {
                "MOTDYN_TEMPLATE_NAME" => Some("api".to_string()),
                _ => None,
            },
        )
        .unwrap();

        assert_eq!(rendered, "name=api missing=fallback");
    }

    #[test]
    fn renders_literal_dollar() {
        let rendered = render_template("cost=$$5 raw=$x", None).unwrap();
        assert_eq!(rendered, "cost=$5 raw=$x");
    }

    #[test]
    fn rejects_unclosed_variable() {
        assert_eq!(
            render_template("${BROKEN", None).unwrap_err(),
            TemplateError::UnclosedVariable
        );
    }
}
