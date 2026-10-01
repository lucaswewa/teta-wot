//! Public HTTP options, route setup, and stable Thing identifiers.

use std::sync::Arc;

use teta_wot_core::{Runtime, Thing, ThingDefinition};
use teta_wot_http::{
    HttpOptions, RESERVED_THING_NAMES, RouteError, default_server_id, router, td_id,
};
use uuid::Uuid;

struct Empty;
impl Thing for Empty {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Empty")
    }
}

fn options(prefix: &str) -> HttpOptions {
    HttpOptions {
        api_prefix: prefix.into(),
        server_id: "test-server".into(),
    }
}

#[test]
fn default_options_use_no_prefix_and_the_default_server_identity() {
    let defaults = HttpOptions::default();
    assert!(defaults.api_prefix.is_empty());
    assert_eq!(defaults.server_id, default_server_id());
}

#[test]
fn valid_prefixes_build_a_router() {
    for prefix in ["", "/api", "/api/v1", "/api_v1/test-server", "/1"] {
        let runtime = Arc::new(Runtime::builder().build().unwrap());
        assert!(router(runtime, options(prefix)).is_ok(), "{prefix}");
    }
}

#[test]
fn invalid_prefixes_are_reported_without_panicking() {
    for prefix in [
        "api",
        "/",
        "/api/",
        "//api",
        "/api//v1",
        "/a.b",
        "/with space",
        "/api?x=1",
        "/api#fragment",
    ] {
        let runtime = Arc::new(Runtime::builder().build().unwrap());
        assert!(
            matches!(router(runtime, options(prefix)), Err(RouteError::InvalidPrefix(actual)) if actual == prefix)
        );
    }
}

#[test]
fn reserved_names_are_rejected_by_the_http_binding() {
    for name in RESERVED_THING_NAMES {
        let runtime = Arc::new(Runtime::builder().thing(name, Empty).build().unwrap());
        assert!(
            matches!(router(runtime, options("/api")), Err(RouteError::ReservedThingName(actual)) if actual == name)
        );
    }
    for name in ["things_1", "my-docs", "counter"] {
        let runtime = Arc::new(Runtime::builder().thing(name, Empty).build().unwrap());
        assert!(router(runtime, options("")).is_ok());
    }
}

#[test]
fn thing_ids_are_stable_uuid_v5_urns_and_distinguish_servers_and_things() {
    let id = td_id("server", "counter");
    assert_eq!(id, td_id("server", "counter"));
    let uuid = Uuid::parse_str(id.strip_prefix("urn:uuid:").unwrap()).unwrap();
    assert_eq!(uuid.get_version_num(), 5);
    assert_ne!(id, td_id("other-server", "counter"));
    assert_ne!(id, td_id("server", "other-thing"));
}
