//! JER encoding instructions as parsed and as resolved for code generation.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Array,
    Base64,
    Name,
    Object,
    Text,
    Unwrapped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rename {
    Literal(String),
    Reference { module: Option<String>, identifier: String },
    Capitalized,
    UpperCamelCased,
    UpperCased,
    LowerCamelCased,
    LowerCased,
}

impl Rename {
    pub(crate) fn apply(&self, name: &str) -> Option<String> {
        Some(match self {
            Self::Reference { .. } => return None,
            Self::Literal(value) => value.clone(),
            Self::UpperCased => name.to_ascii_uppercase(),
            Self::LowerCased => name.to_ascii_lowercase(),
            Self::Capitalized => capitalize(name),
            Self::UpperCamelCased => name.split('-').map(capitalize).collect(),
            Self::LowerCamelCased => {
                let mut parts = name.split('-');
                let mut result = parts.next().unwrap_or_default().to_owned();
                for part in parts {
                    result.push_str(&capitalize(part));
                }
                result
            }
        })
    }
}

fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        let mut result = String::from(first.to_ascii_uppercase());
        result.push_str(chars.as_str());
        result
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Instruction {
    Enable(Category),
    Disable(Category),
    Name(Rename),
    Text(Vec<(Option<String>, Rename)>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    All,
    Imports(String),
    BuiltIn(Category),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub instruction: Instruction,
    pub targets: Vec<Target>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub representation: Option<Representation>,
    pub names: Vec<(String, String)>,
    pub text: Option<Vec<(String, String)>>,
}

impl Resolved {
    pub(crate) fn is_empty(&self) -> bool {
        self.representation.is_none() && self.names.is_empty() && self.text.is_none()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Representation {
    Default,
    Array,
    Base64,
    Object,
    Unwrapped,
}
