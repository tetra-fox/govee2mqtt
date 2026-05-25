//! A small MQTT topic router with axum-style extractors.
//!
//! rumqttc delivers raw Publish packets; it has no routing layer. This module
//! provides one: you register routes with `:param` placeholders (the same shape
//! the publish-side topic helpers produce), each route subscribes to the
//! corresponding MQTT filter (`:param` becomes `+`), and incoming messages are
//! matched back to a handler with the path parameters bound.
//!
//! Handlers declare their inputs as extractor arguments (`Payload`, `Params`,
//! `State`) and the router applies them in order, mirroring how axum handlers
//! work.

use rumqttc::{AsyncClient, QoS};
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RouterError {
    #[error("payload is not utf8, cannot parse into string")]
    PayloadIsNotUtf8,
    #[error("failed to parse payload {text}: {error}")]
    PayloadParseFailed { text: String, error: String },
    #[error("no route matches topic {0}")]
    NoMatchingRoute(String),
    #[error(transparent)]
    Client(#[from] rumqttc::ClientError),
    #[error(transparent)]
    JsonError(#[from] serde_json::Error),
    #[error(transparent)]
    Any(#[from] anyhow::Error),
}

pub type RouterResult<T> = Result<T, RouterError>;
pub type MqttHandlerResult = anyhow::Result<()>;

/// An incoming MQTT message: the topic it arrived on and the raw payload bytes.
pub struct Message {
    pub topic: String,
    pub payload: Vec<u8>,
}

/// The context handed to extractors: the parsed path parameters, the message,
/// and the shared application state.
pub struct Request<S> {
    params: JsonValue,
    message: Message,
    state: S,
}

/// Parse and extract one piece of information from a Request.
pub trait FromRequest<S>: Sized {
    fn from_request(request: &Request<S>) -> RouterResult<Self>;
}

/// Extracts the message payload and parses it via `FromStr` into `T`.
pub struct Payload<T>(pub T);

impl<S, T> FromRequest<S> for Payload<T>
where
    T: FromStr,
    <T as FromStr>::Err: std::fmt::Debug,
{
    fn from_request(request: &Request<S>) -> RouterResult<Payload<T>> {
        let s = std::str::from_utf8(&request.message.payload)
            .map_err(|_| RouterError::PayloadIsNotUtf8)?;
        let result: T = s.parse().map_err(|err| RouterError::PayloadParseFailed {
            text: s.to_string(),
            error: format!("{err:#?}"),
        })?;
        Ok(Self(result))
    }
}

/// Extracts the `:param` values bound from the topic and deserializes them into
/// `T`. The parameter map is a JSON object of string values, so `T`'s fields
/// are typically `String`.
pub struct Params<T>(pub T);

impl<S, T> FromRequest<S> for Params<T>
where
    T: DeserializeOwned,
{
    fn from_request(request: &Request<S>) -> RouterResult<Params<T>> {
        let parsed: T = serde_json::from_value(request.params.clone())?;
        Ok(Self(parsed))
    }
}

/// Extracts a clone of the shared application state.
pub struct State<S>(pub S);

impl<S> FromRequest<S> for State<S>
where
    S: Clone + Send + Sync,
{
    fn from_request(request: &Request<S>) -> RouterResult<State<S>> {
        Ok(Self(request.state.clone()))
    }
}

/// The future returned by a dispatch closure.
type HandlerFuture = Pin<Box<dyn Future<Output = MqttHandlerResult> + Send>>;

/// The type-erased dispatch closure: takes a request, returns the handler's
/// future.
type DispatchFn<S> = Box<dyn Fn(Request<S>) -> HandlerFuture + Send + Sync>;

/// A type-erased handler. Built from a function via `MakeDispatcher`. Public
/// because it appears in the `MakeDispatcher` trait that bounds `route`, but its
/// fields and construction stay private to this module.
pub struct Dispatcher<S> {
    func: DispatchFn<S>,
}

impl<S: Clone + Send + Sync + 'static> Dispatcher<S> {
    async fn call(&self, params: JsonValue, message: Message, state: S) -> MqttHandlerResult {
        (self.func)(Request {
            params,
            message,
            state,
        })
        .await
    }
}

/// Adapts a handler function (taking extractor arguments) into a `Dispatcher`.
pub trait MakeDispatcher<T, S: Clone + Send + Sync> {
    fn make_dispatcher(func: Self) -> Dispatcher<S>;
}

macro_rules! impl_make_dispatcher {
    (
        [$($ty:ident),*], $last:ident
    ) => {

impl<F, S, Fut, $($ty,)* $last> MakeDispatcher<($($ty,)* $last,), S> for F
where
    F: (Fn($($ty,)* $last) -> Fut) + Send + Sync + 'static,
    Fut: Future<Output = MqttHandlerResult> + Send,
    S: Clone + Send + Sync + 'static,
    $( $ty: FromRequest<S>, )*
    $last: FromRequest<S>
{
    #[allow(non_snake_case)]
    fn make_dispatcher(func: F) -> Dispatcher<S> {
        let func = Arc::new(func);
        let wrap: DispatchFn<S> =
            Box::new(move |request: Request<S>| {
                let func = func.clone();
                Box::pin(async move {
                    $(
                    let $ty = $ty::from_request(&request)?;
                    )*
                    let $last = $last::from_request(&request)?;
                    func($($ty,)* $last).await
                })
            });

        Dispatcher { func: wrap }
    }
}

    }
}

#[rustfmt::skip]
macro_rules! all_the_tuples {
    ($name:ident) => {
        $name!([], T1);
        $name!([T1], T2);
        $name!([T1, T2], T3);
        $name!([T1, T2, T3], T4);
        $name!([T1, T2, T3, T4], T5);
        $name!([T1, T2, T3, T4, T5], T6);
    };
}

all_the_tuples!(impl_make_dispatcher);

/// One registered route: its path split into segments (a `:name` segment binds
/// the parameter `name`), and the handler to invoke when a topic matches.
struct Route<S> {
    segments: Vec<Segment>,
    dispatcher: Dispatcher<S>,
}

enum Segment {
    Literal(String),
    Param(String),
}

impl Segment {
    fn parse(s: &str) -> Self {
        if let Some(name) = s.strip_prefix(':') {
            Segment::Param(name.to_string())
        } else {
            Segment::Literal(s.to_string())
        }
    }
}

/// Number of literal (non-parameter) segments in a route. Used to break ties
/// when more than one route matches a topic: the route with more literals is
/// the more specific one and wins, mirroring how a radix-tree router prefers
/// static segments over wildcards.
fn literal_count(segments: &[Segment]) -> usize {
    segments
        .iter()
        .filter(|s| matches!(s, Segment::Literal(_)))
        .count()
}

/// Matches a topic against a route's segments. Returns the bound parameters on
/// success. A literal segment must match exactly; a param segment binds any one
/// segment. The topic must have the same number of segments as the route.
fn match_route(segments: &[Segment], topic: &str) -> Option<Vec<(String, String)>> {
    let topic_segments: Vec<&str> = topic.split('/').collect();
    if topic_segments.len() != segments.len() {
        return None;
    }
    let mut params = Vec::new();
    for (seg, value) in segments.iter().zip(topic_segments) {
        match seg {
            Segment::Literal(lit) => {
                if lit != value {
                    return None;
                }
            }
            Segment::Param(name) => {
                params.push((name.clone(), value.to_string()));
            }
        }
    }
    Some(params)
}

/// Convert a route path into the MQTT subscription filter: each `:param`
/// segment becomes a single-level `+` wildcard.
fn route_to_topic(route: &str) -> String {
    route
        .split('/')
        .map(|seg| if seg.starts_with(':') { "+" } else { seg })
        .collect::<Vec<_>>()
        .join("/")
}

/// Routes incoming MQTT messages to handlers. Register routes with `route`,
/// then call `dispatch` for each received message.
///
/// The generic `S` is application state cloned into each handler invocation;
/// use a cheaply-clonable handle (the codebase uses `StateHandle`, an `Arc`).
pub struct MqttRouter<S>
where
    S: Clone + Send + Sync,
{
    routes: Vec<Route<S>>,
    client: AsyncClient,
}

impl<S: Clone + Send + Sync + 'static> MqttRouter<S> {
    pub fn new(client: AsyncClient) -> Self {
        Self {
            routes: Vec::new(),
            client,
        }
    }

    /// Register a route from a path like `foo/:bar` to a handler. Subscribes to
    /// the corresponding MQTT filter (`foo/+`). When a message on `foo/hello`
    /// arrives, the handler is called with `bar` bound to `hello`.
    pub async fn route<P, T, F>(&mut self, path: P, handler: F) -> RouterResult<()>
    where
        P: Into<String>,
        F: MakeDispatcher<T, S>,
    {
        let path = path.into();
        self.client
            .subscribe(route_to_topic(&path), QoS::AtMostOnce)
            .await?;
        let segments = path.split('/').map(Segment::parse).collect();
        self.routes.push(Route {
            segments,
            dispatcher: F::make_dispatcher(handler),
        });
        Ok(())
    }

    /// Dispatch a message to the matching route. When several routes match the
    /// same topic (possible because device ids occupy a wildcard segment that
    /// could in principle equal a literal from another route), the route with
    /// the most literal segments wins, so dispatch does not depend on
    /// registration order.
    pub async fn dispatch(&self, message: Message, state: S) -> RouterResult<()> {
        let best = self
            .routes
            .iter()
            .filter_map(|route| {
                match_route(&route.segments, &message.topic).map(|params| (route, params))
            })
            .max_by_key(|(route, _)| literal_count(&route.segments));

        let Some((route, params)) = best else {
            return Err(RouterError::NoMatchingRoute(message.topic));
        };

        let params = if params.is_empty() {
            JsonValue::Null
        } else {
            let mut map = serde_json::Map::new();
            for (k, v) in params {
                map.insert(k, JsonValue::String(v));
            }
            JsonValue::Object(map)
        };
        Ok(route.dispatcher.call(params, message, state).await?)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_route_to_topic() {
        assert_eq!(route_to_topic("hello/:there"), "hello/+");
        assert_eq!(route_to_topic("a/:b/foo"), "a/+/foo");
        assert_eq!(route_to_topic("hello"), "hello");
    }

    #[test]
    fn literal_route_does_not_match_different_arity() {
        let segs: Vec<Segment> = "gv/light/:id/command"
            .split('/')
            .map(Segment::parse)
            .collect();
        assert!(match_route(&segs, "gv/light/abc/command").is_some());
        assert!(match_route(&segs, "gv/light/abc/command/1").is_none());
        assert!(match_route(&segs, "gv/switch/abc/command").is_none());
    }

    #[test]
    fn more_literal_segments_win() {
        // These two routes both have 5 segments and can both match a topic
        // whose device id happens to equal "set-temperature". The light-segment
        // route has more literals, so it is the one that should be chosen.
        let light: Vec<Segment> = "gv/light/:id/command/:segment"
            .split('/')
            .map(Segment::parse)
            .collect();
        let set_temp: Vec<Segment> = "gv/:id/set-temperature/:instance/:units"
            .split('/')
            .map(Segment::parse)
            .collect();
        let topic = "gv/light/set-temperature/command/2";
        assert!(match_route(&light, topic).is_some());
        assert!(match_route(&set_temp, topic).is_some());
        assert!(literal_count(&light) > literal_count(&set_temp));
    }

    #[test]
    fn params_are_bound() {
        let segs: Vec<Segment> = "gv/light/:id/command/:segment"
            .split('/')
            .map(Segment::parse)
            .collect();
        let params = match_route(&segs, "gv/light/AA:BB/command/3").unwrap();
        assert_eq!(
            params,
            vec![
                ("id".to_string(), "AA:BB".to_string()),
                ("segment".to_string(), "3".to_string()),
            ]
        );
    }
}
