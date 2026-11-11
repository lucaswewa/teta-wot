//! Who is calling: the principal an authenticator found for a request.

use std::future::Future;

use serde_json::{Map, Value};

use crate::context::InvocationScope;

tokio::task_local! {
    static REQUEST: Principal;
}

/// Who is calling: the identity an authenticator gave a request, and any
/// attributes it attached, such as roles. Without security, everyone is
/// anonymous.
///
/// Code that a request runs can ask for its principal with
/// [`Principal::current`]: actions (and their child invocations and
/// blocking code), property getters and setters, and custom endpoints.
/// Actions also have [`ActionCtx::principal`](crate::ActionCtx::principal).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Principal {
    id: Option<String>,
    attributes: Map<String, Value>,
}

impl Principal {
    /// Nobody in particular: a request without security.
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// A principal with an identity, such as a user name.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: Some(id.into()),
            attributes: Map::new(),
        }
    }

    /// Adds an attribute, such as `role`.
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    /// The identity, or `None` if anonymous.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// An attribute, if it was set.
    pub fn attribute(&self, name: &str) -> Option<&Value> {
        self.attributes.get(name)
    }

    /// Whether this is the anonymous principal.
    pub fn is_anonymous(&self) -> bool {
        self.id.is_none()
    }

    /// The principal of the code that is running: its invocation's, or else
    /// its request's, or else anonymous.
    pub fn current() -> Self {
        InvocationScope::current()
            .map(|scope| scope.principal().clone())
            .or_else(|| REQUEST.try_with(Clone::clone).ok())
            .unwrap_or_default()
    }

    /// Runs `future` as this principal: [`current`](Self::current) returns
    /// it, and invocations it starts carry it.
    pub fn scope<F: Future>(self, future: F) -> impl Future<Output = F::Output> {
        REQUEST.scope(self, future)
    }

    /// Runs blocking code as this principal.
    pub fn sync_scope<R>(self, f: impl FnOnce() -> R) -> R {
        REQUEST.sync_scope(self, f)
    }
}
