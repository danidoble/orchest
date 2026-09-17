//! Loopback-only HTTP adapter for the Orchest core.
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use orchest_core::{Orchest, OrchestError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    env,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};

struct ApiState {
    orchest: Orchest,
    token: String,
}
type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::var_os("ORCHEST_ROOT")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(orchest_platform::default_root)?;
    let orchest = Orchest::open(root.clone(), &root.join("config/packages"))?;
    let token_path = root.join("runtime/state/api-token");
    let token = if token_path.exists() {
        std::fs::read_to_string(&token_path)?
    } else {
        let token = uuid::Uuid::new_v4().to_string();
        orchest_platform::atomic_write(&token_path, token.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600))?;
        }
        token
    };
    let state = Arc::new(ApiState { orchest, token });
    let routes = Router::new()
        .route("/api/v1/status", get(status))
        .route("/api/v1/packages", get(packages))
        .route("/api/v1/packages/installed", get(installed))
        .route("/api/v1/packages/:package/install", post(install))
        .route(
            "/api/v1/packages/:package/:version",
            axum::routing::delete(remove),
        )
        .route("/api/v1/projects", get(projects).post(add_project))
        .route("/api/v1/projects/:name", get(project))
        .route("/api/v1/projects/:name/php", post(set_php))
        .route("/api/v1/services/:name", get(service_status))
        .route("/api/v1/services/:name/config", get(service_config))
        .route("/api/v1/services/:name/start", post(start_service))
        .route("/api/v1/services/:name/stop", post(stop_service))
        .route("/api/v1/doctor", get(doctor))
        .route("/api/v1/ports", get(ports))
        .route("/api/v1/ports/:port", get(check_port))
        .with_state(state);
    let port = env::var("ORCHEST_API_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8765);
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("Orchest API listening on http://{address}");
    axum::serve(listener, routes).await?;
    Ok(())
}
fn authorize(headers: &HeaderMap, state: &ApiState) -> Result<(), (StatusCode, Json<Value>)> {
    let expected = format!("Bearer {}", state.token);
    if headers.get("authorization").and_then(|h| h.to_str().ok()) == Some(expected.as_str()) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":{"code":"unauthorized","message":"bearer token required"}})),
        ))
    }
}
fn error(error: OrchestError) -> (StatusCode, Json<Value>) {
    let (status, code) = match error {
        OrchestError::ProjectNotFound(_) => (StatusCode::NOT_FOUND, "project_not_found"),
        OrchestError::RuntimeNotInstalled(_) => (StatusCode::NOT_FOUND, "runtime_not_installed"),
        OrchestError::ProjectExists(_) => (StatusCode::CONFLICT, "project_exists"),
        OrchestError::PackageInUse(_) => (StatusCode::CONFLICT, "package_in_use"),
        OrchestError::InvalidInput(_) | OrchestError::Config(_) => {
            (StatusCode::BAD_REQUEST, "invalid_input")
        }
        OrchestError::Package(_) => (StatusCode::BAD_REQUEST, "package_error"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
    };
    (
        status,
        Json(json!({"error":{"code":code,"message":error.to_string()}})),
    )
}
async fn status(State(state): State<Arc<ApiState>>, headers: HeaderMap) -> ApiResult {
    authorize(&headers, &state)?;
    Ok(Json(
        json!({"root":state.orchest.root(),"installed":state.orchest.installed(None).map_err(error)?.len(),"projects":state.orchest.projects().map_err(error)?.len()}),
    ))
}
async fn packages(State(state): State<Arc<ApiState>>, headers: HeaderMap) -> ApiResult {
    authorize(&headers, &state)?;
    Ok(Json(json!(state.orchest.catalog().list())))
}
async fn installed(State(state): State<Arc<ApiState>>, headers: HeaderMap) -> ApiResult {
    authorize(&headers, &state)?;
    Ok(Json(json!(state.orchest.installed(None).map_err(error)?)))
}
#[derive(Deserialize)]
struct InstallBody {
    version: String,
}
async fn install(
    State(state): State<Arc<ApiState>>,
    Path(package): Path<String>,
    headers: HeaderMap,
    Json(body): Json<InstallBody>,
) -> ApiResult {
    authorize(&headers, &state)?;
    let installed =
        tokio::task::spawn_blocking(move || state.orchest.install(&package, &body.version))
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error":{"code":"task_error","message":e.to_string()}})),
                )
            })?
            .map_err(error)?;
    Ok(Json(json!(installed)))
}
#[derive(Deserialize)]
struct RemoveQuery {
    force: Option<bool>,
}
async fn remove(
    State(state): State<Arc<ApiState>>,
    Path((package, version)): Path<(String, String)>,
    axum::extract::Query(query): axum::extract::Query<RemoveQuery>,
    headers: HeaderMap,
) -> ApiResult {
    authorize(&headers, &state)?;
    state
        .orchest
        .remove(&package, &version, query.force.unwrap_or(false))
        .map_err(error)?;
    Ok(Json(json!({"removed":format!("{package}@{version}")})))
}
async fn projects(State(state): State<Arc<ApiState>>, headers: HeaderMap) -> ApiResult {
    authorize(&headers, &state)?;
    Ok(Json(json!(state.orchest.projects().map_err(error)?)))
}
#[derive(Deserialize)]
struct AddProjectBody {
    name: String,
    path: Option<PathBuf>,
}
async fn add_project(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(body): Json<AddProjectBody>,
) -> ApiResult {
    authorize(&headers, &state)?;
    let project = match body.path {
        Some(path) => state.orchest.add_project(&path, &body.name),
        None => state.orchest.add_project_default(&body.name),
    }
    .map_err(error)?;
    Ok(Json(json!(project)))
}
async fn project(
    State(state): State<Arc<ApiState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> ApiResult {
    authorize(&headers, &state)?;
    Ok(Json(json!(state.orchest.project(&name).map_err(error)?)))
}
#[derive(Deserialize)]
struct PhpBody {
    version: String,
}
async fn set_php(
    State(state): State<Arc<ApiState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PhpBody>,
) -> ApiResult {
    authorize(&headers, &state)?;
    Ok(Json(json!(state
        .orchest
        .set_project_php(&name, &body.version)
        .map_err(error)?)))
}
fn supported_service(name: &str) -> Result<(), (StatusCode, Json<Value>)> {
    if matches!(name, "mailpit" | "meilisearch" | "nginx")
        || name
            .strip_prefix("php@")
            .is_some_and(|version| !version.is_empty())
    {
        Ok(())
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(
                json!({"error":{"code":"service_not_found","message":format!("unknown service: {name}")}}),
            ),
        ))
    }
}
async fn service_config(
    State(state): State<Arc<ApiState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> ApiResult {
    authorize(&headers, &state)?;
    if name != "nginx" {
        supported_service(&name)?;
        return Err(error(OrchestError::InvalidInput(format!(
            "configuration preview is unavailable for {name}"
        ))));
    }
    Ok(Json(
        json!({"name":name,"config":state.orchest.nginx_config().map_err(error)?}),
    ))
}
async fn service_status(
    State(state): State<Arc<ApiState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> ApiResult {
    authorize(&headers, &state)?;
    supported_service(&name)?;
    Ok(Json(
        json!({"name":name,"status":state.orchest.service_status(&name).map_err(error)?}),
    ))
}
async fn start_service(
    State(state): State<Arc<ApiState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> ApiResult {
    authorize(&headers, &state)?;
    supported_service(&name)?;
    let process = tokio::task::spawn_blocking(move || state.orchest.start_service(&name))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":{"code":"task_error","message":e.to_string()}})),
            )
        })?
        .map_err(error)?;
    Ok(Json(json!(process)))
}
async fn stop_service(
    State(state): State<Arc<ApiState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> ApiResult {
    authorize(&headers, &state)?;
    supported_service(&name)?;
    let service_name = name.clone();
    let status = tokio::task::spawn_blocking(move || state.orchest.stop_service(&service_name))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":{"code":"task_error","message":e.to_string()}})),
            )
        })?
        .map_err(error)?;
    Ok(Json(json!({"name":name,"status":status})))
}
async fn doctor(State(state): State<Arc<ApiState>>, headers: HeaderMap) -> ApiResult {
    authorize(&headers, &state)?;
    Ok(Json(json!(state.orchest.doctor())))
}
async fn ports(State(state): State<Arc<ApiState>>, headers: HeaderMap) -> ApiResult {
    authorize(&headers, &state)?;
    Ok(Json(json!(state.orchest.ports().map_err(error)?)))
}
async fn check_port(
    State(state): State<Arc<ApiState>>,
    Path(port): Path<u16>,
    headers: HeaderMap,
) -> ApiResult {
    authorize(&headers, &state)?;
    Ok(Json(json!(state
        .orchest
        .check_port(port)
        .map_err(error)?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bearer_token_is_required() {
        let root = tempfile::tempdir().unwrap();
        let manifests = tempfile::tempdir().unwrap();
        let orchest = Orchest::init(root.path().to_path_buf(), manifests.path()).unwrap();
        let state = ApiState {
            orchest,
            token: "secret".into(),
        };
        assert!(authorize(&HeaderMap::new(), &state).is_err());
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer secret".parse().unwrap());
        assert!(authorize(&headers, &state).is_ok());
    }
}
