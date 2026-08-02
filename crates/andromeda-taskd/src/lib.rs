//! Local HTTP API for the Andromeda task control plane.

pub mod auth;

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use andromeda_core::TaskId;
use andromeda_runtime::{
    CreateTaskRequest, EvaluationRequest, GrantCapabilitiesRequest, RecordOutcomeRequest,
    ServiceError, StateTransitionRequest, StoreError, TaskService, TransitionGuardError,
};
use axum::extract::{Path, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

pub use auth::{AuthError, Authenticator};

#[derive(Debug, Clone)]
struct AppState {
    service: Arc<TaskService>,
}

/// Builds the authenticated API router.
///
/// `authenticator` is a required, by-value argument, and [`Authenticator`] has
/// no variant meaning "no authentication" — so every router this function can
/// produce checks a bearer token on every route, including `/healthz`. That is
/// the whole enforcement story: there is no configuration flag, no environment
/// variable, and no alternative constructor that yields an anonymous listener,
/// which is what the previous design was rejected for
/// (`docs/reviews/remediation-design-review.md` §1, "认证保证要在 serve 接线上
/// 不可绕过").
///
/// Layer order is deliberate: authentication is the *outermost* layer, so an
/// unauthenticated request is rejected before the Host check, before body
/// parsing, and before any store lock is taken.
pub fn app(service: TaskService, authenticator: Authenticator) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/tasks", get(list_tasks).post(create_task))
        .route("/v1/tasks/{task_id}", get(get_task))
        .route("/v1/tasks/{task_id}/capabilities", post(grant_capabilities))
        .route("/v1/tasks/{task_id}/outcomes", post(record_outcome))
        .route("/v1/tasks/{task_id}/evaluate", post(evaluate_task))
        .route("/v1/tasks/{task_id}/transition", post(transition_task))
        .layer(middleware::from_fn(require_loopback_host))
        .layer(middleware::from_fn_with_state(
            Arc::new(authenticator),
            require_token,
        ))
        .with_state(AppState {
            service: Arc::new(service),
        })
}

/// Rejects any request that does not carry the local API token.
///
/// The token is read from `Authorization: Bearer <token>` and compared in
/// constant time. A browser cannot attach this header to a cross-origin request
/// without a preflight the daemon never answers, so this also closes the
/// residual DNS-rebinding surface that the Host check alone left open.
async fn require_token(
    State(authenticator): State<Arc<Authenticator>>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);
    if presented.is_some_and(|token| authenticator.matches(token.as_bytes())) {
        return next.run(request).await;
    }
    // The response says nothing about *why* — whether the header was absent,
    // malformed, or simply wrong is not information a caller needs, and
    // distinguishing them helps an attacker probe.
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        Json(json!({
            "error": "unauthorized",
            "message": "this API requires the local token: \
                        Authorization: Bearer $(cat $ANDROMEDA_AUTH_TOKEN_FILE)",
        })),
    )
        .into_response()
}

/// Rejects requests that are not addressed to a loopback host.
///
/// `taskd` binds to loopback by default, but a malicious web page can reach
/// "localhost" services through DNS rebinding, where the browser resolves an
/// attacker-controlled name to 127.0.0.1. Such requests still carry the
/// attacker's `Host` header, so only `localhost` and literal loopback IPs
/// (any of 127.0.0.0/8, `[::1]`, and their IPv4-mapped forms, each with an
/// optional port) are accepted — exactly the addresses `taskd` will bind
/// without an override.
async fn require_loopback_host(request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .or_else(|| request.uri().host().map(str::to_owned));
    if host.as_deref().is_some_and(is_loopback_host) {
        next.run(request).await
    } else {
        (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "forbidden_host",
                "message": "taskd only accepts requests addressed to localhost",
            })),
        )
            .into_response()
    }
}

/// A bind address was rejected because it would expose the API beyond the
/// local host.
#[derive(Debug, PartialEq, Eq)]
pub struct NonLoopbackBind {
    pub address: SocketAddr,
}

impl std::fmt::Display for NonLoopbackBind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "refusing to bind {}: taskd's only credential is a local bearer token designed for \
             same-host callers, so binding beyond loopback would put the full task API on that \
             network behind a single shared secret. The Host-header check only defends browsers \
             against DNS rebinding and does not protect a non-loopback bind. Set \
             ANDROMEDA_ALLOW_NON_LOOPBACK=1 only inside an already-isolated network namespace.",
            self.address
        )
    }
}

impl std::error::Error for NonLoopbackBind {}

/// Enforces the documented rule that `taskd` must not listen beyond loopback.
///
/// The `Host` middleware inspects a header, not the accepting socket, so it
/// cannot make a public bind safe; a non-browser client simply sends
/// `Host: localhost`. This check is the actual mechanism behind the rule, and
/// it fails closed: a non-loopback address is rejected unless the operator
/// explicitly opts out via `allow_override`.
///
/// # Errors
///
/// Returns [`NonLoopbackBind`] when `address` is not a loopback address and
/// `allow_override` is false.
pub fn ensure_loopback_bind(
    address: SocketAddr,
    allow_override: bool,
) -> Result<(), NonLoopbackBind> {
    // `to_canonical` unwraps IPv4-mapped addresses so `[::ffff:127.0.0.1]`
    // counts as the loopback it actually is.
    if address.ip().to_canonical().is_loopback() || allow_override {
        Ok(())
    } else {
        Err(NonLoopbackBind { address })
    }
}

fn is_loopback_host(value: &str) -> bool {
    let (host, port) = if let Some(rest) = value.strip_prefix('[') {
        let Some((address, suffix)) = rest.split_once(']') else {
            return false;
        };
        let port = if suffix.is_empty() {
            None
        } else if let Some(port) = suffix.strip_prefix(':') {
            Some(port)
        } else {
            return false;
        };
        (address, port)
    } else if let Some((address, port)) = value.rsplit_once(':') {
        // A second colon means an unbracketed IPv6 literal or garbage.
        if address.contains(':') {
            return false;
        }
        (address, Some(port))
    } else {
        (value, None)
    };
    if port.is_some_and(|port| port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit())) {
        return false;
    }
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    // Accept exactly the addresses `ensure_loopback_bind` can be asked to
    // bind: any literal loopback IP (all of 127.0.0.0/8, `::1`, and their
    // IPv4-mapped forms). Anything that is not an IP literal — including
    // `127.0.0.1.evil.example` — fails the parse and is rejected.
    host.parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.to_canonical().is_loopback())
}

/// Runs one synchronous [`TaskService`] operation on the blocking thread
/// pool so file locks and fsyncs never stall the async workers (and thus
/// keep `/healthz` responsive even while the store lock is contended).
async fn run_blocking<T, F>(state: &AppState, operation: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&TaskService) -> Result<T, ServiceError> + Send + 'static,
{
    let service = Arc::clone(&state.service);
    tokio::task::spawn_blocking(move || operation(&service))
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?
        .map_err(ApiError::from)
}

/// Liveness plus the security posture the daemon is actually running under.
///
/// `capability_admission` is reported so an operator can verify from outside
/// the process whether issuer signatures are being enforced, instead of
/// inferring it from documentation. Reaching this route still requires the
/// token: a caller who cannot authenticate learns nothing at all.
async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "andromeda-taskd",
        "api_version": "v1",
        "authentication": "bearer_token",
        "capability_admission": state.service.admission().mode_name(),
    }))
}

async fn create_task(
    State(state): State<AppState>,
    Json(request): Json<CreateTaskRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let record = run_blocking(&state, move |service| service.create(request)).await?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(record)?)))
}

async fn list_tasks(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let listing = run_blocking(&state, TaskService::list_detailed).await?;
    Ok(Json(json!({
        "tasks": serde_json::to_value(listing.records)?,
        "warnings": serde_json::to_value(listing.warnings)?,
    })))
}

async fn get_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let task_id = parse_task_id(&task_id)?;
    let record = run_blocking(&state, move |service| service.get(task_id)).await?;
    Ok(Json(serde_json::to_value(record)?))
}

async fn grant_capabilities(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<GrantCapabilitiesRequest>,
) -> Result<Json<Value>, ApiError> {
    let task_id = parse_task_id(&task_id)?;
    let record = run_blocking(&state, move |service| {
        service.grant_capabilities(task_id, request)
    })
    .await?;
    Ok(Json(serde_json::to_value(record)?))
}

async fn record_outcome(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<RecordOutcomeRequest>,
) -> Result<Json<Value>, ApiError> {
    let task_id = parse_task_id(&task_id)?;
    let record = run_blocking(&state, move |service| {
        service.record_outcome(task_id, request)
    })
    .await?;
    Ok(Json(serde_json::to_value(record)?))
}

async fn evaluate_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<EvaluationRequest>,
) -> Result<Json<Value>, ApiError> {
    let task_id = parse_task_id(&task_id)?;
    let report = run_blocking(&state, move |service| service.evaluate(task_id, &request)).await?;
    Ok(Json(serde_json::to_value(report)?))
}

async fn transition_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<StateTransitionRequest>,
) -> Result<Json<Value>, ApiError> {
    let task_id = parse_task_id(&task_id)?;
    let record = run_blocking(&state, move |service| service.transition(task_id, request)).await?;
    Ok(Json(serde_json::to_value(record)?))
}

fn parse_task_id(value: &str) -> Result<TaskId, ApiError> {
    TaskId::from_str(value).map_err(|error| ApiError::BadRequest(error.to_string()))
}

#[derive(Debug)]
enum ApiError {
    BadRequest(String),
    Internal(String),
    Service(ServiceError),
    Json(serde_json::Error),
}

impl From<ServiceError> for ApiError {
    fn from(error: ServiceError) -> Self {
        Self::Service(error)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let message = self.to_string();
        let (status, code) = match &self {
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            Self::Service(ServiceError::Store(StoreError::NotFound(_))) => {
                (StatusCode::NOT_FOUND, "not_found")
            }
            Self::Service(ServiceError::Store(StoreError::AlreadyExists(_))) => {
                (StatusCode::CONFLICT, "already_exists")
            }
            Self::Service(ServiceError::Store(StoreError::RevisionConflict { .. })) => {
                (StatusCode::CONFLICT, "revision_conflict")
            }
            // An unconfirmed L3 external side effect is a distinct operator
            // action (obtain the final confirmation) from fixing an invalid
            // task, so it gets its own wire code.
            Self::Service(ServiceError::Guard(
                TransitionGuardError::ExternalConfirmationRequired { .. },
            )) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "external_confirmation_required",
            ),
            // Likewise, the evidence gate on `Verifying -> Succeeded` asks the
            // caller to record (evidenced, successful) outcomes, not to repair
            // the plan or the transition.
            Self::Service(ServiceError::Guard(
                TransitionGuardError::MissingOutcomes { .. }
                | TransitionGuardError::UnsuccessfulOutcomes { .. }
                | TransitionGuardError::MissingEvidence { .. },
            )) => (StatusCode::UNPROCESSABLE_ENTITY, "missing_evidence"),
            // A refused capability asks for a different operator action from a
            // malformed plan: obtain a grant the configured issuer signed, or
            // reconfigure the keyring. It gets its own wire code so a client
            // does not have to parse prose to tell the two apart.
            Self::Service(ServiceError::Admission(_)) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "capability_not_admitted")
            }
            Self::Service(
                ServiceError::Validation(_) | ServiceError::Transition(_) | ServiceError::Guard(_),
            ) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_task"),
            Self::Service(_) | Self::Json(_) | Self::Internal(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
        };
        (status, Json(json!({ "error": code, "message": message }))).into_response()
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRequest(message) | Self::Internal(message) => formatter.write_str(message),
            Self::Service(error) => error.fmt(formatter),
            Self::Json(error) => error.fmt(formatter),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use andromeda_core::{
        ActionId, ActionKind, ActionPlan, ActionSpec, Capability, CapabilityId, CapabilityResource,
        FileAccess, Intent, IsolationLevel, RecoverySemantics, RiskLevel, TaskState,
    };
    use andromeda_policy::PolicyEngine;
    use andromeda_runtime::{CapabilityAdmission, FileTaskStore};
    use axum::body::Body;
    use axum::http::Request;
    use chrono::Utc;
    use http_body_util::BodyExt;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;

    const LOCAL_HOST_HEADER: &str = "127.0.0.1:7777";

    /// The token every test authenticates with. A fixed literal, never a
    /// generated one: tests must not depend on an RNG.
    const TEST_TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn test_app(temp: &TempDir) -> Router {
        test_app_with(temp, CapabilityAdmission::unsigned_for_development())
    }

    fn test_app_with(temp: &TempDir, admission: CapabilityAdmission) -> Router {
        let service = TaskService::new(
            FileTaskStore::open(temp.path()).expect("store"),
            PolicyEngine::default(),
            admission,
        );
        // Note what cannot be written here: there is no `app(service)`. Every
        // router in every test, like every router in production, carries an
        // Authenticator because the signature demands one.
        app(
            service,
            Authenticator::from_token(TEST_TOKEN).expect("token"),
        )
    }

    fn inspection_request(path: &str) -> CreateTaskRequest {
        let task_id = TaskId::new();
        let capability = Capability {
            id: CapabilityId::new(),
            resource: CapabilityResource::Files {
                root: PathBuf::from(path),
                access: FileAccess::Read,
            },
            issued_to: task_id.to_string(),
            issued_at: Utc::now(),
            expires_at: None,
            single_use: false,
            signature: None,
        };
        let plan = ActionPlan {
            schema_version: ActionPlan::CURRENT_SCHEMA_VERSION,
            task_id,
            intent: Intent::new("Inspect", "test"),
            actions: vec![ActionSpec {
                id: ActionId::new(),
                name: "Inspect directory".into(),
                kind: ActionKind::Inspect,
                target: path.into(),
                arguments: BTreeMap::new(),
                depends_on: Vec::new(),
                required_capabilities: vec![capability.id],
                risk: RiskLevel::L1Sandboxed,
                recovery: RecoverySemantics::None,
            }],
        };
        CreateTaskRequest {
            plan,
            capabilities: vec![capability],
            actor: "test".into(),
        }
    }

    fn local_request(
        method: &str,
        uri: &str,
        body: Option<&impl serde::Serialize>,
    ) -> Request<Body> {
        authenticated_request(method, uri, body, Some(TEST_TOKEN))
    }

    fn authenticated_request(
        method: &str,
        uri: &str,
        body: Option<&impl serde::Serialize>,
        token: Option<&str>,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, LOCAL_HOST_HEADER);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(body).expect("body")))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        }
    }

    async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
        let response = app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).expect("json body")
        };
        (status, json)
    }

    #[test]
    fn non_loopback_binds_are_refused_unless_explicitly_allowed() {
        // The Host middleware checks a header, not the accepting socket, so it
        // cannot protect a public bind. This is the mechanism that does.
        let public: SocketAddr = "0.0.0.0:7777".parse().expect("addr");
        let routable: SocketAddr = "192.168.1.10:7777".parse().expect("addr");
        let any_v6: SocketAddr = "[::]:7777".parse().expect("addr");
        for address in [public, routable, any_v6] {
            assert_eq!(
                ensure_loopback_bind(address, false),
                Err(NonLoopbackBind { address }),
                "{address} must be refused by default"
            );
            assert_eq!(
                ensure_loopback_bind(address, true),
                Ok(()),
                "{address} must be reachable behind an explicit opt-out"
            );
        }
    }

    #[test]
    fn loopback_binds_are_accepted() {
        for address in [
            "127.0.0.1:7777",
            "[::1]:7777",
            "127.0.0.5:1",
            "[::ffff:127.0.0.1]:7777",
        ] {
            let address: SocketAddr = address.parse().expect("addr");
            assert_eq!(ensure_loopback_bind(address, false), Ok(()));
        }
    }

    #[tokio::test]
    async fn succeeding_a_task_requires_recorded_evidence() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let create = inspection_request(workspace_path());
        let task_id = create.plan.task_id;
        let action_id = create.plan.actions[0].id;
        send(&app, local_request("POST", "/v1/tasks", Some(&create))).await;

        let mut revision = 0;
        for state in ["running", "verifying"] {
            let (status, body) = send(
                &app,
                local_request(
                    "POST",
                    &format!("/v1/tasks/{task_id}/transition"),
                    Some(&json!({
                        "to": state,
                        "actor": "runner",
                        "expected_revision": revision,
                    })),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{state}: {body}");
            revision = body["revision"].as_u64().expect("revision");
        }

        // Without an outcome the verifier cannot declare success; the wire
        // code tells the caller evidence is what is missing.
        let (status, body) = send(
            &app,
            local_request(
                "POST",
                &format!("/v1/tasks/{task_id}/transition"),
                Some(&json!({
                    "to": "succeeded",
                    "actor": "verifier",
                    "expected_revision": revision,
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"], "missing_evidence");

        let now = chrono::Utc::now();
        let (status, body) = send(
            &app,
            local_request(
                "POST",
                &format!("/v1/tasks/{task_id}/outcomes"),
                Some(&json!({
                    "outcome": {
                        "action_id": action_id,
                        "status": "succeeded",
                        "started_at": now,
                        "finished_at": now,
                        "evidence": [{"kind": "assertion", "summary": "listing matched"}],
                        "error": null,
                    },
                    "actor": "executor",
                    "expected_revision": revision,
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        revision = body["revision"].as_u64().expect("revision");

        let (status, body) = send(
            &app,
            local_request(
                "POST",
                &format!("/v1/tasks/{task_id}/transition"),
                Some(&json!({
                    "to": "succeeded",
                    "actor": "verifier",
                    "expected_revision": revision,
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["state"], "succeeded");
    }

    #[tokio::test]
    async fn health_endpoint_reports_api_version() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let (status, json) = send(&app, local_request("GET", "/healthz", None::<&Value>)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["api_version"], "v1");
    }

    #[tokio::test]
    async fn create_get_and_list_round_trip() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let request = inspection_request(workspace_path());
        let task_id = request.plan.task_id;

        let (status, created) =
            send(&app, local_request("POST", "/v1/tasks", Some(&request))).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(created["state"], "ready");
        assert_eq!(created["revision"], 0);

        let (status, fetched) = send(
            &app,
            local_request("GET", &format!("/v1/tasks/{task_id}"), None::<&Value>),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(fetched, created);

        let (status, listing) = send(&app, local_request("GET", "/v1/tasks", None::<&Value>)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listing["tasks"].as_array().expect("tasks").len(), 1);
        assert_eq!(listing["warnings"].as_array().expect("warnings").len(), 0);
    }

    #[tokio::test]
    async fn duplicate_create_maps_to_conflict() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let request = inspection_request(workspace_path());

        let (status, _) = send(&app, local_request("POST", "/v1/tasks", Some(&request))).await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, error) = send(&app, local_request("POST", "/v1/tasks", Some(&request))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["error"], "already_exists");
    }

    #[tokio::test]
    async fn unknown_task_maps_to_not_found() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let (status, error) = send(
            &app,
            local_request(
                "GET",
                &format!("/v1/tasks/{}", TaskId::new()),
                None::<&Value>,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error["error"], "not_found");
    }

    #[tokio::test]
    async fn invalid_uuid_maps_to_bad_request() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let (status, error) = send(
            &app,
            local_request("GET", "/v1/tasks/not-a-uuid", None::<&Value>),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["error"], "bad_request");
    }

    #[tokio::test]
    async fn invalid_plan_maps_to_unprocessable_entity() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let mut request = inspection_request(workspace_path());
        request.plan.schema_version = 999;
        let (status, error) = send(&app, local_request("POST", "/v1/tasks", Some(&request))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error["error"], "invalid_task");
    }

    #[tokio::test]
    async fn evaluate_returns_decisions_and_persists_event() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let request = inspection_request(workspace_path());
        let task_id = request.plan.task_id;
        send(&app, local_request("POST", "/v1/tasks", Some(&request))).await;

        let evaluation = EvaluationRequest {
            isolation: Some(IsolationLevel::Sandbox),
            ..EvaluationRequest::default()
        };
        let (status, report) = send(
            &app,
            local_request(
                "POST",
                &format!("/v1/tasks/{task_id}/evaluate"),
                Some(&evaluation),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(report["revision"], 1);
        assert!(
            report["decisions"]
                .as_object()
                .expect("decisions")
                .values()
                .all(|decision| decision["effect"] == "allow")
        );

        let (status, fetched) = send(
            &app,
            local_request("GET", &format!("/v1/tasks/{task_id}"), None::<&Value>),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(fetched["revision"], 1);
        let events = fetched["events"].as_array().expect("events");
        assert_eq!(events.last().expect("event")["kind"]["type"], "evaluated");
    }

    #[tokio::test]
    async fn transition_applies_and_rejects_stale_revisions() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let request = inspection_request(workspace_path());
        let task_id = request.plan.task_id;
        send(&app, local_request("POST", "/v1/tasks", Some(&request))).await;

        let transition = StateTransitionRequest {
            to: andromeda_core::TaskState::Running,
            actor: "runner".into(),
            expected_revision: 0,
            external_side_effect_confirmed: false,
        };
        let (status, updated) = send(
            &app,
            local_request(
                "POST",
                &format!("/v1/tasks/{task_id}/transition"),
                Some(&transition),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(updated["state"], "running");
        assert_eq!(updated["revision"], 1);

        let (status, error) = send(
            &app,
            local_request(
                "POST",
                &format!("/v1/tasks/{task_id}/transition"),
                Some(&transition),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["error"], "revision_conflict");
    }

    #[tokio::test]
    async fn invalid_transition_maps_to_unprocessable_entity() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let request = inspection_request(workspace_path());
        let task_id = request.plan.task_id;
        send(&app, local_request("POST", "/v1/tasks", Some(&request))).await;

        let transition = StateTransitionRequest {
            to: andromeda_core::TaskState::Succeeded,
            actor: "runner".into(),
            expected_revision: 0,
            external_side_effect_confirmed: false,
        };
        let (status, error) = send(
            &app,
            local_request(
                "POST",
                &format!("/v1/tasks/{task_id}/transition"),
                Some(&transition),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error["error"], "invalid_task");
    }

    /// Builds a create request whose single action is an L3 external side
    /// effect, so `Ready -> Running` demands explicit confirmation.
    fn l3_request() -> CreateTaskRequest {
        let task_id = TaskId::new();
        let capability = Capability {
            id: CapabilityId::new(),
            resource: CapabilityResource::Network {
                host: "api.example.com".into(),
                port: None,
            },
            issued_to: task_id.to_string(),
            issued_at: Utc::now(),
            expires_at: None,
            single_use: false,
            signature: None,
        };
        let plan = ActionPlan {
            schema_version: ActionPlan::CURRENT_SCHEMA_VERSION,
            task_id,
            intent: Intent::new("Send", "test"),
            actions: vec![ActionSpec {
                id: ActionId::new(),
                name: "send result through broker".into(),
                kind: ActionKind::NetworkRequest,
                target: "api.example.com".into(),
                arguments: BTreeMap::new(),
                depends_on: Vec::new(),
                required_capabilities: vec![capability.id],
                risk: RiskLevel::L3ExternalSideEffect,
                recovery: RecoverySemantics::None,
            }],
        };
        CreateTaskRequest {
            plan,
            capabilities: vec![capability],
            actor: "test".into(),
        }
    }

    #[tokio::test]
    async fn unconfirmed_l3_transition_reports_external_confirmation_required() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let request = l3_request();
        let task_id = request.plan.task_id;
        let (status, created) =
            send(&app, local_request("POST", "/v1/tasks", Some(&request))).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(created["state"], "ready");

        let transition = StateTransitionRequest {
            to: TaskState::Running,
            actor: "runner".into(),
            expected_revision: 0,
            external_side_effect_confirmed: false,
        };
        let (status, error) = send(
            &app,
            local_request(
                "POST",
                &format!("/v1/tasks/{task_id}/transition"),
                Some(&transition),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error["error"], "external_confirmation_required");
    }

    #[tokio::test]
    async fn list_reports_corrupt_records_as_warnings() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let request = inspection_request(workspace_path());
        send(&app, local_request("POST", "/v1/tasks", Some(&request))).await;
        let corrupt = temp
            .path()
            .join(format!("{}.{:020}.json", TaskId::new(), 0));
        std::fs::write(&corrupt, b"{ definitely not json").expect("corrupt record");

        let (status, listing) = send(&app, local_request("GET", "/v1/tasks", None::<&Value>)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listing["tasks"].as_array().expect("tasks").len(), 1);
        let warnings = listing["warnings"].as_array().expect("warnings");
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0]["path"].as_str().expect("warning path"),
            corrupt.to_str().expect("corrupt path"),
        );
    }

    #[tokio::test]
    async fn foreign_host_header_is_forbidden() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        // Authenticated on purpose: this asserts the Host check still fires for
        // a caller who *does* hold the token, so the two layers are independent
        // rather than one masking the other.
        let request = Request::builder()
            .uri("/healthz")
            .header(header::HOST, "rebind.attacker.example:7777")
            .header(header::AUTHORIZATION, format!("Bearer {TEST_TOKEN}"))
            .body(Body::empty())
            .expect("request");
        let (status, error) = send(&app, request).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(error["error"], "forbidden_host");
    }

    #[tokio::test]
    async fn missing_host_header_is_forbidden() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let request = Request::builder()
            .uri("/healthz")
            .header(header::AUTHORIZATION, format!("Bearer {TEST_TOKEN}"))
            .body(Body::empty())
            .expect("request");
        let (status, error) = send(&app, request).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(error["error"], "forbidden_host");
    }

    #[test]
    fn loopback_host_matching_is_strict() {
        for allowed in [
            "localhost",
            "LOCALHOST",
            "localhost:7777",
            "127.0.0.1",
            "127.0.0.1:7777",
            // Any 127.0.0.0/8 address `taskd` can bind must also pass the
            // Host check, or `--listen 127.0.0.5` would 403 every request.
            "127.0.0.5",
            "127.0.0.5:7777",
            "[::1]",
            "[::1]:7777",
            "[::ffff:127.0.0.1]:7777",
        ] {
            assert!(is_loopback_host(allowed), "{allowed} must be allowed");
        }
        for rejected in [
            "",
            "example.com",
            "example.com:7777",
            "127.0.0.1.evil.example",
            "localhost.evil.example",
            "localhost:",
            "localhost:7777x",
            "::1",
            "[::1]x",
            "[::2]",
            "192.168.1.10:7777",
        ] {
            assert!(!is_loopback_host(rejected), "{rejected} must be rejected");
        }
    }

    #[tokio::test]
    async fn grant_endpoint_unblocks_gated_ready_transition() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let mut request = inspection_request(workspace_path());
        let needed = request.capabilities[0].clone();
        request.capabilities.clear();
        let task_id = request.plan.task_id;

        let (status, created) =
            send(&app, local_request("POST", "/v1/tasks", Some(&request))).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(created["state"], "awaiting_approval");

        // Ready is policy-gated: an ungranted plan cannot reach it.
        let premature = StateTransitionRequest {
            to: TaskState::Ready,
            actor: "sneaky".into(),
            expected_revision: 0,
            external_side_effect_confirmed: false,
        };
        let (status, error) = send(
            &app,
            local_request(
                "POST",
                &format!("/v1/tasks/{task_id}/transition"),
                Some(&premature),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error["error"], "invalid_task");

        // Grant the missing capability, then Ready is allowed.
        let grant = GrantCapabilitiesRequest {
            capabilities: vec![needed],
            actor: "approver".into(),
            expected_revision: 0,
        };
        let (status, granted) = send(
            &app,
            local_request(
                "POST",
                &format!("/v1/tasks/{task_id}/capabilities"),
                Some(&grant),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(granted["revision"], 1);

        let promote = StateTransitionRequest {
            to: TaskState::Ready,
            actor: "approver".into(),
            expected_revision: 1,
            external_side_effect_confirmed: false,
        };
        let (status, ready) = send(
            &app,
            local_request(
                "POST",
                &format!("/v1/tasks/{task_id}/transition"),
                Some(&promote),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ready["state"], "ready");
    }

    #[tokio::test]
    async fn create_rejects_unknown_capability_field() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let request = inspection_request(workspace_path());
        let mut value = serde_json::to_value(&request).expect("serialize request");
        // A camelCase typo of `expires_at` must be rejected, not dropped.
        value["capabilities"][0]["expiresAt"] = json!("2099-01-01T00:00:00Z");
        let http_request = Request::builder()
            .method("POST")
            .uri("/v1/tasks")
            .header(header::HOST, LOCAL_HOST_HEADER)
            .header(header::AUTHORIZATION, format!("Bearer {TEST_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&value).expect("body")))
            .expect("request");
        let response = app.clone().oneshot(http_request).await.expect("response");
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Every route, including `/healthz`, must refuse an unauthenticated
    /// caller. This is the runtime half of the guarantee; the compile-time half
    /// is that `app` cannot be called without an [`Authenticator`] at all.
    #[tokio::test]
    async fn every_route_rejects_a_caller_without_the_token() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let task_id = TaskId::new();
        let routes = [
            ("GET", "/healthz".to_owned()),
            ("GET", "/v1/tasks".to_owned()),
            ("POST", "/v1/tasks".to_owned()),
            ("GET", format!("/v1/tasks/{task_id}")),
            ("POST", format!("/v1/tasks/{task_id}/capabilities")),
            ("POST", format!("/v1/tasks/{task_id}/outcomes")),
            ("POST", format!("/v1/tasks/{task_id}/evaluate")),
            ("POST", format!("/v1/tasks/{task_id}/transition")),
        ];
        for (method, uri) in routes {
            let request = authenticated_request(method, &uri, None::<&serde_json::Value>, None);
            let (status, error) = send(&app, request).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
            assert_eq!(error["error"], "unauthorized", "{method} {uri}");
        }
    }

    /// Wrong, truncated, extended, and wrongly-framed tokens must all fail.
    #[tokio::test]
    async fn only_the_exact_token_is_accepted() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        for wrong in [
            "",
            "wrong",
            &TEST_TOKEN[..TEST_TOKEN.len() - 1],
            &format!("{TEST_TOKEN}x"),
            &TEST_TOKEN.to_uppercase(),
        ] {
            let request =
                authenticated_request("GET", "/healthz", None::<&serde_json::Value>, Some(wrong));
            let (status, _) = send(&app, request).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "token {wrong:?}");
        }

        // A correct token in the wrong scheme is still not authentication.
        let request = Request::builder()
            .uri("/healthz")
            .header(header::HOST, LOCAL_HOST_HEADER)
            .header(header::AUTHORIZATION, format!("Basic {TEST_TOKEN}"))
            .body(Body::empty())
            .expect("request");
        let (status, _) = send(&app, request).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let request = authenticated_request(
            "GET",
            "/healthz",
            None::<&serde_json::Value>,
            Some(TEST_TOKEN),
        );
        let (status, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["authentication"], "bearer_token");
    }

    /// An unauthenticated listener is not constructible. The compile-time part
    /// cannot be asserted at runtime, so this test pins the two properties that
    /// make it true and that a future refactor could quietly remove: no
    /// `Authenticator` can be built from an empty or trivially short secret,
    /// and the only way to reach a `Router` is through `app`, which demands one.
    #[test]
    fn an_unauthenticated_listener_cannot_be_constructed() {
        // Were an `Authenticator::none()`-style escape hatch ever added, it
        // would have to produce a value accepting the empty credential. No such
        // value exists: every constructor is fallible and rejects weak input.
        assert!(Authenticator::from_token("").is_err());
        assert!(Authenticator::from_token("   ").is_err());
        assert!(Authenticator::from_token(&"a".repeat(auth::MIN_TOKEN_CHARS - 1)).is_err());

        let authenticator = Authenticator::from_token(TEST_TOKEN).expect("token");
        assert!(!authenticator.matches(b""));
        assert!(authenticator.matches(TEST_TOKEN.as_bytes()));

        // `app` takes the authenticator by value; there is no second
        // constructor, and `Router` is only reachable through it.
        let temp = TempDir::new().expect("tempdir");
        let _: Router = test_app(&temp);
    }

    /// Reports the capability admission mode so an operator can see, from
    /// outside the process, whether issuer signatures are enforced.
    #[tokio::test]
    async fn health_reports_the_capability_admission_mode() {
        let temp = TempDir::new().expect("tempdir");
        let app = test_app(&temp);
        let (status, body) = send(&app, local_request("GET", "/healthz", None::<&Value>)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["capability_admission"], "unsigned_allowed");

        let signing_key = andromeda_core::CapabilitySigningKey::from_seed(&[3u8; 32]);
        let mut keyring = andromeda_core::CapabilityKeyring::new();
        keyring
            .insert_hex("issuer-2026", &signing_key.verifying_key_hex())
            .expect("key");
        let signed_temp = TempDir::new().expect("tempdir");
        let signed_app = test_app_with(
            &signed_temp,
            CapabilityAdmission::require_signed(keyring).expect("keyring"),
        );
        let (status, body) = send(
            &signed_app,
            local_request("GET", "/healthz", None::<&Value>),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["capability_admission"], "require_signed");
    }

    /// End-to-end over HTTP: with a keyring configured, an unsigned capability
    /// is refused at `POST /v1/tasks`, and the same capability signed by the
    /// trusted issuer is accepted.
    #[tokio::test]
    async fn create_enforces_capability_signatures_when_a_keyring_is_configured() {
        let signing_key = andromeda_core::CapabilitySigningKey::from_seed(&[4u8; 32]);
        let mut keyring = andromeda_core::CapabilityKeyring::new();
        keyring
            .insert_hex("issuer-2026", &signing_key.verifying_key_hex())
            .expect("key");
        let temp = TempDir::new().expect("tempdir");
        let app = test_app_with(
            &temp,
            CapabilityAdmission::require_signed(keyring).expect("keyring"),
        );

        let mut request = inspection_request(workspace_path());
        let (status, error) = send(&app, local_request("POST", "/v1/tasks", Some(&request))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error["error"], "capability_not_admitted");
        assert!(
            error["message"]
                .as_str()
                .expect("message")
                .contains("no issuer signature"),
            "{error}"
        );

        signing_key
            .sign_in_place(&mut request.capabilities[0], "issuer-2026")
            .expect("sign");
        let (status, _) = send(&app, local_request("POST", "/v1/tasks", Some(&request))).await;
        assert_eq!(status, StatusCode::CREATED);
    }

    /// The rework constraint that the key directory's protection and the
    /// service's runtime identity are defined in **one** place, with an
    /// assertion. The constants in `crate::auth` are that place; this asserts
    /// the shipped unit agrees with them, so the pair cannot drift into the
    /// "0700 root:root directory under `DynamicUser`" failure that sank the
    /// previous attempt.
    #[test]
    fn the_shipped_unit_matches_the_token_permission_constants() {
        let unit_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../os/files/usr/lib/systemd/system/andromeda-taskd.service");
        let unit = std::fs::read_to_string(&unit_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", unit_path.display()));
        let directives: Vec<&str> = unit
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .collect();
        let has = |directive: &str| directives.iter().any(|line| *line == directive);

        // The directory holding the token, and its mode, come from the code.
        assert!(
            has(&format!(
                "RuntimeDirectory={}",
                auth::RUNTIME_DIRECTORY_NAME
            )),
            "unit must create the token directory systemd-side: {directives:?}"
        );
        assert!(
            has(&format!(
                "RuntimeDirectoryMode={:04o}",
                auth::TOKEN_DIR_MODE
            )),
            "RuntimeDirectoryMode must equal auth::TOKEN_DIR_MODE"
        );
        assert!(
            has(&format!("StateDirectoryMode={:04o}", auth::TOKEN_DIR_MODE)),
            "StateDirectoryMode must equal auth::TOKEN_DIR_MODE"
        );
        assert!(
            has(&format!(
                "Environment=ANDROMEDA_AUTH_TOKEN_FILE={}",
                auth::SYSTEM_TOKEN_PATH
            )),
            "the unit must point taskd at the documented token path"
        );

        // Identity: exactly one runtime identity is declared, and it is the
        // dynamic one. A static `User=`/`Group=` here would reintroduce the
        // two-places problem, because nothing in the tree would own that name.
        assert!(has("DynamicUser=yes"), "{directives:?}");
        assert!(
            !directives
                .iter()
                .any(|line| line.starts_with("User=") || line.starts_with("Group=")),
            "the unit must not name a static identity: {directives:?}"
        );

        // UMask must not be looser than the file mode the code creates, or the
        // unit would silently contradict `auth::TOKEN_FILE_MODE`.
        let umask = directives
            .iter()
            .find_map(|line| line.strip_prefix("UMask="))
            .expect("unit must set UMask");
        let umask = u32::from_str_radix(umask, 8).expect("octal UMask");
        assert_eq!(
            !umask & 0o777 & auth::PRIVATE_MODE_MASK,
            0,
            "UMask={umask:04o} would permit group/other access the code forbids"
        );
        assert_eq!(
            auth::TOKEN_FILE_MODE & auth::PRIVATE_MODE_MASK,
            0,
            "TOKEN_FILE_MODE must grant nothing beyond the owner"
        );

        // Nothing may re-enable anonymous access. There is no such switch, and
        // this asserts none is introduced by way of the unit.
        assert!(
            !unit.contains("ANDROMEDA_ALLOW_NON_LOOPBACK"),
            "the shipped unit must not opt out of the loopback bind check"
        );
    }

    #[cfg(not(target_os = "windows"))]
    const fn workspace_path() -> &'static str {
        "/workspace"
    }

    #[cfg(target_os = "windows")]
    const fn workspace_path() -> &'static str {
        r"C:\workspace"
    }
}
