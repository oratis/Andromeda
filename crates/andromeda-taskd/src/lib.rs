//! Local HTTP API for the Andromeda task control plane.

use std::str::FromStr;
use std::sync::Arc;

use andromeda_core::TaskId;
use andromeda_runtime::{
    CreateTaskRequest, EvaluationRequest, GrantCapabilitiesRequest, ServiceError,
    StateTransitionRequest, StoreError, TaskService,
};
use axum::extract::{Path, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
struct AppState {
    service: Arc<TaskService>,
}

pub fn app(service: TaskService) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/tasks", get(list_tasks).post(create_task))
        .route("/v1/tasks/{task_id}", get(get_task))
        .route("/v1/tasks/{task_id}/capabilities", post(grant_capabilities))
        .route("/v1/tasks/{task_id}/evaluate", post(evaluate_task))
        .route("/v1/tasks/{task_id}/transition", post(transition_task))
        .layer(middleware::from_fn(require_loopback_host))
        .with_state(AppState {
            service: Arc::new(service),
        })
}

/// Rejects requests that are not addressed to a loopback host.
///
/// `taskd` binds to loopback by default, but a malicious web page can reach
/// "localhost" services through DNS rebinding, where the browser resolves an
/// attacker-controlled name to 127.0.0.1. Such requests still carry the
/// attacker's `Host` header, so only `localhost`, `127.0.0.1`, and `[::1]`
/// (each with an optional port) are accepted.
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
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
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

async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "andromeda-taskd",
        "api_version": "v1"
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
    use andromeda_runtime::FileTaskStore;
    use axum::body::Body;
    use axum::http::Request;
    use chrono::Utc;
    use http_body_util::BodyExt;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;

    const LOCAL_HOST_HEADER: &str = "127.0.0.1:7777";

    fn test_app(temp: &TempDir) -> Router {
        let service = TaskService::new(
            FileTaskStore::open(temp.path()).expect("store"),
            PolicyEngine::default(),
        );
        app(service)
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
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, LOCAL_HOST_HEADER);
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
        let request = Request::builder()
            .uri("/healthz")
            .header(header::HOST, "rebind.attacker.example:7777")
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
            "[::1]",
            "[::1]:7777",
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
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&value).expect("body")))
            .expect("request");
        let response = app.clone().oneshot(http_request).await.expect("response");
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
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
