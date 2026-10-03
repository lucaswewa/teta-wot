//! The OpenAPI 3.1 document, built from the registry.
//!
//! Paths carry the API prefix and there is no `servers` entry, so clients
//! resolve them against the document's own host.

use std::collections::HashSet;

use serde_json::{Map, Value, json};
use teta_wot_core::{ActionEntry, PropertyEntry, Runtime, ThingHandle};
use teta_wot_td::DataSchema;

use crate::routes::RESERVED_THING_NAMES;
use crate::{HttpOptions, Security, WireProfile};

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
    let mut document = Document::new(&options.api_prefix, options.profile);
    document.framework_routes();
    document.discovery_routes();
    for thing in runtime.things() {
        if !RESERVED_THING_NAMES.contains(&thing.name()) {
            document.thing(thing);
        }
    }
    let mut schemas = shared_schemas();
    let wot_schemas = wot_schemas();
    // Discovery's errors are problems in both profiles.
    schemas.insert("Problem".into(), wot_schemas["Problem"].clone());
    if document.wot() {
        schemas.extend(wot_schemas);
    }
    schemas.extend(document.schemas);
    let mut components = json!({"schemas": schemas});
    if let Some(security) = &options.security {
        let (name, scheme) = match security {
            Security::Basic { .. } => ("basic", "basic"),
            Security::Bearer { .. } => ("bearer", "bearer"),
        };
        components["securitySchemes"] = json!({name: {"type": "http", "scheme": scheme}});
        for item in document.paths.values_mut().filter_map(Value::as_object_mut) {
            for operation in item.values_mut() {
                let public = operation["operationId"]
                    .as_str()
                    .is_some_and(|id| PUBLIC_OPERATIONS.iter().any(|p| id.starts_with(p)));
                if !public {
                    operation["security"] = json!([{ name: [] }]);
                    operation["responses"]["401"] = json!({"description": "Not authenticated"});
                }
            }
        }
    }
    json!({
        "openapi": "3.1.0",
        "info": {"title": options.api_title, "version": options.api_version},
        "paths": document.paths,
        "components": components,
    })
}

/// The operations that need no credentials: descriptions.
const PUBLIC_OPERATIONS: [&str; 5] = [
    "thing_descriptions",
    "thing_paths",
    "things_",
    "well_known_wot",
    "directory_",
];

struct Document {
    prefix: String,
    profile: WireProfile,
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

/// An error answer in the `wot` profile: problem details.
fn problem(description: &str) -> Value {
    json!({
        "description": description,
        "content": {"application/problem+json": {"schema": reference("Problem")}},
    })
}

/// A success without a body (`wot` profile).
fn no_content(description: &str) -> Value {
    json!({ "description": description })
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
    fn new(prefix: &str, profile: WireProfile) -> Self {
        Self {
            prefix: prefix.to_owned(),
            profile,
            paths: Map::new(),
            schemas: Map::new(),
            ids: HashSet::new(),
        }
    }

    fn wot(&self) -> bool {
        self.profile == WireProfile::Wot
    }

    /// How server-sent events are named, in the profile.
    fn named_events(&self) -> &'static str {
        if self.wot() {
            ", in an event named after the affordance, with its time as its `id`"
        } else {
            ""
        }
    }

    /// The answer to invalid input: FastAPI's 422, or the `wot` profile's
    /// 400.
    fn invalid(&self, responses: &mut Value) {
        if self.wot() {
            responses["400"] = problem("Validation Error");
        } else {
            responses["422"] = validation_error();
        }
    }

    /// The answer to an unknown or malformed invocation ID.
    fn unknown_id(&self, responses: &mut Value, description: &str) {
        if self.wot() {
            responses["404"] = problem(description);
        } else {
            responses["404"] = json!({ "description": description });
            responses["422"] = validation_error();
        }
    }

    /// Adds an operation, named as FastAPI names it. A duplicate
    /// `operationId` (possible with FastAPI's rule) gets `_2`, `_3`, … so
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
        let (summary, full) = if self.wot() {
            ("ActionStatus", "ActionStatus")
        } else {
            ("InvocationSummary", "Invocation")
        };
        let summaries = json!({"type": "array", "items": reference(summary)});
        self.operation(&invocations, "GET", "list_all_invocations", op(json!({
            "summary": "List All Invocations",
            "description": "Every invocation the server is keeping, of every action.",
            "responses": {"200": {"description": "Successful Response", "content": json_content(summaries)}},
        })));
        let one = format!("{invocations}/{{id}}");
        let mut responses = json!({
            "200": {"description": "Successful Response", "content": json_content(reference(full))},
        });
        self.unknown_id(&mut responses, "Invocation ID not found");
        self.operation(
            &one,
            "GET",
            "action_invocation",
            op(json!({
                "summary": "Action Invocation",
                "description": if self.wot() {
                    "One invocation's `ActionStatus` (`queryaction`)."
                } else {
                    "One invocation: its status, input, output, log and error."
                },
                "parameters": uuid_parameter("id"),
                "responses": responses,
            })),
        );
        let mut responses = if self.wot() {
            json!({
                "204": no_content("Cancel request sent"),
                "409": problem("Invocation may not be cancelled"),
            })
        } else {
            json!({
                "200": {"description": "Cancel request sent", "content": json_content(json!({}))},
                "503": {"description": "Invocation may not be cancelled"},
            })
        };
        self.unknown_id(&mut responses, "Invocation ID not found");
        self.operation(&one, "DELETE", "delete_invocation", op(json!({
            "summary": "Delete Invocation",
            "description": "Cancel an invocation (`cancelaction`). The action stops at its next cancellation check.",
            "parameters": uuid_parameter("id"),
            "responses": responses,
        })));
        let mut responses = json!({
            "200": {
                "description": "Action invocation output",
                "content": {"application/json": {"schema": {}}, "*/*": {}},
            },
        });
        if self.wot() {
            responses["404"] = problem("Invocation ID not found, or no output is available");
        } else {
            responses["503"] = json!({"description": "No result is available for this invocation"});
            self.unknown_id(&mut responses, "Invocation ID not found");
        }
        self.operation(&format!("{one}/output"), "GET", "action_invocation_output", op(json!({
            "summary": "Action Invocation Output",
            "description": "The output of a completed invocation. When the output is a Blob, this is its data.",
            "parameters": uuid_parameter("id"),
            "responses": responses,
        })));
        let mut responses = json!({
            "200": {"description": "Successful Response", "content": {"*/*": {}}},
        });
        self.unknown_id(&mut responses, "Blob not found");
        self.operation(
            &format!("{p}/blob/{{blob_id}}"),
            "GET",
            "download_blob",
            op(json!({
                "summary": "Download Blob",
                "description": "A Blob's data, while an invocation or the Thing still holds it.",
                "parameters": uuid_parameter("blob_id"),
                "responses": responses,
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

    /// Discovery: the well-known URL and the directory.
    fn discovery_routes(&mut self) {
        let problems = |description: &str| json!({"content": {"application/problem+json": {"schema": reference("Problem")}}, "description": description});
        self.operation("/.well-known/wot", "GET", "well_known_wot", op(json!({
            "summary": "Thing Description Directory",
            "description": "The TD of this server's Thing Description Directory (W3C WoT Discovery).",
            "responses": {"200": {
                "description": "Successful Response",
                "content": {"application/td+json": {"schema": reference("ThingDescription")}},
            }},
        })));
        let things = format!("{}/directory/things", self.prefix);
        let number = |name: &str, description: &str| json!({"name": name, "in": "query", "required": false, "description": description, "schema": {"type": "integer", "minimum": 0}});
        self.operation(&things, "GET", "directory_things", op(json!({
            "summary": "Thing Descriptions",
            "description": "Every Thing's TD, sorted by `id`. With `limit`, a page, with `next` and `canonical` links.",
            "parameters": [
                number("offset", "How many TDs to skip"),
                number("limit", "How many TDs in a page (at least 1)"),
                {"name": "format", "in": "query", "required": false, "schema": {"type": "string", "enum": ["array", "collection"], "default": "array"}},
            ],
            "responses": {
                "200": {
                    "description": "Successful Response",
                    "content": {"application/ld+json": {"schema": {"oneOf": [
                        {"type": "array", "items": reference("ThingDescription")},
                        {"type": "object", "description": "A ThingCollection"},
                    ]}}},
                },
                "400": problems("Invalid query arguments"),
            },
        })));
        self.operation(&format!("{things}/{{id}}"), "GET", "directory_thing", op(json!({
            "summary": "Thing Description",
            "description": "One TD, by its `id`.",
            "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "iri-reference"}}],
            "responses": {
                "200": {
                    "description": "Successful Response",
                    "content": {"application/td+json": {"schema": reference("ThingDescription")}},
                },
                "404": problems("No TD has this ID"),
            },
        })));
    }

    /// A Thing's top-level resources, where its TD has their forms.
    fn top_level(&mut self, thing: &ThingHandle, base: &str) {
        let name = thing.name();
        let path = format!("{base}properties");
        if thing.properties().next().is_some() {
            let mut content = json_content(
                json!({"type": "object", "description": "Each property's value, by name."}),
            );
            content["text/event-stream"] = json!({"schema": {
                "type": "string",
                "description": "Server-sent events: each observable property's new values, as events named after it. Ask for them with `Accept: text/event-stream`.",
            }});
            self.operation(&path, "GET", "read_all_properties", op(json!({
                "summary": "Read All Properties",
                "description": format!("Every property of `{name}` (`readallproperties`), or their changes (`observeallproperties`)."),
                "responses": {"200": {"description": "Successful Response", "content": content}},
            })));
        }
        if thing.properties().any(|p| !p.is_read_only()) {
            let mut responses = json!({"204": no_content("Properties written")});
            self.invalid(&mut responses);
            self.operation(&path, "PUT", "write_multiple_properties", op(json!({
                "summary": "Write Multiple Properties",
                "description": format!("Writes several properties of `{name}` (`writemultipleproperties`): nothing is written unless every value is valid."),
                "requestBody": {"content": json_content(json!({"type": "object"})), "required": true},
                "responses": responses,
            })));
        }
        if thing.actions().next().is_some() {
            let item = if self.wot() {
                "ActionStatus"
            } else {
                "InvocationSummary"
            };
            self.operation(&format!("{base}actions"), "GET", "query_all_actions", op(json!({
                "summary": "Query All Actions",
                "description": format!("The invocations of each action of `{name}`, newest first (`queryallactions`)."),
                "responses": {"200": {
                    "description": "Successful Response",
                    "content": json_content(json!({"type": "object", "additionalProperties": {"type": "array", "items": reference(item)}})),
                }},
            })));
        }
        if thing.events().next().is_some() {
            self.operation(&format!("{base}events"), "GET", "subscribe_all_events", op(json!({
                "summary": "Subscribe All Events",
                "description": format!("Server-sent events: every event of `{name}`, named after it (`subscribeallevents`)."),
                "responses": {"200": {
                    "description": "Server-sent events",
                    "content": {"text/event-stream": {"schema": {"type": "string"}}},
                }},
            })));
        }
    }

    /// A Thing's routes: its affordances, streams and endpoints by name, its
    /// top-level resources, then its TD.
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
        self.top_level(thing, &base);
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
                    "description": format!("Server-sent events: each new value as JSON in a `data:` line{}. Ask for them with `Accept: text/event-stream`.", self.named_events()),
                }}),
            );
        }
        self.operation(&path, "GET", "get_property", op(json!({
            "summary": title,
            "description": description,
            "responses": {"200": {"description": format!("Value of {name}"), "content": content}},
        })));
        if !property.is_read_only() {
            let mut responses = if self.wot() {
                json!({"204": no_content("Property set successfully")})
            } else {
                json!({"201": {"description": "Property set successfully", "content": json_content(json!({}))}})
            };
            self.invalid(&mut responses);
            self.operation(
                &path,
                "PUT",
                "set_property",
                op(json!({
                    "summary": format!("Set {title}"),
                    "description": description,
                    "requestBody": {"content": json_content(value), "required": true},
                    "responses": responses,
                })),
            );
        }
        if property.is_resettable() {
            let responses = if self.wot() {
                json!({"204": no_content("Successful Response")})
            } else {
                json!({"200": {"description": "Successful Response", "content": json_content(json!({}))}})
            };
            self.operation(&format!("{path}/reset"), "POST", "reset", op(json!({
                "summary": format!("Reset {title}."),
                "description": format!("## Reset {title}\n\nResets the property to its default value, which the Thing Description gives."),
                "responses": responses,
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
        let invocation = if self.wot() {
            self.schema(
                format!("{thing}_{name}_status"),
                action_status_schema(&output),
            )
        } else {
            self.schema(
                format!("{thing}_{name}_invocation"),
                invocation_schema(&input, &output),
            )
        };
        let item = if self.wot() {
            "ActionStatus"
        } else {
            "InvocationSummary"
        };
        self.operation(&path, "GET", "list_invocations", op(json!({
            "summary": format!("All invocations of {name}."),
            "description": format!("List all the invocations of {name} that the server is keeping, with their times and links."),
            "responses": {"200": {
                "description": format!("A list of every invocation of {name}."),
                "content": json_content(json!({"type": "array", "items": reference(item)})),
            }},
        })));
        let mut body = json!({"content": json_content(input)});
        if !takes_nothing {
            body["required"] = json!(true);
        }
        let started = json!({"description": "Action has been invoked (and may still be running).", "content": json_content(invocation)});
        let (notice, mut responses) = match (self.wot(), action.is_synchronous()) {
            (false, _) => (
                ACTION_NOTICE,
                json!({
                    "201": started,
                    "200": {"description": "Action completed.", "content": json_content(output)},
                }),
            ),
            (true, false) => (ASYNCHRONOUS_NOTICE, json!({"201": started})),
            (true, true) if action.has_output() => (
                SYNCHRONOUS_NOTICE,
                json!({
                    "200": {"description": "Action completed", "content": json_content(output)},
                    "500": problem("Action failed"),
                    "503": problem("Action unavailable"),
                }),
            ),
            (true, true) => (
                SYNCHRONOUS_NOTICE,
                json!({
                    "204": no_content("Action completed"),
                    "500": problem("Action failed"),
                    "503": problem("Action unavailable"),
                }),
            ),
        };
        self.invalid(&mut responses);
        self.operation(&path, "POST", "start_action", op(json!({
            "summary": title,
            "description": format!("## {title}\n\n{}\n\n{notice}", action.description().unwrap_or_default()),
            "requestBody": body,
            "responses": responses,
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
                "## {title}\n\n{}\n\nServer-sent events: each event's data, as JSON in a `data:` line{}, following this schema:\n\n```json\n{}\n```",
                event.description().unwrap_or_default(),
                self.named_events(),
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

const ASYNCHRONOUS_NOTICE: &str = "## Important note\n\nThis `POST` request starts an action: the server may carry on after answering. The answer is a 201 with the invocation's `ActionStatus`, whose `href` (also in `Location`) can be polled to follow it; its `output` is there once it has completed.";

const SYNCHRONOUS_NOTICE: &str = "## Important note\n\nThis action is synchronous: the answer comes when it has finished, with its output.";

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

/// One action's `ActionStatus` (`wot` profile), with its output.
fn action_status_schema(output: &Value) -> Value {
    let mut schema = wot_schemas()["ActionStatus"].clone();
    schema["properties"]["output"] = output.clone();
    schema
}

/// The schemas of the `wot` profile (`Problem` is also in the other, for
/// discovery).
fn wot_schemas() -> Map<String, Value> {
    let time = json!({"type": "string", "format": "date-time"});
    op(json!({
        "ActionStatus": {
            "type": "object",
            "properties": {
                "status": {"type": "string", "enum": ["pending", "running", "completed", "failed"]},
                "output": {},
                "error": reference("Problem"),
                "href": {"type": "string", "format": "uri"},
                "timeRequested": time,
                "timeEnded": time,
            },
            "required": ["status", "href", "timeRequested"],
            "description": "An invocation, as the W3C WoT HTTP Basic Profile describes it.",
        },
        "Problem": {
            "type": "object",
            "properties": {
                "type": {"type": "string", "format": "uri-reference"},
                "title": {"type": "string"},
                "status": {"type": "integer"},
                "detail": {"type": "string"},
                "instance": {"type": "string"},
                "invalid-params": {"type": "array", "items": {
                    "type": "object",
                    "properties": {
                        "in": {"type": "string"},
                        "name": {"type": "string"},
                        "reason": {"type": "string"},
                        "code": {"type": "string"},
                    },
                }},
            },
            "additionalProperties": true,
            "description": "Problem details (RFC 9457). The framework's types are documented at https://github.com/lucaswewa/wot/blob/main/docs/problems.md.",
        },
    }))
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
        let mut document = Document::new("", WireProfile::TetaThing);
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
