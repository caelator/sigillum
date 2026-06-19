//! Route tree for the gateway API.

pub mod health;
pub mod payments;
pub mod projects;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::ConnectInfo;
use axum::http::{HeaderValue, header};
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use tokio::sync::Mutex;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;

use crate::auth::{
    require_admin_key, require_api_key, require_payments_cancel, require_payments_create,
    require_payments_list, require_payments_read,
};
use crate::state::AppState;

const RATE_LIMIT_BUCKET_TTL: Duration = Duration::from_secs(10 * 60);
const RATE_LIMIT_CLEANUP_INTERVAL: u64 = 1_000;

async fn security_headers_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let is_api = req.uri().path().starts_with("/api/");
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    if is_api && !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store, no-cache, must-revalidate"),
        );
    }
    resp
}

/// Build the full router with all security layers.
pub fn build_router(state: AppState) -> Router {
    // ── CORS (S4) ──────────────────────────────────────────────────
    let cors = if state.config.cors_origins.is_empty() {
        CorsLayer::new()
    } else {
        let origins: Vec<_> = state
            .config
            .cors_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
            ])
    };

    // ── Rate limiter state (token bucket per IP) ───────────────────
    let rate_limiter = if state.config.rate_limit_rps > 0 {
        Some(Arc::new(Mutex::new(RateLimiter::new(
            state.config.rate_limit_rps,
        ))))
    } else {
        None
    };

    // ── Public routes (no auth) ────────────────────────────────────
    let public = Router::new().route("/api/v1/health", get(health::health_check));

    // ── Admin routes (admin API key required — S1) ─────────────────
    let admin = Router::new()
        .route("/api/v1/projects", post(projects::create_project))
        .route(
            "/api/v1/projects/{id}/scopes",
            axum::routing::patch(projects::update_project_scopes),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_admin_key,
        ));

    // ── Authenticated routes (project API key required) ────────────
    let authenticated = Router::new()
        .route(
            "/api/v1/projects/{id}",
            get(projects::get_project).route_layer(middleware::from_fn(require_payments_read)),
        )
        .route(
            "/api/v1/payments",
            post(payments::create_payment)
                .route_layer(middleware::from_fn(require_payments_create)),
        )
        .route(
            "/api/v1/payments",
            get(payments::list_payments).route_layer(middleware::from_fn(require_payments_list)),
        )
        .route(
            "/api/v1/payments/{id}",
            get(payments::get_payment).route_layer(middleware::from_fn(require_payments_read)),
        )
        .route(
            "/api/v1/payments/{id}/cancel",
            post(payments::cancel_payment)
                .route_layer(middleware::from_fn(require_payments_cancel)),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ));

    // ── Static widget files (embedded at compile time — R2) ────────
    let widget = Router::new()
        .route("/widget/widget.js", get(serve_widget_js))
        .route("/widget/widget.css", get(serve_widget_css));

    let mut app = public
        .merge(admin)
        .merge(authenticated)
        .merge(widget)
        // S2: Request body size limit (1 MB)
        .layer(RequestBodyLimitLayer::new(1_048_576))
        // S4: CORS
        .layer(cors)
        .with_state(state);

    // P3: Rate limiting middleware (if configured)
    if let Some(limiter) = rate_limiter {
        app = app.layer(middleware::from_fn(move |req, next| {
            let limiter = limiter.clone();
            rate_limit_middleware(limiter, req, next)
        }));
    }

    app.layer(middleware::from_fn(security_headers_middleware))
}

// ── Embedded static files (R2) with Cache-Control (P3) ─────────────

static WIDGET_JS: &str = include_str!("../../static/widget.js");
static WIDGET_CSS: &str = include_str!("../../static/widget.css");

async fn serve_widget_js() -> (
    [(axum::http::header::HeaderName, &'static str); 2],
    &'static str,
) {
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/javascript"),
            (axum::http::header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        WIDGET_JS,
    )
}

async fn serve_widget_css() -> (
    [(axum::http::header::HeaderName, &'static str); 2],
    &'static str,
) {
    (
        [
            (axum::http::header::CONTENT_TYPE, "text/css"),
            (axum::http::header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        WIDGET_CSS,
    )
}

// ── Simple token-bucket rate limiter (P3) ──────────────────────────

use std::collections::HashMap;

struct RateLimiter {
    buckets: HashMap<String, TokenBucket>,
    max_rps: u64,
    checks: u64,
}

struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    fn new(max_rps: u64) -> Self {
        Self {
            buckets: HashMap::new(),
            max_rps,
            checks: 0,
        }
    }

    fn check(&mut self, key: &str) -> bool {
        let now = Instant::now();
        self.checks += 1;
        if self.checks % RATE_LIMIT_CLEANUP_INTERVAL == 0 {
            self.buckets
                .retain(|_, bucket| now.duration_since(bucket.last_refill) < RATE_LIMIT_BUCKET_TTL);
        }
        let max = self.max_rps as f64;
        let bucket = self.buckets.entry(key.to_string()).or_insert(TokenBucket {
            tokens: max,
            last_refill: now,
        });

        // Refill tokens based on elapsed time
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * max).min(max * 2.0); // burst allowance = 2x
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

async fn rate_limit_middleware(
    limiter: Arc<Mutex<RateLimiter>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let key = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
        .unwrap_or_else(|| "unknown".into());

    let allowed = {
        let mut guard = limiter.lock().await;
        guard.check(&key)
    };

    if allowed {
        next.run(req).await
    } else {
        (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            axum::Json(serde_json::json!({
                "error": "rate limit exceeded"
            })),
        )
            .into_response()
    }
}
