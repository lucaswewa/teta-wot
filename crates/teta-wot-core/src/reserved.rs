// Affordance naming rules. This file is also `include!`d by
// `teta-wot-macros`, so that the macros check the same rules at compile time: it
// must stay self-contained (no `use`, no crate paths).

/// Affordance names that a Thing may not use, because the server needs the
/// URL `/{thing}/{name}` for something else: `ws` is WebSocket
/// endpoint.
pub const RESERVED_AFFORDANCE_NAMES: &[&str] = &["ws"];

/// Why `name` can't be the name of an affordance, if it can't: it must be
/// usable as one URL path segment (letters, digits, `_` and `-`) and not be
/// reserved. `properties`, `actions` and `events` are the Thing's
/// top-level resources, such as `readallproperties`
pub fn affordance_name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("an affordance name can't be empty".to_owned());
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Some(format!(
            "`{name}` can't be an affordance name: names may only contain letters, digits, `_` and `-`"
        ));
    }
    if RESERVED_AFFORDANCE_NAMES.contains(&name) {
        return Some(format!(
            "`{name}` is reserved: the server uses `/{{thing}}/{name}` for something else"
        ));
    }
    None
}
