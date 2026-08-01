//! Local HTTP API for the Andromeda task control plane.

use std::str::FromStr;
use std::sync::Arc;

use andromeda_core::{IsolationLevel, TaskId};
use andromeda_runtime::{CreateTaskRequest, ServiceError, StateTransitionRequest, TaskService};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
struct AppState {
    service: Arc<TaskService>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationRequest {
    pub isolation: IsolationLevel,
    #[serde(default)]
    pub external_side_effect_confirmed: bool,
}

pub fn app(service: TaskService) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/tasks", get(list_tasks).post(create_task))
        .route("/v1/tasks/{task_id}", get(get_task))
        .route("/v1/tasks/{task_id}/evaluate", post(evaluate_task))
        .route("/v1/tasks/{task_id}/transition", post(transition_task))
        .with_state(AppState {
            service: Arc::new(service),
        })
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
    let record = state.service.create(request)?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(record)?)))
}

async fn list_tasks(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(state.service.list()?)?))
}

async fn get_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let task_id = parse_task_id(&task_id)?;
    Ok(Json(serde_json::to_value(state.service.get(task_id)?)?))
}

async fn evaluate_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<EvaluationRequest>,
) -> Result<Json<Value>, ApiError> {
    let task_id = parse_task_id(&task_id)?;
    Ok(Json(serde_json::to_value(state.service.evaluate(
        task_id,
        request.isolation,
        request.external_side_effect_confirmed,
    )?)?))
}

async fn transition_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(request): Json<StateTransitionRequest>,
) -> Result<Json<Value>, ApiError> {
    let task_id = parse_task_id(&task_id)?;
    Ok(Json(serde_json::to_value(
        state.service.transition(task_id, request)?,
    )?))
}

fn parse_task_id(value: &str) -> Result<TaskId, ApiError> {
    TaskId::from_str(value).map_err(|error| ApiError::BadRequest(error.to_string()))
}

#[derive(Debug)]
enum ApiError {
    BadRequest(String),
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
            Self::Service(ServiceError::Store(andromeda_runtime::StoreError::NotFound(_))) => {
                (StatusCode::NOT_FOUND, "not_found")
            }
            Self::Service(ServiceError::Store(
                andromeda_runtime::StoreError::RevisionConflict { .. },
            )) => (StatusCode::CONFLICT, "revision_conflict"),
            Self::Service(ServiceError::Validation(_) | ServiceError::Transition(_)) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "invalid_task")
            }
            Self::Service(_) | Self::Json(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
        };
        (status, Json(json!({ "error": code, "message": message }))).into_response()
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRequest(message) => formatter.write_str(message),
            Self::Service(error) => error.fmt(formatter),
            Self::Json(error) => error.fmt(formatter),
        }
    }
}

#[cfg(test)]
mod tests {
    use andromeda_policy::PolicyEngine;
    use andromeda_runtime::FileTaskStore;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn health_endpoint_reports_api_version() {
        let temp = TempDir::new().expect("tempdir");
        let service = TaskService::new(
            FileTaskStore::open(temp.path()).expect("store"),
            PolicyEngine::default(),
        );
        let response = app(service)
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let json: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["api_version"], "v1");
    }
}
