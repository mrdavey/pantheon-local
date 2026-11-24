//! Preserve the byte representation used by stored command receipts and eval
//! fingerprints. Object field order, absent fields, and escaping are significant.
use anyhow::{Context, Result};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

fn empty(v: &Value) -> bool {
    v.is_null()
        || v.as_str() == Some("")
        || v.as_bool() == Some(false)
        || v.as_f64() == Some(0.)
        || v.as_array().is_some_and(Vec::is_empty)
        || v.as_object().is_some_and(Map::is_empty)
}

fn fields(input: &Value, keys: &[&str]) -> Value {
    let mut out = Map::new();
    for &key in keys {
        let v = &input[key];
        let mandatory = matches!(
            key,
            "kind" | "checks" | "metrics" | "objective" | "name" | "direction" | "unit"
        );
        let present = match key {
            "input" | "expected" => input.get(key).is_some(),
            "expected_revision" | "recipe" | "rule" => !v.is_null(),
            _ => !empty(v),
        };
        if !mandatory && !present {
            continue;
        }
        let field = match key {
            "cases" => Value::Array(
                v.as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| {
                        fields(
                            c,
                            &[
                                "id",
                                "revision",
                                "driver",
                                "program",
                                "interpreter",
                                "digest",
                                "input",
                                "expected",
                                "holdback",
                                "required",
                                "samples",
                                "seed",
                            ],
                        )
                    })
                    .collect(),
            ),
            "metrics" if v.is_array() => Value::Array(
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|m| fields(m, &["name", "unit", "direction", "aggregate"]))
                    .collect(),
            ),
            "recipe" => recipe(v),
            "rule" => fields(
                v,
                &[
                    "metrics",
                    "objective",
                    "minimum",
                    "limits",
                    "tie_break",
                    "min_samples",
                    "min_lead",
                ],
            ),
            "limits" => v
                .as_object()
                .map(|m| json!(m.iter().collect::<std::collections::BTreeMap<_, _>>()))
                .unwrap_or(Value::Null),
            _ if mandatory && v.is_null() && !matches!(key, "metrics" | "checks") => "".into(),
            _ => v.clone(),
        };
        out.insert(key.into(), field);
    }
    out.into()
}
fn recipe(v: &Value) -> Value {
    fields(v, &["cases", "rule", "rubric", "checks", "preview"])
}
fn digest(v: &Value) -> Result<String> {
    let encoded = serde_json::to_string(v)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    Ok(hex::encode(Sha256::digest(encoded.as_bytes())))
}
pub fn command_digest(v: &Value) -> Result<String> {
    digest(&fields(
        v,
        &[
            "id",
            "expected_revision",
            "donors",
            "kind",
            "task",
            "text",
            "candidate",
            "tokens",
            "recipe",
        ],
    ))
}
pub fn inputs_digest(frozen: &Value, commit: &str, binaries: &Value) -> Result<String> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        "x86" => "386",
        other => other,
    };
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let bins = binaries
        .as_object()
        .context("invalid binary fingerprints")?
        .iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    digest(
        &json!({"Recipe":recipe(frozen),"Commit":commit,"OS":os,"Arch":arch,"Path":std::env::var("PATH").unwrap_or_default(),"Lang":std::env::var("LANG").unwrap_or_default(),"Locale":std::env::var("LC_ALL").unwrap_or_default(),"Binaries":bins}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_receipt_preserves_field_order_and_zero_revision() {
        let command =
            json!({"text":"A&B","tokens":0,"kind":"answer","expected_revision":0,"id":"retry"});
        let bytes = br#"{"id":"retry","expected_revision":0,"kind":"answer","text":"A\u0026B"}"#;
        assert_eq!(
            command_digest(&command).unwrap(),
            hex::encode(Sha256::digest(bytes))
        );
    }
}
