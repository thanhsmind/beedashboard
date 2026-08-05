//! Terminal authentication (D4, P1, P2, P5) — the mechanism a later cell
//! will use to gate the terminal routes. This module ships the mechanism
//! and its unit tests only; no route is mounted here.
//!
//! Modeled on herdr-go's `src/web/auth.rs`: a static token compared in
//! constant time, a random in-memory session-id set behind an `HttpOnly`
//! `SameSite=Strict` cookie, and an opaque 404 on every auth failure so a
//! probing request never learns whether a route exists.
//!
//! Per P1 the token is never a `Config` field — `api_config`
//! (`crates/mdview/src/server.rs:180-189`) serializes the whole `Config` as
//! JSON on an unauthenticated route, so anything stored inside `Config`
//! would be one unauthenticated `GET /api/config` away regardless of what
//! the settings HTML masks. The token instead lives in its own file beside
//! the config (`<data_dir>/terminal.token`), created with owner-only
//! permissions where the platform supports them (unix `0600`).
//!
//! Per P2 the token is reveal-once: [`TerminalAuth::rotate`] — the only
//! call in this module that generates or replaces the token — returns it in
//! full; every other read ([`TerminalAuth::masked`]) exposes only its last
//! four characters. There is no "already shown" flag to get wrong: the full
//! token is simply never returned by any function other than `rotate`.
//!
//! Per P5 rotating the token clears every live session immediately — a
//! session cookie minted under the previous token stops working the instant
//! the new one is written.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::FromRequestParts;
use axum::http::{header, request::Parts, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

/// Cookie carrying the session id. A distinct name from herdr-go's
/// `hg_session`, since this is a different product surface sharing only the
/// mechanism.
const COOKIE_NAME: &str = "mdview_terminal_session";

/// File name for the token, written beside `config.toml` in the same data
/// directory (`~/.mdview/terminal.token` in production).
const TOKEN_FILE_NAME: &str = "terminal.token";

/// Random bytes in a freshly generated token (32 bytes -> 64 hex chars).
const TOKEN_BYTES: usize = 32;

/// `<data_dir>/terminal.token`, or `override_dir/terminal.token` when given.
/// Mirrors `mdview_core::config::config_path_override` so a route-level test
/// never touches the real `~/.mdview`.
pub fn token_path_override(override_dir: Option<&Path>) -> PathBuf {
    mdview_core::config::resolve_data_dir(override_dir).join(TOKEN_FILE_NAME)
}

/// The terminal auth mechanism: where the token lives on disk, plus the
/// in-memory set of sessions minted against it. Clones share the same
/// session set (via `Arc`), the way `AppState` is shared across handlers.
#[derive(Clone)]
pub struct TerminalAuth {
    data_dir: Option<PathBuf>,
    sessions: Arc<Mutex<HashSet<String>>>,
}

impl TerminalAuth {
    /// `data_dir` is the same injectable override the config/settings routes
    /// use (`AppState.config_data_dir`) — `None` resolves to the real
    /// `~/.mdview` in production, `Some(tmp)` keeps tests isolated.
    pub fn new(data_dir: Option<PathBuf>) -> Self {
        Self {
            data_dir,
            sessions: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn token_path(&self) -> PathBuf {
        token_path_override(self.data_dir.as_deref())
    }

    /// The token currently on disk, if one has ever been generated. Internal
    /// only — `verify` needs the full value to compare; nothing outside this
    /// module ever gets it this way. Use `masked` for a display-safe read.
    fn load_token(&self) -> Option<String> {
        fs::read_to_string(self.token_path())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Generate a fresh token, write it to disk (owner-only permissions
    /// where the platform supports them), and clear every live session
    /// (P5) so a cookie minted under the previous token stops working
    /// immediately. Returns the token in full — per P2, the only call in
    /// this module that ever does.
    pub fn rotate(&self) -> std::io::Result<String> {
        let token = generate_token();
        let path = self.token_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_token_file(&path, &token)?;
        self.sessions.lock().unwrap().clear();
        Ok(token)
    }

    /// True once a token has been generated at least once.
    pub fn is_configured(&self) -> bool {
        self.load_token().is_some()
    }

    /// The token masked to its last four characters, for every render after
    /// the one call to `rotate` that returned it in full (P2). `None` when
    /// no token has ever been generated.
    pub fn masked(&self) -> Option<String> {
        self.load_token().map(|t| mask(&t))
    }

    /// Constant-time check of `presented` against the token on disk. A
    /// missing token (never configured) fails closed.
    pub fn verify(&self, presented: &str) -> bool {
        match self.load_token() {
            Some(configured) => constant_time_eq(presented.as_bytes(), configured.as_bytes()),
            None => false,
        }
    }

    /// Mint and record a new session id.
    pub fn mint_session(&self) -> String {
        let id = new_session_id();
        self.sessions.lock().unwrap().insert(id.clone());
        id
    }

    /// Whether `id` is a currently live session.
    pub fn session_valid(&self, id: &str) -> bool {
        self.sessions.lock().unwrap().contains(id)
    }

    /// Drop one session (logout). Idempotent.
    pub fn end_session(&self, id: &str) {
        self.sessions.lock().unwrap().remove(id);
    }
}

/// Last four characters of `token`, with the rest replaced by `*`. Shorter
/// inputs mask entirely (defensive; `generate_token` never produces fewer
/// than four characters).
fn mask(token: &str) -> String {
    let n = token.chars().count();
    if n <= 4 {
        "*".repeat(n)
    } else {
        let visible: String = token.chars().skip(n - 4).collect();
        format!("{}{}", "*".repeat(n - 4), visible)
    }
}

fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn new_session_id() -> String {
    generate_token()
}

/// Write `token` to `path`, creating the file with owner-only permissions
/// where the platform supports them (unix `0600`). Windows has no
/// equivalent mode bit; the file is written with the platform default there.
fn write_token_file(path: &Path, token: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(token.as_bytes())?;
        // A pre-existing file (e.g. from an older run) keeps its prior mode
        // across a truncate+write — set it explicitly so rotation always
        // leaves 0600 behind, not only first creation.
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, token)
    }
}

/// Constant-time byte comparison — walks every byte regardless of an early
/// mismatch, so timing does not leak how much of the token was correct.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Implemented by whatever application state exposes the shared
/// [`TerminalAuth`], so [`AuthSession`] can extract from it without this
/// module depending on `AppState`. A later cell implements this for
/// `AppState` when it mounts the terminal routes.
pub trait HasTerminalAuth {
    fn terminal_auth(&self) -> &TerminalAuth;
}

/// Extractor proving a request carries a live terminal session. Generic
/// over any state `S` exposing [`HasTerminalAuth`]. On any failure (no
/// cookie, unknown or rotated-away session id) it short-circuits with an
/// opaque 404 — the caller never learns whether the route exists, only that
/// a bad or missing credential was presented.
pub struct AuthSession;

#[axum::async_trait]
impl<S> FromRequestParts<S> for AuthSession
where
    S: HasTerminalAuth + Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        if let Some(sid) = session_cookie(&parts.headers) {
            if state.terminal_auth().session_valid(&sid) {
                return Ok(AuthSession);
            }
        }
        Err(opaque_404())
    }
}

/// The response every auth failure returns: a plain 404 indistinguishable
/// from a route that does not exist.
pub fn opaque_404() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

/// The `Set-Cookie` header value for a freshly minted session.
pub fn session_cookie_header(session_id: &str) -> String {
    format!("{COOKIE_NAME}={session_id}; HttpOnly; SameSite=Strict; Path=/; Max-Age=604800")
}

/// The `Set-Cookie` header value that expires the session cookie (logout).
pub fn expired_cookie_header() -> String {
    format!("{COOKIE_NAME}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0")
}

/// Extract the session cookie's value from a request's `Cookie` header, if present.
pub fn session_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        let pair = pair.trim();
        if let Some(v) = pair.strip_prefix(&format!("{COOKIE_NAME}=")) {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Every test gets its own scratch data dir, so nothing here ever
    /// touches the developer's real `~/.mdview`.
    fn scratch_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "mdview-terminal-auth-{label}-{}-{n}",
            std::process::id()
        ))
    }

    #[test]
    fn constant_time_eq_matches_semantics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[test]
    fn mask_shows_only_last_four() {
        assert_eq!(mask("abcdef1234"), "******1234");
        assert_eq!(mask("ab"), "**");
        assert_eq!(mask("abcd"), "****");
        assert_eq!(mask("abcde"), "*bcde");
    }

    #[test]
    fn not_configured_until_rotate_is_called() {
        let dir = scratch_dir("not-configured");
        let auth = TerminalAuth::new(Some(dir.clone()));
        assert!(!auth.is_configured());
        assert_eq!(auth.masked(), None);
        assert!(!auth.verify("anything"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rotate_returns_full_token_and_masked_shows_only_last_four() {
        let dir = scratch_dir("reveal-once");
        let auth = TerminalAuth::new(Some(dir.clone()));
        let full = auth.rotate().unwrap();
        assert_eq!(full.len(), TOKEN_BYTES * 2);

        // Every read *other than* the rotate() return value is masked.
        let masked = auth.masked().unwrap();
        assert_ne!(masked, full);
        assert!(masked.ends_with(&full[full.len() - 4..]));
        assert!(masked.starts_with(&"*".repeat(full.len() - 4)));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn verify_accepts_the_configured_token_and_rejects_others() {
        let dir = scratch_dir("verify");
        let auth = TerminalAuth::new(Some(dir.clone()));
        let token = auth.rotate().unwrap();
        assert!(auth.verify(&token));
        assert!(!auth.verify("wrong-token"));
        assert!(!auth.verify(""));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rotating_replaces_the_token_so_the_old_value_no_longer_verifies() {
        let dir = scratch_dir("rotate-replaces");
        let auth = TerminalAuth::new(Some(dir.clone()));
        let first = auth.rotate().unwrap();
        let second = auth.rotate().unwrap();
        assert_ne!(first, second);
        assert!(!auth.verify(&first));
        assert!(auth.verify(&second));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rotating_clears_every_live_session_immediately() {
        let dir = scratch_dir("rotate-clears-sessions");
        let auth = TerminalAuth::new(Some(dir.clone()));
        auth.rotate().unwrap();
        let sid = auth.mint_session();
        assert!(auth.session_valid(&sid));

        auth.rotate().unwrap();
        assert!(
            !auth.session_valid(&sid),
            "a session minted under the previous token must not survive rotation"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mint_and_end_session_round_trip() {
        let dir = scratch_dir("session-lifecycle");
        let auth = TerminalAuth::new(Some(dir.clone()));
        auth.rotate().unwrap();
        let sid = auth.mint_session();
        assert!(auth.session_valid(&sid));
        auth.end_session(&sid);
        assert!(!auth.session_valid(&sid));
        // Ending an unknown session is a no-op, not an error.
        auth.end_session("never-existed");
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn token_file_is_created_owner_only_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch_dir("perms");
        let auth = TerminalAuth::new(Some(dir.clone()));
        auth.rotate().unwrap();
        let path = token_path_override(Some(&dir));
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "token file must be owner-read/write only");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn token_path_override_lives_beside_config() {
        let dir = scratch_dir("token-path");
        let token_path = token_path_override(Some(&dir));
        let config_path = mdview_core::config::config_path_override(Some(&dir));
        assert_eq!(token_path.parent(), config_path.parent());
        assert_eq!(token_path.file_name().unwrap(), "terminal.token");
    }

    #[test]
    fn session_cookie_header_is_http_only_and_same_site_strict() {
        let header = session_cookie_header("abc123");
        assert!(header.contains("HttpOnly"));
        assert!(header.contains("SameSite=Strict"));
        assert!(header.starts_with("mdview_terminal_session=abc123;"));
    }

    #[test]
    fn expired_cookie_header_clears_the_cookie() {
        let header = expired_cookie_header();
        assert!(header.contains("Max-Age=0"));
        assert!(header.starts_with("mdview_terminal_session=;"));
    }

    #[test]
    fn session_cookie_reads_the_named_cookie_only() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "other=1; mdview_terminal_session=the-sid; another=2"
                .parse()
                .unwrap(),
        );
        assert_eq!(session_cookie(&headers).as_deref(), Some("the-sid"));

        let empty = HeaderMap::new();
        assert_eq!(session_cookie(&empty), None);
    }

    /// A minimal state implementing `HasTerminalAuth`, standing in for
    /// `AppState` before a later cell wires this extractor into it.
    #[derive(Clone)]
    struct TestState {
        auth: TerminalAuth,
    }

    impl HasTerminalAuth for TestState {
        fn terminal_auth(&self) -> &TerminalAuth {
            &self.auth
        }
    }

    async fn guarded(_: AuthSession) -> &'static str {
        "ok"
    }

    fn test_router(state: TestState) -> axum::Router {
        axum::Router::new()
            .route("/guarded", axum::routing::get(guarded))
            .with_state(state)
    }

    #[tokio::test]
    async fn missing_session_cookie_is_opaque_404_not_401() {
        use tower::ServiceExt;
        let dir = scratch_dir("extractor-missing");
        let auth = TerminalAuth::new(Some(dir.clone()));
        auth.rotate().unwrap();
        let app = test_router(TestState { auth });

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/guarded")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn unknown_session_cookie_is_opaque_404() {
        use tower::ServiceExt;
        let dir = scratch_dir("extractor-unknown");
        let auth = TerminalAuth::new(Some(dir.clone()));
        auth.rotate().unwrap();
        let app = test_router(TestState { auth });

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/guarded")
                    .header(header::COOKIE, "mdview_terminal_session=not-a-real-session")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn valid_session_cookie_passes_through() {
        use tower::ServiceExt;
        let dir = scratch_dir("extractor-valid");
        let auth = TerminalAuth::new(Some(dir.clone()));
        auth.rotate().unwrap();
        let sid = auth.mint_session();
        let app = test_router(TestState { auth });

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/guarded")
                    .header(header::COOKIE, format!("mdview_terminal_session={sid}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn session_cookie_minted_before_rotation_is_refused_after(
    ) {
        use tower::ServiceExt;
        let dir = scratch_dir("extractor-rotated-away");
        let auth = TerminalAuth::new(Some(dir.clone()));
        auth.rotate().unwrap();
        let sid = auth.mint_session();

        // P5: rotating clears every live session immediately.
        auth.rotate().unwrap();

        let app = test_router(TestState { auth });
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/guarded")
                    .header(header::COOKIE, format!("mdview_terminal_session={sid}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        fs::remove_dir_all(&dir).ok();
    }
}
