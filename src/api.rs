use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::{OpenApi, ToSchema};

use crate::client::{self, DepError};

pub const SERVICE: &str = "srvcs-rank";
pub const CONCERN: &str = "comparison: 1-indexed rank of value within a list";
pub const DEPENDS_ON: &[&str] = &["srvcs-lessthan"];

/// Dependency endpoints, injected as router state so tests can point them at
/// mock services.
#[derive(Clone)]
pub struct Deps {
    pub lessthan_url: String,
}

#[derive(Serialize, ToSchema)]
pub struct Info {
    pub service: &'static str,
    pub concern: &'static str,
    pub depends_on: Vec<&'static str>,
}

/// `GET /` — service identity (srvcs service standard).
#[utoipa::path(get, path = "/", responses((status = 200, body = Info)))]
pub async fn index() -> Json<Info> {
    Json(Info {
        service: SERVICE,
        concern: CONCERN,
        depends_on: DEPENDS_ON.to_vec(),
    })
}

#[derive(Deserialize, ToSchema)]
pub struct EvalRequest {
    /// The list of integers to rank `value` against.
    #[schema(value_type = Object)]
    pub values: Vec<Value>,
    /// The value whose 1-indexed rank is computed.
    #[schema(value_type = Object)]
    pub value: Value,
}

#[derive(Serialize, ToSchema)]
pub struct RankResponse {
    #[schema(value_type = Object)]
    pub values: Vec<Value>,
    #[schema(value_type = Object)]
    pub value: Value,
    /// The 1-indexed rank: one plus the count of elements strictly less than
    /// `value`.
    pub result: i64,
}

fn ok(values: Vec<Value>, value: Value, result: i64) -> Response {
    (
        StatusCode::OK,
        Json(json!({ "values": values, "value": value, "result": result })),
    )
        .into_response()
}

fn degraded(dependency: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "dependency unavailable", "dependency": dependency })),
    )
        .into_response()
}

/// Forward a dependency's response verbatim (used to propagate `422` for invalid
/// input from a leaf dependency).
fn forward(status: u16, body: Value) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    (code, Json(body)).into_response()
}

/// Ask `srvcs-lessthan` whether `a < b`, mapping its failures to the response
/// this service should return: `503` if it is unreachable, the forwarded `422`
/// if it rejects an operand (e.g. a non-integer), and a generic `500` if it
/// returns an unusable body.
async fn ask_lessthan(url: &str, a: &Value, b: &Value) -> Result<bool, Response> {
    let body = json!({ "a": a, "b": b });
    match client::call(url, &body).await {
        Err(DepError::Unreachable) => Err(degraded("srvcs-lessthan")),
        Ok((200, body)) => match body.get("result").and_then(Value::as_bool) {
            Some(b) => Ok(b),
            None => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "srvcs-lessthan returned no boolean result" })),
            )
                .into_response()),
        },
        // Bad operand (e.g. not an integer) — lessthan already judged it; forward it.
        Ok((422, body)) => Err(forward(422, body)),
        Ok(_) => Err(degraded("srvcs-lessthan")),
    }
}

/// `POST /` — the 1-indexed rank of `value` within `values`.
///
/// This service does no comparison of its own. For each element `v` of `values`
/// it asks `srvcs-lessthan` whether `v < value`, counting how many are strictly
/// less. The rank is `count + 1`, so `rank([10,20,30], 20) == 2` and
/// `rank([10,20,30], 5) == 1`. If `lessthan` rejects an operand the `422` is
/// forwarded; if it is unreachable this service reports itself degraded rather
/// than guessing.
#[utoipa::path(
    post,
    path = "/",
    request_body = EvalRequest,
    responses(
        (status = 200, body = RankResponse),
        (status = 422, description = "an operand is not a valid integer (forwarded from srvcs-lessthan)"),
        (status = 500, description = "srvcs-lessthan returned an unusable response"),
        (status = 503, description = "the srvcs-lessthan dependency is unavailable")
    )
)]
pub async fn evaluate(State(deps): State<Deps>, Json(req): Json<EvalRequest>) -> Response {
    let mut count: i64 = 0;
    for v in &req.values {
        let less = match ask_lessthan(&deps.lessthan_url, v, &req.value).await {
            Ok(b) => b,
            Err(resp) => return resp,
        };
        if less {
            count += 1;
        }
    }
    ok(req.values, req.value, count + 1)
}

#[derive(OpenApi)]
#[openapi(
    paths(index, evaluate),
    components(schemas(Info, EvalRequest, RankResponse))
)]
pub struct ApiDoc;

/// Serve OpenAPI document
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_documents_routes() {
        let doc = ApiDoc::openapi();
        let root = doc.paths.paths.get("/").expect("path / present");
        assert!(root.get.is_some());
        assert!(root.post.is_some());
    }

    #[tokio::test]
    async fn index_reports_dependency() {
        let Json(info) = index().await;
        assert_eq!(info.service, "srvcs-rank");
        assert_eq!(
            info.concern,
            "comparison: 1-indexed rank of value within a list"
        );
        assert_eq!(info.depends_on, vec!["srvcs-lessthan"]);
    }
}
