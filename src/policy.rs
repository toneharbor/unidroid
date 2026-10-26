use crate::{Emulator, JniCall, JniHandler, JniValue, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub(crate) mod handler;
mod parse;
pub(crate) mod value;
pub use handler::PolicyHandler;

/// Ordered Java answers. Sequence positions belong to this policy, not its handlers.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    #[serde(deserialize_with = "parse::rules")]
    rules: Vec<Rule>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    op: Option<String>,
    #[serde(default, rename = "static", skip_serializing_if = "Option::is_none")]
    is_static: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    values: Option<Vec<Value>>,
    #[serde(skip)]
    position: usize,
}

fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

impl Policy {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        Self::parse(&std::fs::read_to_string(path)?)
    }

    pub fn parse(text: &str) -> Result<Self> {
        serde_json::from_str(text).map_err(|error| {
            let message = error.to_string();
            if message.starts_with("policy rule ") {
                message.into()
            } else {
                format!("policy: {message}").into()
            }
        })
    }

    pub fn handler<'a>(&'a mut self, fallback: &'a mut dyn JniHandler) -> PolicyHandler<'a> {
        PolicyHandler {
            policy: self,
            fallback,
            error: None,
        }
    }

    pub(crate) fn member(
        &mut self,
        emu: &mut Emulator,
        call: &JniCall<'_>,
    ) -> Option<Result<JniValue>> {
        self.answer(
            emu,
            Request {
                op: call.op.name(),
                is_static: Some(call.is_static),
                class: Some(call.class),
                name: Some(call.name),
                signature: Some(call.signature),
                from: None,
                to: None,
            },
            call.type_code,
        )
    }

    pub(crate) fn is_assignable(
        &mut self,
        emu: &mut Emulator,
        from: &str,
        to: &str,
    ) -> Option<Result<JniValue>> {
        self.answer(
            emu,
            Request {
                op: "is_assignable",
                is_static: None,
                class: None,
                name: None,
                signature: None,
                from: Some(from),
                to: Some(to),
            },
            b'Z',
        )
    }

    fn answer(
        &mut self,
        emu: &mut Emulator,
        request: Request<'_>,
        kind: u8,
    ) -> Option<Result<JniValue>> {
        for (index, rule) in self.rules.iter_mut().enumerate() {
            if !rule.matches(&request) {
                continue;
            }
            let value = if let Some(values) = &rule.values {
                let Some(value) = values.get(rule.position) else {
                    continue;
                };
                rule.position += 1;
                Some(value)
            } else {
                rule.value.as_ref()
            };
            let call = match (request.from, request.to) {
                (Some(from), Some(to)) => format!("is_assignable {from}->{to}"),
                _ => format!(
                    "{}{} {}->{}{}",
                    if request.is_static == Some(true) {
                        "static "
                    } else {
                        ""
                    },
                    request.op,
                    request.class.unwrap(),
                    request.name.unwrap(),
                    request.signature.unwrap()
                ),
            };
            let context = emu.jni_message(format!("policy rule {index} for {call}"));
            return Some(value::decode(emu, kind, value, &context));
        }
        None
    }
}

struct Request<'a> {
    op: &'a str,
    is_static: Option<bool>,
    class: Option<&'a str>,
    name: Option<&'a str>,
    signature: Option<&'a str>,
    from: Option<&'a str>,
    to: Option<&'a str>,
}

impl Rule {
    fn matches(&self, request: &Request<'_>) -> bool {
        self.is_static
            .is_none_or(|value| Some(value) == request.is_static)
            && [
                (&self.op, Some(request.op)),
                (&self.class, request.class),
                (&self.name, request.name),
                (&self.signature, request.signature),
                (&self.from, request.from),
                (&self.to, request.to),
            ]
            .into_iter()
            .all(|(pattern, text)| {
                pattern
                    .as_ref()
                    .is_none_or(|pattern| text.is_some_and(|text| wildcard(pattern, text)))
            })
    }
}

fn wildcard(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*').peekable();
    let Some(mut rest) = text.strip_prefix(parts.next().unwrap()) else {
        return false;
    };
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return rest.ends_with(part);
        }
        let Some(index) = rest.find(part) else {
            return false;
        };
        rest = &rest[index + part.len()..];
    }
    rest.is_empty()
}
