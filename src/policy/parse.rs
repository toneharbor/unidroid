use super::{Rule, wildcard};
use serde::Deserialize;
use serde::de::{DeserializeSeed, Deserializer, Error, SeqAccess, Visitor};
use std::fmt;

pub(super) fn rules<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Rule>, D::Error> {
    struct Rules;
    impl<'de> Visitor<'de> for Rules {
        type Value = Vec<Rule>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a list of policy rules")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut rules = Vec::new();
            loop {
                let index = rules.len();
                let Some(rule) = seq.next_element_seed(RuleSeed { index })? else {
                    return Ok(rules);
                };
                if let Some(op) = &rule.op
                    && !["call", "get", "set", "new", "is_assignable"]
                        .iter()
                        .any(|name| wildcard(op, name))
                {
                    return Err(A::Error::custom(format!(
                        "policy rule {index}: unknown op {op}"
                    )));
                }
                if rule.value.is_some() && rule.values.is_some() {
                    return Err(A::Error::custom(format!(
                        "policy rule {index}: value and values cannot both be present"
                    )));
                }
                rules.push(rule);
            }
        }
    }
    deserializer.deserialize_seq(Rules)
}

struct RuleSeed {
    index: usize,
}

impl<'de> DeserializeSeed<'de> for RuleSeed {
    type Value = Rule;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        Rule::deserialize(deserializer)
            .map_err(|error| D::Error::custom(format!("policy rule {}: {error}", self.index)))
    }
}
