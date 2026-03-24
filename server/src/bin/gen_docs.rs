/// gen-docs: generates docs/api.md from the utoipa OpenAPI spec.
///
/// Run from the server/ directory:
///   cargo run --bin gen-docs
use serde_json::Value;
use terminal_server::openapi::ApiDoc;
use utoipa::OpenApi;

fn main() {
    let json = ApiDoc::openapi().to_pretty_json().unwrap();
    let spec: Value = serde_json::from_str(&json).unwrap();

    let md = render_markdown(&spec);

    let out_path = "../docs/api.md";
    std::fs::write(out_path, &md).unwrap();
    println!("Written to {out_path}");
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn render_markdown(spec: &Value) -> String {
    let mut out = String::new();

    // Header
    let title = spec["info"]["title"].as_str().unwrap_or("API");
    let version = spec["info"]["version"].as_str().unwrap_or("");
    let desc = spec["info"]["description"].as_str().unwrap_or("");

    out += &format!("# {title}\n\n");
    out += &format!("> **Version:** {version}\n\n");
    out += &format!("{desc}\n\n");
    out += "---\n\n";
    out += "## Endpoints\n\n";

    // Collect and sort paths for deterministic output
    let schemas = &spec["components"]["schemas"];
    let mut paths: Vec<(&str, &Value)> = spec["paths"]
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k.as_str(), v)).collect())
        .unwrap_or_default();
    paths.sort_by_key(|(p, _)| *p);

    let method_order = ["get", "post", "put", "patch", "delete"];

    for (path, path_item) in &paths {
        for method in method_order {
            let Some(op) = path_item.get(method) else {
                continue;
            };

            let summary = op["summary"].as_str().unwrap_or("");
            let description = op["description"].as_str().unwrap_or("");

            out += &format!("### `{} {}`\n\n", method.to_uppercase(), path);

            if !summary.is_empty() {
                out += &format!("{summary}\n\n");
            }
            if !description.is_empty() {
                out += &format!("{description}\n\n");
            }

            // Path + query parameters
            if let Some(params) = op["parameters"].as_array() {
                let path_params: Vec<_> = params
                    .iter()
                    .filter(|p| p["in"].as_str() == Some("path"))
                    .collect();
                let query_params: Vec<_> = params
                    .iter()
                    .filter(|p| p["in"].as_str() == Some("query"))
                    .collect();

                if !path_params.is_empty() {
                    out += "**Path Parameters**\n\n";
                    out += "| Name | Type | Description |\n|------|------|-------------|\n";
                    for p in &path_params {
                        let name = p["name"].as_str().unwrap_or("");
                        let typ = schema_type_name(&p["schema"], schemas);
                        let desc = p["description"].as_str().unwrap_or("");
                        out += &format!("| `{name}` | {typ} | {desc} |\n");
                    }
                    out += "\n";
                }

                if !query_params.is_empty() {
                    out += "**Query Parameters**\n\n";
                    out += "| Name | Type | Required | Description |\n|------|------|----------|-------------|\n";
                    for p in &query_params {
                        let name = p["name"].as_str().unwrap_or("");
                        let typ = schema_type_name(&p["schema"], schemas);
                        let req = if p["required"].as_bool() == Some(true) {
                            "✓"
                        } else {
                            "—"
                        };
                        let desc = p["description"].as_str().unwrap_or("");
                        out += &format!("| `{name}` | {typ} | {req} | {desc} |\n");
                    }
                    out += "\n";
                }
            }

            // Request body
            if let Some(body) = op.get("requestBody") {
                let schema =
                    resolve_schema(&body["content"]["application/json"]["schema"], schemas);
                if !schema.is_null() {
                    out += "**Request Body** (`application/json`)\n\n";
                    out += &render_schema_fields(schema, schemas);
                    out += "\n";
                }
            }

            // Responses
            if let Some(responses) = op["responses"].as_object() {
                out += "**Responses**\n\n";
                out += "| Status | Description |\n|--------|-------------|\n";
                let mut codes: Vec<_> = responses.iter().collect();
                codes.sort_by_key(|(c, _)| c.parse::<u16>().unwrap_or(999));
                for (code, resp) in &codes {
                    let desc = resp["description"].as_str().unwrap_or("");
                    // Mention the schema type if present
                    let body_ref = &resp["content"]["application/json"]["schema"];
                    let type_hint = if !body_ref.is_null() {
                        let resolved = resolve_schema(body_ref, schemas);
                        if let Some(title) = resolved.get("title").and_then(|t| t.as_str()) {
                            format!(" → `{title}`")
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };
                    out += &format!("| `{code}` | {desc}{type_hint} |\n");
                }
                out += "\n";

                // Inline response body schema for 2xx
                for (code, resp) in &codes {
                    if code.starts_with('2') {
                        let body_schema = &resp["content"]["application/json"]["schema"];
                        if body_schema.is_null() {
                            continue;
                        }
                        let resolved = resolve_schema(body_schema, schemas);
                        // array wrapper?
                        let (is_array, item_schema) = if resolved["type"].as_str() == Some("array")
                        {
                            (true, resolve_schema(&resolved["items"], schemas))
                        } else {
                            (false, resolved)
                        };
                        let fields = render_schema_fields(item_schema, schemas);
                        if !fields.is_empty() {
                            let label = if is_array {
                                "Response Body (array items)"
                            } else {
                                "Response Body"
                            };
                            out += &format!("**{label}**\n\n{fields}\n");
                        }
                    }
                }
            }

            out += "---\n\n";
        }
    }

    // Schemas appendix
    if let Some(schema_map) = schemas.as_object() {
        if !schema_map.is_empty() {
            out += "## Schemas\n\n";
            let mut names: Vec<_> = schema_map.keys().collect();
            names.sort();
            for name in names {
                let schema = &schemas[name.as_str()];
                out += &format!("### `{name}`\n\n");
                if let Some(desc) = schema["description"].as_str() {
                    out += &format!("{desc}\n\n");
                }
                let fields = render_schema_fields(schema, schemas);
                if !fields.is_empty() {
                    out += &fields;
                    out += "\n";
                }
            }
        }
    }

    out
}

fn resolve_schema<'a>(schema: &'a Value, schemas: &'a Value) -> &'a Value {
    if let Some(ref_path) = schema.get("$ref").and_then(|v| v.as_str()) {
        let name = ref_path
            .strip_prefix("#/components/schemas/")
            .unwrap_or(ref_path);
        let resolved = &schemas[name];
        if !resolved.is_null() {
            return resolved;
        }
    }
    schema
}

fn schema_type_name(schema: &Value, schemas: &Value) -> String {
    if let Some(ref_path) = schema.get("$ref").and_then(|v| v.as_str()) {
        return ref_path
            .strip_prefix("#/components/schemas/")
            .unwrap_or(ref_path)
            .to_string();
    }
    match schema["type"].as_str() {
        Some("integer") => {
            let fmt = schema["format"].as_str().unwrap_or("integer");
            fmt.to_string()
        }
        Some("string") => {
            if schema["format"].as_str() == Some("uuid") {
                "uuid".to_string()
            } else {
                "string".to_string()
            }
        }
        Some("boolean") => "boolean".to_string(),
        Some("array") => {
            let item = schema_type_name(&schema["items"], schemas);
            format!("{item}[]")
        }
        Some(t) => t.to_string(),
        None => {
            let resolved = resolve_schema(schema, schemas);
            if resolved != schema {
                schema_type_name(resolved, schemas)
            } else {
                "object".to_string()
            }
        }
    }
}

fn render_schema_fields(schema: &Value, schemas: &Value) -> String {
    // Handle array wrapper
    let schema = if schema["type"].as_str() == Some("array") {
        resolve_schema(&schema["items"], schemas)
    } else {
        schema
    };

    let Some(props) = schema["properties"].as_object() else {
        return String::new();
    };

    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut table = String::from(
        "| Field | Type | Required | Description |\n|-------|------|----------|-------------|\n",
    );
    let mut keys: Vec<_> = props.keys().collect();
    keys.sort();
    for key in keys {
        let prop = &props[key.as_str()];
        let typ = schema_type_name(prop, schemas);
        let req = if required.contains(&key.as_str()) {
            "✓"
        } else {
            "—"
        };
        let desc = prop["description"].as_str().unwrap_or("");
        table += &format!("| `{key}` | {typ} | {req} | {desc} |\n");
    }
    table
}
