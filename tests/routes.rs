use axum::body::Body;
use axum::extract::Json as JsonExtract;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router as AxumRouter};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use srvcs_rank::{api::Deps, health, router, telemetry};
use tower::ServiceExt;

const DEAD_URL: &str = "http://127.0.0.1:1";

/// A COMPUTING mock for any srvcs primitive `rank` (or its sibling
/// orchestrators) might depend on. It reads the operands from the request and
/// returns a real `result`, so a composition is genuinely tested rather than
/// faked. `rank` itself only uses the `lessthan` shape, but the full table is
/// provided so the mock is faithful to the srvcs contract:
///
/// - lessthan/greaterthan -> real boolean comparison
/// - abs -> |x|            - compare -> -1/0/1
/// - subtract -> a-b       - floatadd -> a+b      - floatdivide -> a/b
/// - floatmultiply -> a*b  - floatsubtract -> a-b - percentage -> a/b*100
/// - sortascending -> the sorted array
fn computing(op: &'static str) -> AxumRouter {
    AxumRouter::new().route(
        "/",
        post(move |JsonExtract(req): JsonExtract<Value>| async move {
            let a_i = req["a"].as_i64();
            let b_i = req["b"].as_i64();
            let a_f = req["a"].as_f64();
            let b_f = req["b"].as_f64();
            let x_i = req["value"].as_i64();
            let result: Value = match op {
                "lessthan" => json!(a_i.unwrap_or(0) < b_i.unwrap_or(0)),
                "greaterthan" => json!(a_i.unwrap_or(0) > b_i.unwrap_or(0)),
                "abs" => json!(x_i.unwrap_or(0).abs()),
                "compare" => {
                    let (a, b) = (a_i.unwrap_or(0), b_i.unwrap_or(0));
                    json!(a.cmp(&b) as i64)
                }
                "subtract" => json!(a_i.unwrap_or(0) - b_i.unwrap_or(0)),
                "floatadd" => json!(a_f.unwrap_or(0.0) + b_f.unwrap_or(0.0)),
                "floatdivide" => json!(a_f.unwrap_or(0.0) / b_f.unwrap_or(1.0)),
                "floatmultiply" => json!(a_f.unwrap_or(0.0) * b_f.unwrap_or(0.0)),
                "floatsubtract" => json!(a_f.unwrap_or(0.0) - b_f.unwrap_or(0.0)),
                "percentage" => json!(a_f.unwrap_or(0.0) / b_f.unwrap_or(1.0) * 100.0),
                "sortascending" => {
                    let mut vs: Vec<i64> = req["values"]
                        .as_array()
                        .map(|a| a.iter().filter_map(Value::as_i64).collect())
                        .unwrap_or_default();
                    vs.sort_unstable();
                    json!(vs)
                }
                _ => Value::Null,
            };
            let mut out = req;
            out["result"] = result;
            Json(out)
        }),
    )
}

/// Mock that always answers with a fixed status + body (used to simulate a
/// `422` rejection of a bad operand).
async fn spawn_fixed(status: StatusCode, body: Value) -> String {
    let app = AxumRouter::new().route(
        "/",
        post(move || {
            let body = body.clone();
            async move { (status, Json(body)) }
        }),
    );
    serve(app).await
}

async fn spawn_lessthan() -> String {
    serve(computing("lessthan")).await
}

async fn serve(app: AxumRouter) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn app(lessthan_url: &str) -> axum::Router {
    router(
        telemetry::metrics_handle_for_tests(),
        Deps {
            lessthan_url: lessthan_url.to_string(),
        },
    )
}

async fn eval(lessthan_url: &str, values: Value, value: Value) -> (StatusCode, Value) {
    let res = app(lessthan_url)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "values": values, "value": value }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn status_of(uri: &str) -> StatusCode {
    app(DEAD_URL)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn healthz_ok() {
    assert_eq!(status_of("/healthz").await, StatusCode::OK);
}

#[tokio::test]
async fn readyz_reflects_state() {
    health::set_ready(true);
    assert_eq!(status_of("/readyz").await, StatusCode::OK);
}

#[tokio::test]
async fn openapi_ok() {
    assert_eq!(status_of("/openapi.json").await, StatusCode::OK);
}

// --- Correctness cases from the spec, against a REAL computing lessthan ---

#[tokio::test]
async fn rank_of_present_value_counts_strictly_less_plus_one() {
    let lt = spawn_lessthan().await;
    let (status, body) = eval(&lt, json!([10, 20, 30]), json!(20)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], 2);
    assert_eq!(body["values"], json!([10, 20, 30]));
    assert_eq!(body["value"], 20);
}

#[tokio::test]
async fn rank_of_smallest_is_one() {
    let lt = spawn_lessthan().await;
    let (status, body) = eval(&lt, json!([10, 20, 30]), json!(5)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], 1);
}

#[tokio::test]
async fn rank_of_largest_is_count_plus_one() {
    let lt = spawn_lessthan().await;
    let (status, body) = eval(&lt, json!([10, 20, 30]), json!(40)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], 4);
}

#[tokio::test]
async fn duplicates_strictly_less_are_each_counted() {
    let lt = spawn_lessthan().await;
    let (status, body) = eval(&lt, json!([5, 5, 5, 20]), json!(10)).await;
    assert_eq!(status, StatusCode::OK);
    // three 5s are strictly less than 10; rank = 3 + 1 = 4
    assert_eq!(body["result"], 4);
}

#[tokio::test]
async fn equal_elements_do_not_count() {
    let lt = spawn_lessthan().await;
    let (status, body) = eval(&lt, json!([20, 20, 20]), json!(20)).await;
    assert_eq!(status, StatusCode::OK);
    // none strictly less; rank = 0 + 1 = 1
    assert_eq!(body["result"], 1);
}

#[tokio::test]
async fn rank_against_empty_list_is_one_with_no_calls() {
    // DEAD_URL: if rank tried to call lessthan at all on an empty list, this
    // would degrade to 503. It must short-circuit to 1 with no calls.
    let (status, body) = eval(DEAD_URL, json!([]), json!(7)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], 1);
    assert_eq!(body["values"], json!([]));
}

// --- Error / edge cases ---

#[tokio::test]
async fn forwards_422_for_bad_operand() {
    let lt = spawn_fixed(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "error": "value is not an integer" }),
    )
    .await;
    let (status, body) = eval(&lt, json!([1, "nope", 3]), json!(2)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "value is not an integer");
}

#[tokio::test]
async fn degrades_when_lessthan_is_unreachable() {
    let (status, body) = eval(DEAD_URL, json!([1, 2, 3]), json!(2)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-lessthan");
}
