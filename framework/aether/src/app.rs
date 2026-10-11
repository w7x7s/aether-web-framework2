use aether_core::arena::RequestArena;
use aether_core::diagnostic::{Diagnostic, ErrorCode, ServerBanner};
use aether_core::error::AetherError;
use aether_http::limits::ServerLimits;
use aether_http::method::Method;
use aether_http::parser::RequestView;
use aether_http::response::Response;
use aether_middleware::{CorsMiddleware, LoggerMiddleware, SecurityHeadersMiddleware};
use aether_router::trie::{MatchResult, RadixRouter};
use bytes::{Bytes, BytesMut};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::config::AetherConfig;

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Trait converting various types into a unified HTTP Response.
pub trait IntoResponse: Send {
    fn into_response(self) -> Response;
}

impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}

impl IntoResponse for &'static str {
    fn into_response(self) -> Response {
        Response::text(self)
    }
}

impl IntoResponse for String {
    fn into_response(self) -> Response {
        let bytes = Bytes::from(self);
        Response::bytes(bytes, "text/plain; charset=utf-8")
    }
}

impl IntoResponse for (u16, &'static str) {
    fn into_response(self) -> Response {
        Response::status(self.0, self.1)
    }
}

impl IntoResponse for (u16, String) {
    fn into_response(self) -> Response {
        let mut res = Response::status(self.0, "Custom");
        res.headers.push(("Content-Type", "text/plain; charset=utf-8".to_string()));
        res.headers.push(("Content-Length", self.1.len().to_string()));
        res.body = Bytes::from(self.1);
        res
    }
}

impl<T, E> IntoResponse for Result<T, E>
where
    T: IntoResponse,
    E: std::fmt::Display + Send,
{
    fn into_response(self) -> Response {
        match self {
            Ok(val) => val.into_response(),
            Err(err) => {
                let err_msg = format!(r#"{{"error":"{}","code":"E5002"}}"#, err);
                let mut res = Response::json(Bytes::from(err_msg));
                res.status = 400;
                res.status_text = "Bad Request";
                res
            }
        }
    }
}

/// حاوية الحالة العامة المشتركة للتطبيق والآمنة للاستخدام عبر الـ Threads المتعددة (Thread-Safe Dependency Container).
/// 
/// تستخدم لتطبيق نمط حقن التبعيات (Dependency Injection) بالاعتماد على `TypeId`،
/// مما يسمح بمشاركة اتصالات قواعد البيانات، المخازن المؤقتة، والخدمات المشتركة بين جميع المعالجات.
#[derive(Default, Clone)]
pub struct AppStateContainer {
    items: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl AppStateContainer {
    /// إنشاء حاوية جديدة فارغة لتسجيل التبعيات والخدمات.
    pub fn new() -> Self {
        Self {
            items: HashMap::new(),
        }
    }

    /// تسجيل خدمة أو كائن حالة جديد داخل الحاوية مع تغليفه بـ `Arc`.
    pub fn insert<T: Send + Sync + 'static>(&mut self, value: T) {
        self.items.insert(TypeId::of::<T>(), Arc::new(value));
    }

    /// استرجاع مرجع ذري `Arc<T>` للخدمة المطلوبة بناءً على نوعها في وقت التشغيل بـ $O(1)$.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.items
            .get(&TypeId::of::<T>())
            .and_then(|any| any.clone().downcast::<T>().ok())
    }
}

/// سياق الطلب (`RequestContext`) الممرر لكل معالج مسار (Route Handler).
///
/// يحتوي على جميع بيانات الطلب المنقاة مع ملكية منضبطة تمنع تسريبات الذاكرة (No `Box::leak`).
/// يوفر دوال مساعدة لاستخراج المعلمات، الترويسات، وفك تشفير الـ JSON بسرعة فائقة وبدون عمليات نسخ للذاكرة.
pub struct RequestContext {
    pub raw_uri: String,
    pub path: String,
    pub query_str: Option<String>,
    pub method: Method,
    pub params: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Bytes,
    pub request_id: String,
    state: Arc<AppStateContainer>,
}

impl RequestContext {
    /// استخراج قيمة معلمة مسار ديناميكية (Dynamic Route Parameter) مثل `:id` أو `{id}`
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(|s| s.as_str())
    }

    /// استخراج معلمة المسار وتحويلها تلقائياً إلى نوع رقمي أو منطقي محدد (مثل `i32`, `u64`, `bool`).
    pub fn param_parse<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.param(key).and_then(|v| v.parse::<T>().ok())
    }

    /// استخراج قيمة معلمة استعلام (Query Parameter) من الرابط (مثل `?page=2&filter=active`).
    pub fn query(&self, key: &str) -> Option<&str> {
        let q = self.query_str.as_deref()?;
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == key {
                    return Some(v);
                }
            } else if pair == key {
                return Some("");
            }
        }
        None
    }

    /// Parse a query parameter into a typed value (e.g. `i32`, `u64`, `bool`, etc.)
    pub fn query_parse<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.query(key).and_then(|v| v.parse::<T>().ok())
    }

    /// Retrieve all query parameters as a Map
    pub fn queries(&self) -> HashMap<&str, &str> {
        let mut map = HashMap::new();
        if let Some(ref q) = self.query_str {
            for pair in q.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    map.insert(k, v);
                } else if !pair.is_empty() {
                    map.insert(pair, "");
                }
            }
        }
        map
    }

    /// Parses URL-encoded form data (`application/x-www-form-urlencoded`) from request body into key-value map
    pub fn form_data(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        if let Ok(body_str) = self.body_str() {
            for pair in body_str.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    let decoded_k = url_decode_simple(k);
                    let decoded_v = url_decode_simple(v);
                    map.insert(decoded_k, decoded_v);
                } else if !pair.is_empty() {
                    map.insert(url_decode_simple(pair), String::new());
                }
            }
        }
        map
    }

    /// Retrieve an HTTP header case-insensitively
    pub fn header(&self, name: &str) -> Option<&str> {
        for (k, v) in &self.headers {
            if k.eq_ignore_ascii_case(name) {
                return Some(v.as_str());
            }
        }
        None
    }

    /// قراءة قيمة ملف تعريف الارتباط (Cookie) بالاسم من ترويسة `Cookie`
    pub fn cookie(&self, name: &str) -> Option<&str> {
        let cookie_hdr = self.header("cookie")?;
        for part in cookie_hdr.split(';') {
            let trimmed = part.trim();
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == name {
                    return Some(v.trim());
                }
            }
        }
        None
    }

    /// استخراج جسم الطلب (Body) كنص UTF-8 مباشرة بدون أي عمليات نسخ للذاكرة.
    pub fn body_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.body)
    }

    /// فك تشفير جسم الطلب من صيغة JSON مباشرة إلى بنية بيانات Rust محددة النوع.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    /// استرجاع مرجع الخدمة أو الحالة المشتركة المحقونة بنظام الـ DI عبر نوعها `T`.
    pub fn state<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.state.get::<T>()
    }

    /// الحصول على المعرّف الفريد لتتبع هذا الطلب (Request Trace ID).
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}

pub type BoxFutureResponse = Pin<Box<dyn Future<Output = Response> + Send + 'static>>;
pub type AsyncHandlerFn = Arc<dyn Fn(RequestContext) -> BoxFutureResponse + Send + Sync + 'static>;

/// كائن تطبيق Aether الأساسي والمسؤول عن إدارة المسارات، البرمجيات الوسيطة، وحلقة تشغيل الخادم.
pub struct AetherApp {
    router: RadixRouter,
    handlers: HashMap<u32, AsyncHandlerFn>,
    next_id: u32,
    limits: ServerLimits,
    state: AppStateContainer,
    cors: Option<CorsMiddleware>,
    security: Option<SecurityHeadersMiddleware>,
    logger: LoggerMiddleware,
    config: AetherConfig,
}

impl Default for AetherApp {
    fn default() -> Self {
        Self::new()
    }
}

impl AetherApp {
    /// Create a new application instance with default configuration
    pub fn new() -> Self {
        Self::with_config(AetherConfig::default())
    }

    /// Create an application instance with custom configuration
    pub fn with_config(config: AetherConfig) -> Self {
        let mut limits = ServerLimits::default();
        limits.max_body_bytes = config.max_body_bytes;
        limits.max_header_bytes = config.max_header_bytes;

        Self {
            router: RadixRouter::new(),
            handlers: HashMap::new(),
            next_id: 1,
            limits,
            state: AppStateContainer::new(),
            cors: None,
            security: None,
            logger: LoggerMiddleware::new(),
            config,
        }
    }

    /// Register managed application state
    pub fn manage<T: Send + Sync + 'static>(&mut self, state: T) -> &mut Self {
        self.state.insert(state);
        self
    }

    /// Enable CORS middleware
    pub fn enable_cors(&mut self, cors: CorsMiddleware) -> &mut Self {
        self.cors = Some(cors);
        self
    }

    /// Enable Security Headers middleware
    pub fn enable_security_headers(&mut self) -> &mut Self {
        self.security = Some(SecurityHeadersMiddleware::new());
        self
    }

    /// Register a synchronous or asynchronous GET route
    pub fn get<F, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> R + Send + Sync + 'static,
        R: IntoResponse + 'static,
    {
        self.route_sync(Method::Get, path, handler)
    }

    /// Register an asynchronous GET route with `.await`
    pub fn get_async<F, Fut, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: IntoResponse + 'static,
    {
        self.route_async(Method::Get, path, handler)
    }

    /// Register a synchronous POST route
    pub fn post<F, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> R + Send + Sync + 'static,
        R: IntoResponse + 'static,
    {
        self.route_sync(Method::Post, path, handler)
    }

    /// Register an asynchronous POST route with `.await`
    pub fn post_async<F, Fut, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: IntoResponse + 'static,
    {
        self.route_async(Method::Post, path, handler)
    }

    /// Register a synchronous PUT route
    pub fn put<F, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> R + Send + Sync + 'static,
        R: IntoResponse + 'static,
    {
        self.route_sync(Method::Put, path, handler)
    }

    /// Register an asynchronous PUT route
    pub fn put_async<F, Fut, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: IntoResponse + 'static,
    {
        self.route_async(Method::Put, path, handler)
    }

    /// Register a synchronous DELETE route
    pub fn delete<F, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> R + Send + Sync + 'static,
        R: IntoResponse + 'static,
    {
        self.route_sync(Method::Delete, path, handler)
    }

    /// Register an asynchronous DELETE route
    pub fn delete_async<F, Fut, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: IntoResponse + 'static,
    {
        self.route_async(Method::Delete, path, handler)
    }

    /// Register a synchronous PATCH route
    pub fn patch<F, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> R + Send + Sync + 'static,
        R: IntoResponse + 'static,
    {
        self.route_sync(Method::Patch, path, handler)
    }

    /// Register an asynchronous PATCH route
    pub fn patch_async<F, Fut, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: IntoResponse + 'static,
    {
        self.route_async(Method::Patch, path, handler)
    }

    /// Register synchronous handler
    pub fn route_sync<F, R>(&mut self, method: Method, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> R + Send + Sync + 'static,
        R: IntoResponse + 'static,
    {
        let id = self.next_id;
        self.next_id += 1;
        self.router.add_route(method, path, id);
        let handler = Arc::new(handler);
        self.handlers.insert(
            id,
            Arc::new(move |ctx| {
                let h = Arc::clone(&handler);
                Box::pin(async move { h(ctx).into_response() })
            }),
        );
        self
    }

    /// Register asynchronous handler
    pub fn route_async<F, Fut, R>(&mut self, method: Method, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: IntoResponse + 'static,
    {
        let id = self.next_id;
        self.next_id += 1;
        self.router.add_route(method, path, id);
        let handler = Arc::new(handler);
        self.handlers.insert(
            id,
            Arc::new(move |ctx| {
                let h = Arc::clone(&handler);
                Box::pin(async move { h(ctx).await.into_response() })
            }),
        );
        self
    }

    /// Route Grouping with common prefix
    pub fn group<F>(&mut self, prefix: &str, configure: F) -> &mut Self
    where
        F: FnOnce(&mut RouteGroup),
    {
        let mut group = RouteGroup {
            app: self,
            prefix: prefix.to_string(),
        };
        configure(&mut group);
        self
    }

    /// Register a static byte asset route directly (e.g. css, js, images, icons)
    pub fn static_asset(&mut self, path: &str, content_type: &'static str, bytes: &'static [u8]) -> &mut Self {
        self.get(path, move |_| Response::bytes(Bytes::from_static(bytes), content_type))
    }

    /// Register a static file from disk to be served directly on a path with path traversal protection
    pub fn static_file(&mut self, path: &str, file_path: &'static str) -> &mut Self {
        self.get(path, move |_| {
            Response::file(file_path).unwrap_or_else(|_| Response::status(404, "File Not Found"))
        })
    }

    /// Start the server listening on the specified address with rich terminal banner and error handling
    pub async fn listen(mut self, addr: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.config.host = addr.split(':').next().unwrap_or("0.0.0.0").to_string();
        if let Some(port_str) = addr.split(':').nth(1) {
            if let Ok(p) = port_str.parse::<u16>() {
                self.config.port = p;
            }
        }
        let config = self.config.clone();
        self.listen_with_config(config).await
    }

    /// Start the server with full `AetherConfig`
    pub async fn listen_with_config(self, config: AetherConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = config.listen_addr();
        let routes_count = self.handlers.len();

        let banner = ServerBanner {
            env: if config.is_production() { "production" } else { "development" },
            address: format!("http://{}", addr),
            routes_count,
            workers: config.workers,
        };
        println!("{}", banner.render());

        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                let diag = Diagnostic::new(ErrorCode::E1001ServerStartupFailed, format!("TCP Bind failed: {}", e))
                    .with_context("Address", &addr)
                    .with_hint("Port is already occupied by another running application or requires elevated privileges.")
                    .with_suggestion(format!("Change port via AETHER_PORT environment variable (e.g. AETHER_PORT={})", config.port + 1))
                    .with_suggestion("Kill conflicting process: lsof -ti:3000 | xargs kill -9");
                eprintln!("{}", diag.format_terminal(true));
                return Err(Box::new(AetherError::Io(e)));
            }
        };

        let router = Arc::new(self.router);
        let handlers = Arc::new(self.handlers);
        let limits = Arc::new(self.limits);
        let state = Arc::new(self.state);
        let cors = self.cors.map(Arc::new);
        let security = self.security.map(Arc::new);
        let logger = Arc::new(self.logger);
        let conn_semaphore = Arc::new(tokio::sync::Semaphore::new(limits.max_connections));

        // حلقة استقبال اتصالات الـ TCP مع تقييد عدد الاتصالات المتزامنة (Semaphore Connection Limit)
        loop {
            let permit = match conn_semaphore.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => break Ok(()), // توقف الخادم
            };

            let (mut stream, _) = listener.accept().await?;
            let router_clone = Arc::clone(&router);
            let handlers_clone = Arc::clone(&handlers);
            let limits_clone = Arc::clone(&limits);
            let state_clone = Arc::clone(&state);
            let cors_clone = cors.clone();
            let security_clone = security.clone();
            let logger_clone = Arc::clone(&logger);

            // معالجة كل مقبس (Socket Connection) بشكل غير متزامن داخل Tokio Task مستقلة
            tokio::spawn(async move {
                let _permit = permit; // تصريح الـ RAII يتحرر تلقائياً عند إغلاق الاتصال
                let mut conn_buf = BytesMut::with_capacity(8192);
                let mut temp_buf = [0u8; 8192];
                let mut write_buf = BytesMut::with_capacity(4096);
                let arena = RequestArena::with_capacity(4096);

                loop {
                    // 1. محاولة تحليل أي بيانات مكتملة متبقية في البافر التراكمي (دعم Pipelining و Keep-Alive الفوري)
                    loop {
                        if conn_buf.is_empty() {
                            break;
                        }

                        let parse_result = match RequestView::parse(&conn_buf, &limits_clone) {
                            Ok(Some((req, consumed))) => {
                                let raw_uri = req.raw_uri.to_string();
                                let path = req.path.to_string();
                                let query_str = req.query_str.map(|s| s.to_string());
                                let method = req.method;
                                let mut header_map = HashMap::with_capacity(req.headers.len());
                                for h in req.headers.iter() {
                                    if let Ok(val) = std::str::from_utf8(h.value) {
                                        header_map.insert(h.name.to_string(), val.to_string());
                                    }
                                }
                                let body = Bytes::copy_from_slice(req.body);
                                Ok(Some((raw_uri, path, query_str, method, header_map, body, consumed)))
                            }
                            Ok(None) => Ok(None),
                            Err(err) => Err(err),
                        };

                        match parse_result {
                            Ok(Some((raw_uri, path_str, query_str, method_enum, header_map, body_bytes, consumed))) => {
                                let start_time = logger_clone.start();
                                let req_id_num = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
                                let req_id_str = format!("req_{:012x}", req_id_num);

                                // 2. مطابقة المسار عبر محرك الـ Radix Router في زمن O(K)
                                let mut response = match router_clone.find(method_enum, &path_str) {
                                    MatchResult::Found { handler_id, params } => {
                                        if let Some(handler) = handlers_clone.get(&handler_id) {
                                            let mut param_map = HashMap::with_capacity(params.len());
                                            for p in params {
                                                param_map.insert(p.key.to_string(), p.value.to_string());
                                            }

                                            let ctx = RequestContext {
                                                raw_uri,
                                                path: path_str.clone(),
                                                query_str,
                                                method: method_enum,
                                                params: param_map,
                                                headers: header_map,
                                                body: body_bytes,
                                                request_id: req_id_str.clone(),
                                                state: Arc::clone(&state_clone),
                                            };
                                            handler(ctx).await
                                        } else {
                                            Response::status(500, "Internal Server Error")
                                        }
                                    }
                                    MatchResult::MethodNotAllowed { .. } => {
                                        Response::status(405, "Method Not Allowed")
                                    }
                                    MatchResult::NotFound => {
                                        Response::status(404, "Not Found")
                                    }
                                };

                                response.headers.push(("X-Request-ID", req_id_str));

                                if let Some(ref c) = cors_clone {
                                    c.apply(&mut response);
                                }
                                if let Some(ref s) = security_clone {
                                    s.apply(&mut response);
                                }

                                logger_clone.log(
                                    &format!("{:?}", method_enum),
                                    &path_str,
                                    response.status,
                                    start_time,
                                );

                                write_buf.clear();
                                response.write_http1_to(&mut write_buf);
                                let write_future = async {
                                    stream.write_all(&write_buf).await?;
                                    stream.flush().await
                                };
                                if tokio::time::timeout(limits_clone.write_timeout, write_future).await.is_err() {
                                    return;
                                }

                                // Advance the buffer by the exact number of consumed bytes
                                let _ = conn_buf.split_to(consumed);
                                arena.reset();
                            }
                            Ok(None) => {
                                // Need more data from the network
                                break;
                            }
                            Err(err) => {
                                let mut err_res = match err {
                                    AetherError::HeaderTooLarge => Response::status(431, "Request Header Fields Too Large"),
                                    AetherError::BodyLimitExceeded => Response::status(413, "Payload Too Large"),
                                    _ => Response::status(400, "Bad Request"),
                                };
                                err_res.headers.push(("Connection", "close".to_string()));
                                write_buf.clear();
                                err_res.write_http1_to(&mut write_buf);
                                let _ = stream.write_all(&write_buf).await;
                                return;
                            }
                        }
                    }

                    // Read next TCP chunk with read timeout
                    let read_result = tokio::time::timeout(
                        limits_clone.read_timeout,
                        stream.read(&mut temp_buf),
                    ).await;

                    match read_result {
                        Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break, // Client closed or read timed out (Slowloris protection)
                        Ok(Ok(n)) => {
                            // Enforce total accumulated buffer limit to prevent memory exhaustion DoS
                            if conn_buf.len() + n > limits_clone.max_header_bytes + limits_clone.max_body_bytes {
                                let mut err_res = Response::status(413, "Payload Too Large");
                                err_res.headers.push(("Connection", "close".to_string()));
                                write_buf.clear();
                                err_res.write_http1_to(&mut write_buf);
                                let _ = stream.write_all(&write_buf).await;
                                break;
                            }
                            conn_buf.extend_from_slice(&temp_buf[..n]);
                        }
                    }
                }
            });
        }
    }
}

/// Helper struct for Route Grouping
pub struct RouteGroup<'a> {
    app: &'a mut AetherApp,
    prefix: String,
}

impl<'a> RouteGroup<'a> {
    pub fn get<F, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> R + Send + Sync + 'static,
        R: IntoResponse + 'static,
    {
        let full_path = format!("{}{}", self.prefix, path);
        self.app.get(&full_path, handler);
        self
    }

    pub fn get_async<F, Fut, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: IntoResponse + 'static,
    {
        let full_path = format!("{}{}", self.prefix, path);
        self.app.get_async(&full_path, handler);
        self
    }

    pub fn post<F, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> R + Send + Sync + 'static,
        R: IntoResponse + 'static,
    {
        let full_path = format!("{}{}", self.prefix, path);
        self.app.post(&full_path, handler);
        self
    }

    pub fn post_async<F, Fut, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: IntoResponse + 'static,
    {
        let full_path = format!("{}{}", self.prefix, path);
        self.app.post_async(&full_path, handler);
        self
    }

    pub fn put<F, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> R + Send + Sync + 'static,
        R: IntoResponse + 'static,
    {
        let full_path = format!("{}{}", self.prefix, path);
        self.app.put(&full_path, handler);
        self
    }

    pub fn put_async<F, Fut, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: IntoResponse + 'static,
    {
        let full_path = format!("{}{}", self.prefix, path);
        self.app.put_async(&full_path, handler);
        self
    }

    pub fn delete<F, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> R + Send + Sync + 'static,
        R: IntoResponse + 'static,
    {
        let full_path = format!("{}{}", self.prefix, path);
        self.app.delete(&full_path, handler);
        self
    }

    pub fn delete_async<F, Fut, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: IntoResponse + 'static,
    {
        let full_path = format!("{}{}", self.prefix, path);
        self.app.delete_async(&full_path, handler);
        self
    }

    pub fn patch<F, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> R + Send + Sync + 'static,
        R: IntoResponse + 'static,
    {
        let full_path = format!("{}{}", self.prefix, path);
        self.app.patch(&full_path, handler);
        self
    }

    pub fn patch_async<F, Fut, R>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: IntoResponse + 'static,
    {
        let full_path = format!("{}{}", self.prefix, path);
        self.app.patch_async(&full_path, handler);
        self
    }
}

fn url_decode_simple(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut bytes = input.bytes();
    while let Some(b) = bytes.next() {
        if b == b'+' {
            out.push(' ');
        } else if b == b'%' {
            if let (Some(h1), Some(h2)) = (bytes.next(), bytes.next()) {
                let hex_str = [h1, h2];
                if let Ok(s) = std::str::from_utf8(&hex_str) {
                    if let Ok(val) = u8::from_str_radix(s, 16) {
                        out.push(val as char);
                        continue;
                    }
                }
                out.push('%');
                out.push(h1 as char);
                out.push(h2 as char);
            } else {
                out.push('%');
            }
        } else {
            out.push(b as char);
        }
    }
    out
}
