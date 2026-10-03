//! The OpenAPI 3.1 document, built from the registry.
//!
//! Paths carry the API prefix and there is no `servers` entry, so clients
//! resolve them against the document's own host.

use std::collections::HashSet;

use serde_json::{Map, Value, json};
use teta_wot_core::{ActionEntry, PropertyEntry, Runtime, ThingHandle};
use teta_wot_td::DataSchema;

use crate::HttpOptions;
use crate::routes::RESERVED_THING_NAMES;

/// The content type of a successful MJPEG response.
const MJPEG: &str = "multipart/x-mixed-replace; boundary=frame";

/// `operationId` for a route: its name and path, with every
/// character that isn't a letter, a digit or `_` replaced by `_`, then
/// `_` and the lower-case method (`generate_unique_id`).
///
/// ```
/// assert_eq!(
///     teta_wot_http::operation_id("get_property", "/mything/foo", "GET"),
///     "get_property_mything_foo_get"
/// );
/// assert_eq!(
///     teta_wot_http::operation_id("things.mything", "/mything/", "GET"),
///     "things_mything_mything__get"
/// );
/// ```
pub fn operation_id(name: &str, path: &str, method: &str) -> String {
    let id: String = format!("{name}{path}")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{id}_{}", method.to_ascii_lowercase())
}

/// The OpenAPI 3.1 document for a runtime's Things, as served at
/// `/openapi.json`. It doesn't depend on the request, so it can be written
/// to a file without a server (the `openapi-export` example).
pub fn openapi(runtime: &Runtime, options: &HttpOptions) -> Value {
    let mut document = Document::new(&options.api_prefix);
    document.framework_routes();
    for thing in runtime.things() {
        if !RESERVED_THING_NAMES.contains(&thing.name()) {
            document.thing(thing);
        }
    }
    let mut schemas = shared_schemas();
    schemas.extend(document.schemas);
    json!({
        "openapi": "3.1.0",
        "info": {"title": options.api_title, "version": options.api_version},
        "paths": document.paths,
        "components": {"schemas": schemas},
    })
}

struct Document {
    prefix: String,
    paths: Map<String, Value>,
    schemas: Map<String, Value>,
    ids: HashSet<String>,
}

fn reference(name: &str) -> Value {
    json!({ "$ref": format!("#/components/schemas/{name}") })
}

fn json_content(schema: Value) -> Value {
    json!({ "application/json": { "schema": schema } })
}

/// 422 answer.
fn validation_error() -> Value {
    json!({
        "description": "Validation Error",
        "content": json_content(reference("HTTPValidationError")),
    })
}

/// A `{uuid}` path parameter.
fn uuid_parameter(name: &str) -> Value {
    json!([{
        "name": name,
        "in": "path",
        "required": true,
        "schema": {"type": "string", "format": "uuid"},
    }])
}

/// default summary: the route's name, in title case.
fn default_summary(name: &str) -> String {
    name.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars.next().map_or_else(String::new, |first| {
                first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Document {
    fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_owned(),
            paths: Map::new(),
            schemas: Map::new(),
            ids: HashSet::new(),
        }
    }

    /// Adds an operation. A duplicate
    /// `operationId` gets `_2`, `_3`, … so
    /// that the document stays valid.
    fn operation(
        &mut self,
        path: &str,
        method: &str,
        name: &str,
        mut operation: Map<String, Value>,
    ) {
        let base = operation_id(name, path, method);
        let mut id = base.clone();
        let mut n = 1;
        while !self.ids.insert(id.clone()) {
            n += 1;
            id = format!("{base}_{n}");
        }
        operation.insert("operationId".into(), Value::String(id));
        let mut ordered = Map::new();
        for key in [
            "summary",
            "description",
            "operationId",
            "parameters",
            "requestBody",
            "responses",
        ] {
            if let Some(value) = operation.remove(key) {
                ordered.insert(key.into(), value);
            }
        }
        ordered.extend(operation);
        self.paths
            .entry(path.to_owned())
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .expect("a path item is an object")
            .insert(method.to_ascii_lowercase(), Value::Object(ordered));
    }

    fn schema(&mut self, name: String, schema: Value) -> Value {
        let reference = reference(&name);
        self.schemas.insert(name, schema);
        reference
    }

    /// The routes of the action manager, the Blob download and the lists
    /// of Things.
    fn framework_routes(&mut self) {
        let p = self.prefix.clone();
        let invocations = format!("{p}/action_invocations");
        let summaries = json!({"type": "array", "items": reference("InvocationSummary")});
        self.operation(&invocations, "GET", "list_all_invocations", op(json!({
            "summary": "List All Invocations",
            "description": "Every invocation the server is keeping, of every action.",
            "responses": {"200": {"description": "Successful Response", "content": json_content(summaries)}},
        })));
        let one = format!("{invocations}/{{id}}");
        self.operation(&one, "GET", "action_invocation", op(json!({
            "summary": "Action Invocation",
            "description": "One invocation: its status, input, output, log and error.",
            "parameters": uuid_parameter("id"),
            "responses": {
                "200": {"description": "Successful Response", "content": json_content(reference("Invocation"))},
                "404": {"description": "Invocation ID not found"},
                "422": validation_error(),
            },
        })));
        self.operation(&one, "DELETE", "delete_invocation", op(json!({
            "summary": "Delete Invocation",
            "description": "Cancel an invocation. The action stops at its next cancellation check.",
            "parameters": uuid_parameter("id"),
            "responses": {
                "200": {"description": "Cancel request sent", "content": json_content(json!({}))},
                "404": {"description": "Invocation ID not found"},
                "503": {"description": "Invocation may not be cancelled"},
                "422": validation_error(),
            },
        })));
        self.operation(&format!("{one}/output"), "GET", "action_invocation_output", op(json!({
            "summary": "Action Invocation Output",
            "description": "The output of a completed invocation. When the output is a Blob, this is its data.",
            "parameters": uuid_parameter("id"),
            "responses": {
                "200": {
                    "description": "Action invocation output",
                    "content": {"application/json": {"schema": {}}, "*/*": {}},
                },
                "404": {"description": "Invocation ID not found"},
                "503": {"description": "No result is available for this invocation"},
                "422": validation_error(),
            },
        })));
        self.operation(
            &format!("{p}/blob/{{blob_id}}"),
            "GET",
            "download_blob",
            op(json!({
                "summary": "Download Blob",
                "description": "A Blob's data, while an invocation or the Thing still holds it.",
                "parameters": uuid_parameter("blob_id"),
                "responses": {
                    "200": {"description": "Successful Response", "content": {"*/*": {}}},
                    "404": {"description": "Blob not found"},
                    "422": validation_error(),
                },
            })),
        );
        self.operation(&format!("{p}/thing_descriptions/"), "GET", "thing_descriptions", op(json!({
            "summary": "Thing Descriptions",
            "description": "The Thing Description of every Thing, by name.",
            "responses": {"200": {
                "description": "Successful Response",
                "content": json_content(json!({"type": "object", "additionalProperties": reference("ThingDescription")})),
            }},
        })));
        self.operation(&format!("{p}/things/"), "GET", "thing_paths", op(json!({
            "summary": "Thing Paths",
            "description": "The URL of every Thing's Thing Description, by name.",
            "responses": {"200": {
                "description": "Successful Response",
                "content": json_content(json!({"type": "object", "additionalProperties": {"type": "string"}})),
            }},
        })));
    }

    /// A Thing's routes: its affordances, streams and endpoints by name, then its TD.
    fn thing(&mut self, thing: &ThingHandle) {
        let name = thing.name().to_owned();
        let base = format!("{}/{name}/", self.prefix);
        enum Item<'a> {
            Property(&'a PropertyEntry),
            Action(&'a ActionEntry),
            Event(&'a teta_wot_core::EventEntry),
            Stream,
            Endpoint(&'a teta_wot_core::EndpointEntry),
        }
        let mut items: Vec<(String, Item<'_>)> = Vec::new();
        items.extend(
            thing
                .properties()
                .map(|p| (p.name().to_owned(), Item::Property(p))),
        );
        items.extend(
            thing
                .actions()
                .map(|a| (a.name().to_owned(), Item::Action(a))),
        );
        items.extend(
            thing
                .events()
                .map(|e| (e.name().to_owned(), Item::Event(e))),
        );
        items.extend(thing.streams().map(|(s, _)| (s.to_owned(), Item::Stream)));
        items.extend(
            thing
                .endpoints()
                .map(|e| (e.name().to_owned(), Item::Endpoint(e))),
        );
        items.sort_by(|a, b| a.0.cmp(&b.0));
        for (item_name, item) in items {
            match item {
                Item::Property(property) => self.property(&name, &base, property),
                Item::Action(action) => self.action(&name, &base, action),
                Item::Event(event) => self.event(&name, &base, event),
                Item::Stream => self.stream(&base, &item_name),
                Item::Endpoint(endpoint) => self.endpoint(&base, endpoint),
            }
        }
        self.operation(&base, "GET", &format!("things.{name}"), op(json!({
            "summary": "Thing Description",
            "description": format!("The W3C Thing Description of `{name}`: its properties, actions and events, and how to use them."),
            "responses": {"200": {"description": "Successful Response", "content": json_content(reference("ThingDescription"))}},
        })));
    }

    fn property(&mut self, thing: &str, base: &str, property: &PropertyEntry) {
        let name = property.name();
        let title = property.title();
        let path = format!("{base}{name}");
        let description = format!(
            "## {title}\n\n{}",
            property.description().unwrap_or_default()
        );
        let value = self.schema(
            format!("{thing}_{name}_value"),
            json_schema(property.data_schema()),
        );
        let mut content = json_content(value.clone());
        if property.is_observable() {
            content.as_object_mut().expect("content is an object").insert(
                "text/event-stream".into(),
                json!({"schema": {
                    "type": "string",
                    "description": "Server-sent events: each new value as JSON in a `data:` line. Ask for them with `Accept: text/event-stream`.",
                }}),
            );
        }
        self.operation(&path, "GET", "get_property", op(json!({
            "summary": title,
            "description": description,
            "responses": {"200": {"description": format!("Value of {name}"), "content": content}},
        })));
        if !property.is_read_only() {
            self.operation(&path, "PUT", "set_property", op(json!({
                "summary": format!("Set {title}"),
                "description": description,
                "requestBody": {"content": json_content(value), "required": true},
                "responses": {
                    "201": {"description": "Property set successfully", "content": json_content(json!({}))},
                    "422": validation_error(),
                },
            })));
        }
        if property.is_resettable() {
            self.operation(&format!("{path}/reset"), "POST", "reset", op(json!({
                "summary": format!("Reset {title}."),
                "description": format!("## Reset {title}\n\nResets the property to its default value, which the Thing Description gives."),
                "responses": {"200": {"description": "Successful Response", "content": json_content(json!({}))}},
            })));
        }
    }

    fn action(&mut self, thing: &str, base: &str, action: &ActionEntry) {
        let name = action.name();
        let title = action.title();
        let path = format!("{base}{name}");
        let input_schema = json_schema(action.input_schema());
        let takes_nothing = accepts_null(&input_schema);
        let input = self.schema(format!("{thing}_{name}_input"), input_schema);
        let output = self.schema(
            format!("{thing}_{name}_output"),
            json_schema(action.output_schema()),
        );
        let invocation = self.schema(
            format!("{thing}_{name}_invocation"),
            invocation_schema(&input, &output),
        );
        self.operation(&path, "GET", "list_invocations", op(json!({
            "summary": format!("All invocations of {name}."),
            "description": format!("List all the invocations of {name} that the server is keeping, with their times and links."),
            "responses": {"200": {
                "description": format!("A list of every invocation of {name}."),
                "content": json_content(json!({"type": "array", "items": reference("InvocationSummary")})),
            }},
        })));
        let mut body = json!({"content": json_content(input)});
        if !takes_nothing {
            body["required"] = json!(true);
        }
        self.operation(&path, "POST", "start_action", op(json!({
            "summary": title,
            "description": format!("## {title}\n\n{}\n\n{ACTION_NOTICE}", action.description().unwrap_or_default()),
            "requestBody": body,
            "responses": {
                "201": {"description": "Action has been invoked (and may still be running).", "content": json_content(invocation)},
                "200": {"description": "Action completed.", "content": json_content(output)},
                "422": validation_error(),
            },
        })));
    }

    fn event(&mut self, thing: &str, base: &str, event: &teta_wot_core::EventEntry) {
        let name = event.name();
        let title = event.title();
        let data = match event.data_schema() {
            Some(schema) => self.schema(format!("{thing}_{name}_data"), json_schema(schema)),
            None => json!({"type": "null"}),
        };
        self.operation(&format!("{base}{name}"), "GET", "subscribe_event", op(json!({
            "summary": title,
            "description": format!(
                "## {title}\n\n{}\n\nServer-sent events: each event's data, as JSON in a `data:` line, following this schema:\n\n```json\n{}\n```",
                event.description().unwrap_or_default(),
                serde_json::to_string_pretty(&data).unwrap_or_default(),
            ),
            "responses": {"200": {
                "description": "Server-sent events",
                "content": {"text/event-stream": {"schema": {"type": "string"}}},
            }},
        })));
    }

    fn stream(&mut self, base: &str, name: &str) {
        let path = format!("{base}{name}");
        self.operation(&path, "GET", "mjpeg_stream_response", op(json!({
            "summary": "Mjpeg Stream Response",
            "description": "An MJPEG stream: each frame as a part of a `multipart/x-mixed-replace` response, which an `<img>` plays.",
            "responses": {"200": {"description": "Successful Response", "content": {MJPEG: {"schema": {"type": "string"}}}}},
        })));
        self.operation(&format!("{path}/viewer"), "GET", "viewer_page", op(json!({
            "summary": "Viewer Page",
            "description": "A page that shows the stream.",
            "responses": {"200": {"description": "Successful Response", "content": {"text/html": {"schema": {"type": "string"}}}}},
        })));
    }

    /// A custom endpoint. Its extractors are axum's, which can't be
    /// described, so only its description and success are.
    fn endpoint(&mut self, base: &str, endpoint: &teta_wot_core::EndpointEntry) {
        let mut response = json!({"description": "Successful Response"});
        if let Some(media_type) = endpoint.link().and_then(|l| l.media_type.as_deref()) {
            response["content"] = json!({ media_type: {} });
        }
        let mut operation = op(json!({
            "summary": default_summary(endpoint.name()),
            "responses": {"200": response},
        }));
        if let Some(description) = endpoint.description() {
            operation.insert("description".into(), json!(description));
        }
        self.operation(
            &format!("{base}{}", endpoint.path()),
            endpoint.method(),
            endpoint.name(),
            operation,
        );
    }
}

const ACTION_NOTICE: &str = "## Important note\n\nThis `POST` request starts an action: the server may carry on after answering. The answer is always a 201 with the invocation, whose `href` can be polled to follow it, and whose `output` link gives the result when it has completed.";

fn op(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// Whether a JSON Schema allows `null` (an action without parameters).
fn accepts_null(schema: &Value) -> bool {
    schema.get("type") == Some(&json!("null"))
        || ["oneOf", "anyOf"].iter().any(|key| {
            schema
                .get(key)
                .and_then(Value::as_array)
                .is_some_and(|branches| branches.iter().any(accepts_null))
        })
}

/// A TD DataSchema as a JSON Schema 2020-12 schema, which OpenAPI 3.1
/// uses: the same terms, except that a tuple's `items` array becomes
/// `prefixItems`.
pub(crate) fn json_schema(schema: &DataSchema) -> Value {
    let mut value = serde_json::to_value(schema).unwrap_or_else(|_| json!({}));
    tuples_to_prefix_items(&mut value);
    value
}

fn tuples_to_prefix_items(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(_)) = map.get("items")
                && let Some(items) = map.shift_remove("items")
            {
                map.insert("prefixItems".into(), items);
            }
            map.values_mut().for_each(tuples_to_prefix_items);
        }
        Value::Array(items) => items.iter_mut().for_each(tuples_to_prefix_items),
        _ => {}
    }
}

/// The properties update for every invocation.
fn summary_properties() -> Map<String, Value> {
    let time = json!({"oneOf": [{"type": "string", "format": "date-time"}, {"type": "null"}]});
    op(json!({
        "status": reference("InvocationStatus"),
        "id": {"type": "string", "format": "uuid"},
        "action": {"type": "string"},
        "href": {"type": "string", "format": "uri"},
        "timeStarted": time,
        "timeRequested": time,
        "timeCompleted": time,
        "links": {"type": "array", "items": reference("LinkElement")},
    }))
}

const SUMMARY_REQUIRED: [&str; 7] = [
    "status",
    "id",
    "action",
    "href",
    "timeStarted",
    "timeRequested",
    "timeCompleted",
];

/// One action's invocation: the summary, with its input, output, log and
/// error.
fn invocation_schema(input: &Value, output: &Value) -> Value {
    let mut properties = summary_properties();
    properties.insert("input".into(), input.clone());
    properties.insert(
        "output".into(),
        json!({"oneOf": [output, {"type": "null"}]}),
    );
    properties.insert(
        "log".into(),
        json!({"type": "array", "items": reference("LogRecordModel")}),
    );
    properties.insert(
        "error".into(),
        json!({"oneOf": [reference("ProblemDetails"), {"type": "null"}]}),
    );
    let mut required: Vec<&str> = SUMMARY_REQUIRED.to_vec();
    required.extend(["input", "log"]);
    json!({"type": "object", "properties": properties, "required": required})
}

/// The schemas every document has.
fn shared_schemas() -> Map<String, Value> {
    let optional_string = json!({"oneOf": [{"type": "string"}, {"type": "null"}]});
    op(json!({
        "InvocationStatus": {
            "type": "string",
            "enum": ["pending", "running", "completed", "cancelled", "error"],
            "description": "Where an invocation is in its life.",
        },
        "InvocationSummary": {
            "type": "object",
            "properties": summary_properties(),
            "required": SUMMARY_REQUIRED,
            "description": "An invocation, as lists show it.",
        },
        "Invocation": invocation_schema(&json!({}), &json!({})),
        "LinkElement": {
            "type": "object",
            "properties": {
                "href": {"type": "string"},
                "type": optional_string,
                "rel": optional_string,
                "anchor": optional_string,
            },
            "required": ["href"],
            "additionalProperties": true,
            "description": "A link: see https://www.w3.org/TR/wot-thing-description11/#link.",
        },
        "LogRecordModel": {
            "type": "object",
            "properties": {
                "message": {"type": "string"},
                "levelname": {"type": "string"},
                "levelno": {"type": "integer"},
                "lineno": {"type": "integer"},
                "filename": {"type": "string"},
                "created": {"type": "string", "format": "date-time"},
                "exception_type": optional_string,
                "traceback": optional_string,
            },
            "required": ["message", "levelname", "levelno", "lineno", "filename", "created"],
            "description": "A message logged during an invocation.",
        },
        "ProblemDetails": {
            "type": "object",
            "properties": {
                "detail": optional_string,
                "type": optional_string,
                "status": {"oneOf": [{"type": "integer"}, {"type": "null"}]},
                "title": optional_string,
                "instance": optional_string,
            },
            "additionalProperties": true,
            "description": "What went wrong in an invocation (RFC 9457 problem details).",
        },
        "HTTPValidationError": {
            "type": "object",
            "properties": {"detail": {"type": "array", "items": reference("ValidationError")}},
        },
        "ValidationError": {
            "type": "object",
            "properties": {
                "loc": {"type": "array", "items": {"oneOf": [{"type": "string"}, {"type": "integer"}]}},
                "msg": {"type": "string"},
                "type": {"type": "string"},
                "input": {},
                "ctx": {"type": "object"},
            },
            "required": ["loc", "msg", "type"],
        },
        "ThingDescription": {
            "type": "object",
            "description": "A W3C Web of Things Thing Description (TD 1.1).",
            "externalDocs": {"url": "https://www.w3.org/TR/wot-thing-description11/"},
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_ids() {
        assert_eq!(
            operation_id("list_all_invocations", "/action_invocations", "GET"),
            "list_all_invocations_action_invocations_get"
        );
        assert_eq!(
            operation_id(
                "action_invocation_output",
                "/action_invocations/{id}/output",
                "GET"
            ),
            "action_invocation_output_action_invocations__id__output_get"
        );
        assert_eq!(
            operation_id("get_property", "/api/v1/counter/counter", "GET"),
            "get_property_api_v1_counter_counter_get"
        );
        assert_eq!(
            operation_id("mjpeg_stream_response", "/blob_probe/stream", "GET"),
            "mjpeg_stream_response_blob_probe_stream_get"
        );
    }

    #[test]
    fn duplicate_operation_ids_are_numbered() {
        let mut document = Document::new("");
        // A Thing `a_b` with a property `c`, and a Thing `a` with `b_c`.
        document.operation("/a_b/c", "GET", "get_property", Map::new());
        document.operation("/a/b_c", "GET", "get_property", Map::new());
        assert_eq!(
            document.paths["/a_b/c"]["get"]["operationId"],
            "get_property_a_b_c_get"
        );
        assert_eq!(
            document.paths["/a/b_c"]["get"]["operationId"],
            "get_property_a_b_c_get_2"
        );
    }

    #[test]
    fn summaries() {
        assert_eq!(default_summary("readings_csv"), "Readings Csv");
        assert_eq!(
            default_summary("mjpeg_stream_response"),
            "Mjpeg Stream Response"
        );
    }

    #[test]
    fn tuples_become_prefix_items() {
        let mut value = json!({"type": "array", "items": [{"type": "integer"}, {"type": "string"}], "properties": {"a": {"items": {"type": "string"}}}});
        tuples_to_prefix_items(&mut value);
        assert_eq!(value["prefixItems"][1]["type"], "string");
        assert_eq!(value.get("items"), None);
        assert_eq!(value["properties"]["a"]["items"]["type"], "string");
    }
}
