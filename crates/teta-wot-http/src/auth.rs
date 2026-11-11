//! Pluggable authentication and authorisation.
//!
//! - **An [`Authenticator`]** says who sent a request, as a [`Principal`].
//! - **An [`Authorizer`]** says whether that principal may make it, from
//!   the request's [`Access`]: which Thing, which affordance, which
//!   operation, and whether it changes state.
//!
//! Both are optional: [`Security`] (HTTP Basic or a bearer token) is an
//! authenticator too.
//!
//! - **Not authenticated:** `401`, with the authenticator's challenge.
//! - **Not authorised:** `403`.
//! - **Public routes** (TDs, discovery, the docs) are never checked.
//!
//! The code a request runs gets the principal from [`Principal::current`].

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use futures_util::future::BoxFuture;
use http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use http::request::Parts;
use http::{HeaderValue, Method, StatusCode, Uri};
use serde_json::Value;
use teta_wot_core::Principal;
use teta_wot_td::SecurityScheme;

use crate::problem::HttpError;
use crate::{HttpOptions, Security, WireProfile, security};

/// Says who sent a request.
pub trait Authenticator: Send + Sync + 'static {
    /// The principal, from the request's parts (usually its `Authorization`
    /// header), or why the request is refused.
    fn authenticate<'a>(
        &'a self,
        request: &'a Parts,
    ) -> BoxFuture<'a, Result<Principal, Rejection>>;

    /// The security scheme, with its name, that the Thing Descriptions
    /// describe. `None` describes none.
    fn security_scheme(&self) -> Option<(String, SecurityScheme)> {
        None
    }
}

/// Says whether a principal may make a request.
pub trait Authorizer: Send + Sync + 'static {
    /// Whether `principal` may do `access`. `false` is a `403`.
    fn authorize(&self, principal: &Principal, access: &Access<'_>) -> bool;
}

/// What a request asks to do, for an [`Authorizer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Access<'a> {
    /// The Thing, if the request is for one.
    pub thing: Option<&'a str>,
    /// The property, action, event, stream or endpoint path (for a merged
    /// route, its whole path), if any.
    pub affordance: Option<&'a str>,
    /// What kind of request it is.
    pub operation: Operation,
    /// Whether it can change state: writes, actions, cancellations, and
    /// endpoints and routes with other methods than `GET` and `HEAD`.
    pub changes_state: bool,
}

/// The kind of a request, in an [`Access`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Operation {
    /// Reading one property, or all of them.
    ReadProperty,
    /// Writing or resetting properties.
    WriteProperty,
    /// Invoking an action.
    InvokeAction,
    /// Listing or following invocations.
    QueryAction,
    /// Cancelling an invocation.
    CancelAction,
    /// Observing properties or subscribing to events (SSE or WebSocket).
    Observe,
    /// An MJPEG stream or its viewer.
    ReadStream,
    /// Downloading a Blob.
    DownloadBlob,
    /// A custom endpoint, binary streams included.
    Endpoint,
    /// A route merged into the server.
    Route,
}

/// Why a request is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    status: StatusCode,
    challenge: Option<String>,
    detail: String,
}

impl Rejection {
    /// `401`: not authenticated. `challenge` is the `WWW-Authenticate`
    /// header's value, such as `Bearer realm="lab"`.
    pub fn unauthorized(challenge: Option<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            challenge,
            detail: "Not authenticated".to_owned(),
        }
    }

    /// `403`: authenticated, but not allowed.
    pub fn forbidden(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            challenge: None,
            detail: detail.into(),
        }
    }

    pub(crate) fn response(&self, profile: WireProfile) -> Response {
        let mut response = HttpError::Status(self.status, self.detail.clone()).response(profile);
        if let Some(value) = self
            .challenge
            .as_deref()
            .and_then(|c| HeaderValue::from_str(c).ok())
        {
            response.headers_mut().insert(WWW_AUTHENTICATE, value);
        }
        response
    }
}

/// The application's authenticator and authoriser, in
/// [`HttpOptions::auth`]. Either may be left out: without an authenticator,
/// [`HttpOptions::security`] authenticates, if set, and otherwise everyone
/// is anonymous.
#[derive(Clone, Default)]
pub struct Auth {
    authenticator: Option<Arc<dyn Authenticator>>,
    authorizer: Option<Arc<dyn Authorizer>>,
}

impl Auth {
    /// Sets the authenticator.
    #[must_use]
    pub fn authenticator(mut self, authenticator: impl Authenticator) -> Self {
        self.authenticator = Some(Arc::new(authenticator));
        self
    }

    /// Sets the authoriser.
    #[must_use]
    pub fn authorizer(mut self, authorizer: impl Authorizer) -> Self {
        self.authorizer = Some(Arc::new(authorizer));
        self
    }
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Auth")
            .field("authenticator", &self.authenticator.is_some())
            .field("authorizer", &self.authorizer.is_some())
            .finish()
    }
}

impl PartialEq for Auth {
    fn eq(&self, other: &Self) -> bool {
        fn same<T: ?Sized>(a: Option<&Arc<T>>, b: Option<&Arc<T>>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (a, b) => a.is_none() && b.is_none(),
            }
        }
        same(self.authenticator.as_ref(), other.authenticator.as_ref())
            && same(self.authorizer.as_ref(), other.authorizer.as_ref())
    }
}

impl Eq for Auth {}

impl Authenticator for Security {
    fn authenticate<'a>(
        &'a self,
        request: &'a Parts,
    ) -> BoxFuture<'a, Result<Principal, Rejection>> {
        Box::pin(async move {
            if !security::authorized(self, &request.headers) {
                return Err(Rejection::unauthorized(Some(security::challenge(self))));
            }
            Ok(match self {
                Security::Basic { username, .. } => Principal::new(username.clone()),
                Security::Bearer { .. } => Principal::new("bearer"),
            })
        })
    }

    fn security_scheme(&self) -> Option<(String, SecurityScheme)> {
        Some(security::scheme(self))
    }
}

/// Everyone is anonymous: for an authoriser without an authenticator.
struct Anonymous;

impl Authenticator for Anonymous {
    fn authenticate<'a>(&'a self, _: &'a Parts) -> BoxFuture<'a, Result<Principal, Rejection>> {
        Box::pin(async { Ok(Principal::anonymous()) })
    }
}

/// The checks a server makes, resolved from its options; `None` from
/// [`Checker::from_options`] if it makes none.
#[derive(Clone)]
pub(crate) struct Checker {
    authenticator: Arc<dyn Authenticator>,
    authorizer: Option<Arc<dyn Authorizer>>,
    profile: WireProfile,
}

impl Checker {
    pub(crate) fn from_options(options: &HttpOptions) -> Option<Self> {
        let security = options.security.clone();
        let authenticator = options
            .auth
            .authenticator
            .clone()
            .or_else(|| security.map(|s| Arc::new(s) as Arc<dyn Authenticator>));
        let authenticator = match (authenticator, &options.auth.authorizer) {
            (Some(authenticator), _) => authenticator,
            (None, Some(_)) => Arc::new(Anonymous),
            (None, None) => return None,
        };
        Some(Self {
            authenticator,
            authorizer: options.auth.authorizer.clone(),
            profile: options.profile,
        })
    }

    /// Authenticates, then authorises.
    pub(crate) async fn check(
        &self,
        parts: &Parts,
        access: &Access<'_>,
    ) -> Result<Principal, Rejection> {
        let principal = self.authenticator.authenticate(parts).await?;
        match &self.authorizer {
            Some(authorizer) if !authorizer.authorize(&principal, access) => {
                Err(Rejection::forbidden("Not allowed"))
            }
            _ => Ok(principal),
        }
    }
}

/// The security scheme the TDs describe.
pub(crate) fn security_scheme(options: &HttpOptions) -> Option<(String, SecurityScheme)> {
    match &options.auth.authenticator {
        Some(authenticator) => authenticator.security_scheme(),
        None => options.security.as_ref().map(security::scheme),
    }
}

/// Whether a request changes nothing, by its method.
pub(crate) fn is_safe(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// The check, as a layer, for routes merged into the server, which don't go
/// through the Things' dispatch.
pub(crate) async fn guard(
    State(checker): State<Checker>,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();
    let access = Access {
        thing: None,
        affordance: Some(parts.uri.path()),
        operation: Operation::Route,
        changes_state: !is_safe(&parts.method),
    };
    match checker.check(&parts, &access).await {
        Ok(principal) => {
            principal
                .scope(next.run(Request::from_parts(parts, body)))
                .await
        }
        Err(refused) => refused.response(checker.profile),
    }
}

/// How long a WebSocket has to send its first message, when it
/// authenticates with it.
const FIRST_MESSAGE_TIMEOUT: Duration = Duration::from_secs(10);

/// The check a WebSocket makes on its first message, when its upgrade
/// request had no credentials: browsers can't add headers to WebSockets.
///
/// An endpoint marked with `EndpointSpec::websocket` gets one as a request
/// extension (`Option<Extension<FirstMessageAuth>>`) when the upgrade request
/// had no `Authorization` header, and must then call
/// [`authenticate`](Self::authenticate) before serving the socket. Without
/// one, the request was checked as usual. Binary streams do this.
#[derive(Clone)]
pub struct FirstMessageAuth {
    checker: Checker,
    uri: Uri,
    thing: Option<String>,
    affordance: Option<String>,
    operation: Operation,
}

impl fmt::Debug for FirstMessageAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FirstMessageAuth")
            .field("uri", &self.uri)
            .finish_non_exhaustive()
    }
}

impl FirstMessageAuth {
    pub(crate) fn new(checker: &Checker, parts: &Parts, access: &Access<'_>) -> Self {
        Self {
            checker: checker.clone(),
            uri: parts.uri.clone(),
            thing: access.thing.map(str::to_owned),
            affordance: access.affordance.map(str::to_owned),
            operation: access.operation,
        }
    }

    /// Reads the first message, `{"authorization": "Bearer …"}`, and checks
    /// it as the upgrade request's `Authorization` header. If that fails,
    /// or nothing arrives in time, it closes the socket (1008, a policy
    /// violation) and returns `None`. Serve the socket as the principal, in
    /// [`Principal::scope`].
    pub async fn authenticate(&self, socket: &mut WebSocket) -> Option<Principal> {
        let header = match tokio::time::timeout(FIRST_MESSAGE_TIMEOUT, socket.recv()).await {
            Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("authorization")?.as_str().map(str::to_owned)),
            _ => None,
        };
        let request = header.and_then(|header| {
            let request = Request::get(self.uri.clone()).header(AUTHORIZATION, header);
            request.body(()).ok().map(|r| r.into_parts().0)
        });
        let access = Access {
            thing: self.thing.as_deref(),
            affordance: self.affordance.as_deref(),
            operation: self.operation,
            changes_state: false,
        };
        let principal = match &request {
            Some(parts) => self.checker.check(parts, &access).await.ok(),
            None => None,
        };
        if principal.is_none() {
            let close = CloseFrame {
                code: 1008,
                reason: "not authenticated".into(),
            };
            let _ = socket.send(Message::Close(Some(close))).await;
        }
        principal
    }
}
