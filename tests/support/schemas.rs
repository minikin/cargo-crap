//! The published JSON schemas, compiled offline. A `$ref` into another file
//! under `schemas/` resolves to the copy in this checkout, so a test checks
//! the schemas being changed, not the ones on main, and needs no network.

use std::path::Path;

/// Where the schemas are published: the base every `$id` and relative
/// `$ref` resolves against.
const PUBLISHED: &str = "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/";

/// Serves a published schema URL from the local `schemas/` directory.
struct LocalSchemas;

impl jsonschema::Retrieve for LocalSchemas {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        let file = uri
            .as_str()
            .strip_prefix(PUBLISHED)
            .ok_or_else(|| format!("not a published schema: {}", uri.as_str()))?;
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("schemas")
            .join(file);
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }
}

/// The schema at `path`, relative to the crate root, ready to validate.
pub fn compile(path: &str) -> jsonschema::Validator {
    let raw = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|e| panic!("read {path}: {e}"));
    let value: serde_json::Value = serde_json::from_str(&raw).expect("schema must be valid JSON");
    jsonschema::options()
        .with_retriever(LocalSchemas)
        .build(&value)
        .unwrap_or_else(|e| panic!("{path} must compile: {e}"))
}
