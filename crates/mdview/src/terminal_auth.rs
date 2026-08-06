//! Terminal authentication (D4, P1, P2, P5) — the mechanism a later cell
//! will use to gate the terminal routes. This module ships the mechanism
//! and its unit tests only; no route is mounted here.
//!
//! Modeled on herdr-go's `src/web/auth.rs`: a token compared in constant
//! time, a random in-memory session-id set behind an `HttpOnly`
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
//! Per P5 rotating the token cuts every live session immediately — a
//! session cookie minted under the previous token stops working the instant
//! the new one is written. That guarantee is now structural rather than a
//! sequencing convention: [`TerminalAuth::verify_and_mint`] and
//! [`TerminalAuth::rotate`] share a single lock spanning the token
//! read/compare/insert and the token write/session-clear respectively, so
//! the two can never interleave (see "Rotation race" below). There is no
//! public `verify(token) -> bool` to compose with a separate
//! `mint_session()` call any more — `verify_and_mint` is the only way to
//! turn a raw token into a session, and it is atomic.
//!
//! # Rotation race (closed)
//!
//! The previous shape of this module had `rotate()` write the new token and
//! clear the session set as two unsynchronized steps, with `verify()` then
//! `mint_session()` as the only public login shape. A request holding a
//! leaked token could call `verify()` successfully, have the token rotate
//! in a concurrent request, and then call `mint_session()` into the
//! freshly-cleared set — the revoked token bought a live session that
//! survived until the next rotation. `verify_and_mint` and `rotate` now
//! share `TerminalAuth`'s single `Mutex`, so the two operations are
//! strictly ordered: whichever runs first completes in full (including the
//! session-set mutation) before the other starts. A session minted from a
//! token verified just before a rotation is cleared by that same rotation;
//! a token presented just after a rotation is compared against the new
//! value and rejected. See `rotation_race_cannot_mint_a_live_session_after_rotation`.
//!
//! # Method-mismatch route oracle (closed)
//!
//! [`AuthSession`] never runs on a method mismatch, because axum answers a
//! `MethodRouter` method mismatch itself, before any handler extractor
//! executes — and it decorates that response with an `Allow` header derived
//! from whichever methods were registered (`.get()`, `.post()`, `.on()`),
//! **even through a custom `.fallback(...)` handler**. So an unauthenticated
//! `GET` on a `POST`-only gated route would answer `405` with
//! `Allow: POST`, while a path that was never routed at all answers a plain
//! `404` with no `Allow` header — the whole terminal route surface would be
//! enumerable without a token. [`MethodGate`] closes this: cells mounting
//! terminal routes must use `axum::routing::any(handler)` with
//! `MethodGate<Get>` / `MethodGate<Post>` as the handler's first extractor,
//! never `.get(handler)` / `.post(handler)` / `MethodRouter::fallback`. See
//! [`MethodGate`]'s docs for why the alternative can't be fixed by choosing
//! a different fallback handler, and
//! `method_mismatch_is_byte_identical_to_an_unrouted_path` for the proof.
//!
//! The "wrong token yields an opaque 404" half of agent-terminal-3's first
//! truth is satisfied here, through `verify_and_mint`'s caller contract —
//! `None` on a bad or missing token, `Some(session_id)` on a good one,
//! never a status code of its own. `login_terminal` in `server.rs`
//! (agent-terminal-8) is that route: its whole job is to turn `None` into
//! [`opaque_404`] and `Some` into a session cookie — and, since it is now
//! the only caller of `verify_and_mint` anywhere in the product, also the
//! only place a session is ever minted from a presented credential.
//!
//! # Permission race and failed-write truncation (closed)
//!
//! The previous `write_token_file` applied `0600` only on first creation;
//! rotating over an existing file wrote the new secret under whatever mode
//! that file already had, then `chmod`ed after — a window where a
//! previously-widened file held the new secret under the wider mode. It
//! also risked leaving a truncated token on disk (with every old session
//! still live) if the write failed partway. `write_token_file` now writes
//! to a fresh, `0600`-permissioned (`O_EXCL`-created) temp file and
//! `rename`s it over the target — the target path is therefore always
//! either the previous complete, correctly-permissioned file or the new
//! one, never a partial or wrongly-permissioned write, and a failed write
//! never touches the target at all (see [`rotate`](TerminalAuth::rotate)'s
//! call ordering: the session set is only cleared *after* the write
//! succeeds).
//!
//! # Windows silence (closed)
//!
//! [`token_file_permissions_enforced`] is a queryable fact — `false` on any
//! platform without a mode-bit equivalent (Windows today) — rather than the
//! previous silent best-effort write. A settings surface can render it.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::FromRequestParts;
use axum::http::{header, request::Parts, HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq;

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

/// Whether the current platform enforces owner-only permissions on the
/// token file. `true` on unix (`0600`, applied atomically by
/// `write_token_file`). `false` on any platform without a mode-bit
/// equivalent — today, Windows — where the token file is written with the
/// platform default permissions and no protection is applied. A settings
/// surface should render this rather than implying the token is always
/// file-protected.
pub const fn token_file_permissions_enforced() -> bool {
    cfg!(unix)
}

/// The in-memory state a single [`Mutex`] guards: the live session set.
/// `rotate` and `verify_and_mint` both take this lock for their entire
/// critical section (including, for `rotate`, the on-disk write) so the two
/// can never interleave — see the module-level "Rotation race" section.
struct Inner {
    sessions: HashSet<String>,
}

/// The terminal auth mechanism: where the token lives on disk, plus the
/// in-memory set of sessions minted against it. Clones share the same
/// locked state (via `Arc`), the way `AppState` is shared across handlers.
#[derive(Clone)]
pub struct TerminalAuth {
    data_dir: Option<PathBuf>,
    inner: Arc<Mutex<Inner>>,
}

impl TerminalAuth {
    /// `data_dir` is the same injectable override the config/settings routes
    /// use (`AppState.config_data_dir`) — `None` resolves to the real
    /// `~/.mdview` in production, `Some(tmp)` keeps tests isolated.
    pub fn new(data_dir: Option<PathBuf>) -> Self {
        Self {
            data_dir,
            inner: Arc::new(Mutex::new(Inner {
                sessions: HashSet::new(),
            })),
        }
    }

    fn token_path(&self) -> PathBuf {
        token_path_override(self.data_dir.as_deref())
    }

    /// The token currently on disk, if one has ever been generated.
    /// Private — nothing outside this module ever gets the full token this
    /// way. Use [`masked`](Self::masked) for a display-safe read, or
    /// [`verify_and_mint`](Self::verify_and_mint) to check a presented
    /// value.
    fn load_token(&self) -> Option<String> {
        fs::read_to_string(self.token_path())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Generate a fresh token, write it to disk (owner-only permissions
    /// where the platform supports them, applied atomically — see the
    /// module-level "Permission race" section), and clear every live
    /// session (P5) so a cookie minted under the previous token stops
    /// working immediately. Returns the token in full — per P2, the only
    /// call in this module that ever does.
    ///
    /// Holds the same lock [`verify_and_mint`](Self::verify_and_mint) does,
    /// across both the write and the session-set clear, closing the
    /// rotation race described at the module level. If the write fails, the
    /// session set is left untouched and the previous token on disk is
    /// unmodified (the write itself never touches the target path until it
    /// has succeeded in full — see `write_token_file`).
    pub fn rotate(&self) -> io::Result<String> {
        let token = generate_token();
        let path = self.token_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut inner = self.inner.lock().unwrap();
        write_token_file(&path, &token)?;
        inner.sessions.clear();
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

    /// Constant-time check of `presented` against the token on disk, and —
    /// only on a match — mint and record a new session id in the same
    /// locked critical section. `None` on a missing or mismatched token; a
    /// missing token (never configured) always fails closed. This is the
    /// **only** public way to turn a raw token into a session: there is no
    /// standalone `verify()` to compose with a separate `mint_session()`
    /// call, which is exactly the composition that made the rotation race
    /// possible (see the module-level docs).
    pub fn verify_and_mint(&self, presented: &str) -> Option<String> {
        let mut inner = self.inner.lock().unwrap();
        let configured = self.load_token()?;
        if !constant_time_eq(presented.as_bytes(), configured.as_bytes()) {
            return None;
        }
        let id = new_session_id();
        inner.sessions.insert(id.clone());
        Some(id)
    }

    /// Mint and record a new session id without checking a token. Used only
    /// by this module's own tests, to set up an already-live session
    /// directly instead of round-tripping through `rotate` +
    /// `verify_and_mint` every time. Nothing outside `#[cfg(test)]` calls
    /// this — `verify_and_mint` (via `login_terminal` in `server.rs`) is the
    /// **only** production path that ever turns a request into a session
    /// (agent-terminal-8 closed the previous gap, where
    /// `rotate_terminal_token` called this directly with no credential
    /// check at all). Never expose this alongside a way to check a raw
    /// token from outside this module; that pairing is exactly the rotation
    /// race `verify_and_mint` exists to prevent.
    pub fn mint_session(&self) -> String {
        let id = new_session_id();
        self.inner.lock().unwrap().sessions.insert(id.clone());
        id
    }

    /// Whether `id` is a currently live session.
    pub fn session_valid(&self, id: &str) -> bool {
        self.inner.lock().unwrap().sessions.contains(id)
    }

    /// Drop one session (logout). Idempotent.
    pub fn end_session(&self, id: &str) {
        self.inner.lock().unwrap().sessions.remove(id);
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

/// Write `token` to `path` atomically: create a fresh temp file beside
/// `path` (`O_EXCL`/`create_new`, so it can never collide with or inherit
/// the permissions of anything already there) with owner-only permissions
/// where the platform supports them (unix `0600`), write the full token,
/// then `rename` it over `path`. The temp file's name carries a slice of
/// the token's own randomness so concurrent or repeated rotations never
/// collide on a stale leftover name.
///
/// Because the target path is only ever replaced by a single `rename`, it
/// is at every instant either the complete previous file (correct content,
/// correct mode) or the complete new one — never a partially written body,
/// and never new content under an old, looser mode (the "permission race"
/// closed at the module level). A failure at any point before the `rename`
/// (temp-file creation, write, or `sync_all`) leaves `path` completely
/// untouched, so [`TerminalAuth::rotate`] can safely skip clearing sessions
/// only on success.
///
/// Windows has no mode-bit equivalent; the file is written with the
/// platform default there. [`token_file_permissions_enforced`] reports that
/// state instead of leaving it silent.
fn write_token_file(path: &Path, token: &str) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "token path has no parent directory",
        )
    })?;
    let suffix = &token[..token.len().min(8)];
    let tmp_path = dir.join(format!("{TOKEN_FILE_NAME}.tmp-{suffix}"));

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp_path)?;
        f.write_all(token.as_bytes())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp_path, path)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        f.write_all(token.as_bytes())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

/// Constant-time byte comparison via the `subtle` crate — a hand-rolled XOR
/// loop has no compiler barrier stopping an optimizer from short-circuiting
/// it, which would reopen the timing leak this function exists to close.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
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
///
/// This extractor alone does not close the method-mismatch route oracle —
/// axum answers a mismatched method before any extractor runs at all. Every
/// gated route must also be mounted with [`MethodGate`] (see the
/// module-level docs).
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

/// The one HTTP method a [`MethodGate`] accepts, carried as a zero-sized
/// type parameter so `MethodGate<Get>` and `MethodGate<Post>` are distinct
/// extractor types the compiler tracks per route.
pub trait GateMethod {
    const METHOD: Method;
}

/// [`GateMethod`] for `GET`.
pub struct Get;
impl GateMethod for Get {
    const METHOD: Method = Method::GET;
}

/// [`GateMethod`] for `POST`.
pub struct Post;
impl GateMethod for Post {
    const METHOD: Method = Method::POST;
}

/// Extractor closing the method-mismatch route oracle (D4): rejects any
/// request whose method is not `M::METHOD` with the same opaque 404
/// [`AuthSession`] uses, before any other extractor or the handler body
/// runs.
///
/// # Why this instead of `.get(handler)` / `.post(handler)` / `MethodRouter::fallback`
///
/// axum's `MethodRouter` decorates a **mismatched-method** response with an
/// `Allow` header derived from whichever methods were registered
/// (`.get()`, `.post()`, `.on()`) — and it does this unconditionally, even
/// when a custom `.fallback(...)` handler is set to answer the mismatch
/// (axum's `set_allow_header` only skips the header for a route built with
/// *no* named method at all, i.e. `axum::routing::any`). That header alone
/// would let an unauthenticated probe distinguish a gated route (`405` +
/// `Allow: POST`) from a path that was never routed (`404`, no `Allow`)
/// without a token ever being checked — the exact oracle this cell closes.
/// The fix is therefore structural, not a choice of fallback handler: mount
/// a gated route with `axum::routing::any(handler)`, where `handler`'s
/// first extractor argument is `MethodGate<Get>` / `MethodGate<Post>`
/// rather than relying on axum's per-method routing at all. See
/// `method_mismatch_is_byte_identical_to_an_unrouted_path` for the proof.
pub struct MethodGate<M>(PhantomData<M>);

#[axum::async_trait]
impl<S, M> FromRequestParts<S> for MethodGate<M>
where
    S: Send + Sync,
    M: GateMethod + Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if parts.method == M::METHOD {
            Ok(MethodGate(PhantomData))
        } else {
            Err(opaque_404())
        }
    }
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
        assert!(auth.verify_and_mint("anything").is_none());
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
    fn verify_and_mint_accepts_the_configured_token_and_rejects_others() {
        let dir = scratch_dir("verify");
        let auth = TerminalAuth::new(Some(dir.clone()));
        let token = auth.rotate().unwrap();

        let sid = auth.verify_and_mint(&token).expect("configured token must verify");
        assert!(auth.session_valid(&sid));
        assert!(auth.verify_and_mint("wrong-token").is_none());
        assert!(auth.verify_and_mint("").is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rotating_replaces_the_token_so_the_old_value_no_longer_verifies() {
        let dir = scratch_dir("rotate-replaces");
        let auth = TerminalAuth::new(Some(dir.clone()));
        let first = auth.rotate().unwrap();
        let second = auth.rotate().unwrap();
        assert_ne!(first, second);
        assert!(auth.verify_and_mint(&first).is_none());
        assert!(auth.verify_and_mint(&second).is_some());
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

    /// Truth: "A login that verifies a token before a rotation cannot mint
    /// a session after that rotation." `verify_and_mint` and `rotate` share
    /// a single lock, so — regardless of how the OS schedules the two
    /// threads below — whichever call's *entire* critical section runs
    /// first completes before the other starts. Either the verify happens
    /// first and its session is wiped by the rotation that follows, or the
    /// rotation happens first and the verify is checked against (and fails
    /// against) the new token. There is no interleaving in which the old
    /// token's session both exists and outlives the rotation, which is what
    /// this test asserts on every run rather than only on a lucky
    /// interleaving.
    #[test]
    fn rotation_race_cannot_mint_a_live_session_after_rotation() {
        let dir = scratch_dir("rotation-race");
        let auth = TerminalAuth::new(Some(dir.clone()));
        let old_token = auth.rotate().unwrap();

        let auth_for_verify = auth.clone();
        let old_token_for_verify = old_token.clone();
        let verifier = std::thread::spawn(move || auth_for_verify.verify_and_mint(&old_token_for_verify));

        let new_token = auth.rotate().unwrap();
        assert_ne!(old_token, new_token);

        if let Some(sid) = verifier.join().unwrap() {
            assert!(
                !auth.session_valid(&sid),
                "a session minted from a token verified before rotation must not remain live after it"
            );
        }

        // The old token must never verify again; the new one must.
        assert!(auth.verify_and_mint(&old_token).is_none());
        let sid = auth.verify_and_mint(&new_token).unwrap();
        assert!(auth.session_valid(&sid));
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

    /// Truth: "Rotating over an existing token file never exposes the new
    /// token under a looser mode, at any instant." Simulates a token file
    /// left behind under a looser mode (e.g. by a pre-fix mdview version, or
    /// restored from a backup) and asserts the very next rotation leaves it
    /// `0600` — the temp-file-plus-rename write can never inherit an
    /// existing file's mode, unlike the old truncate-in-place write.
    #[cfg(unix)]
    #[test]
    fn rotating_over_a_looser_existing_file_never_inherits_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch_dir("perm-inherit");
        let auth = TerminalAuth::new(Some(dir.clone()));
        auth.rotate().unwrap();
        let path = token_path_override(Some(&dir));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        auth.rotate().unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "rotation must never leave the token under a looser mode");
        fs::remove_dir_all(&dir).ok();
    }

    /// Same truth, proven under concurrent write pressure instead of a
    /// single before/after snapshot: a background thread continuously
    /// stats the token file while the foreground thread rotates repeatedly,
    /// and every observation it makes must be `0600`. The temp-file-plus-
    /// rename write guarantees the target path is always either the
    /// complete previous file or the complete new one — never a wider-mode
    /// window — so this holds regardless of scheduling.
    #[cfg(unix)]
    #[test]
    fn concurrent_rotation_never_exposes_a_looser_mode() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::AtomicBool;

        let dir = scratch_dir("perm-concurrent");
        let auth = TerminalAuth::new(Some(dir.clone()));
        auth.rotate().unwrap();
        let path = token_path_override(Some(&dir));

        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_poller = stop.clone();
        let poll_path = path.clone();
        let poller = std::thread::spawn(move || {
            while !stop_for_poller.load(Ordering::Relaxed) {
                if let Ok(meta) = fs::metadata(&poll_path) {
                    let mode = meta.permissions().mode() & 0o777;
                    assert_eq!(
                        mode, 0o600,
                        "token file observed with a non-owner-only mode mid-rotation"
                    );
                }
            }
        });

        for _ in 0..50 {
            auth.rotate().unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        poller.join().unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    /// Truth: "A failed token write leaves the previous token and its
    /// sessions in a consistent state." Makes the data dir unwritable so
    /// the temp file for the next rotation cannot be created, and asserts
    /// the previous token still verifies and every session minted under it
    /// is still live afterward.
    #[cfg(unix)]
    #[test]
    fn a_failed_rotation_write_leaves_the_previous_token_and_sessions_untouched() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch_dir("write-fails");
        let auth = TerminalAuth::new(Some(dir.clone()));
        let first = auth.rotate().unwrap();
        let sid = auth.mint_session();
        assert!(auth.session_valid(&sid));

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        let result = auth.rotate();
        assert!(
            result.is_err(),
            "rotation should fail when its temp file cannot be created"
        );
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();

        assert!(
            auth.verify_and_mint(&first).is_some(),
            "the previous token must still verify after a failed rotation"
        );
        assert!(
            auth.session_valid(&sid),
            "sessions from before a failed rotation must remain live"
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// Truth: "On a platform without file modes the unprotected state is
    /// queryable, not silent."
    #[cfg(unix)]
    #[test]
    fn token_file_permissions_enforced_is_true_on_unix() {
        assert!(token_file_permissions_enforced());
    }

    #[cfg(not(unix))]
    #[test]
    fn token_file_permissions_enforced_is_false_off_unix() {
        assert!(!token_file_permissions_enforced());
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

    /// Truth: "A wrong-method request to a gated route is byte-identical to
    /// a request for a path that was never routed" — status, headers and
    /// body. Mounts a `MethodGate<Post>`-gated route via
    /// `axum::routing::any` (never `.post()`, which axum would decorate
    /// with an `Allow` header on a method mismatch even through a custom
    /// fallback — see [`MethodGate`]'s docs) and compares a `GET` against
    /// it to a `GET` against a path with no route at all.
    #[tokio::test]
    async fn method_mismatch_is_byte_identical_to_an_unrouted_path() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        async fn post_only(_gate: MethodGate<Post>) -> &'static str {
            "ok"
        }

        let app = axum::Router::new()
            .route("/gated", axum::routing::any(post_only))
            .with_state(());

        let mismatched = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/gated")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(mismatched.status(), StatusCode::NOT_FOUND);
        assert_eq!(mismatched.status(), unrouted.status());
        assert_eq!(
            mismatched.headers(),
            unrouted.headers(),
            "a method mismatch on a gated route must carry no header (e.g. Allow) an unrouted path wouldn't"
        );

        let mismatched_body = mismatched.into_body().collect().await.unwrap().to_bytes();
        let unrouted_body = unrouted.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(mismatched_body, unrouted_body);
    }

    /// The correct method still reaches the handler through the same
    /// `any()`-mounted route `MethodGate` requires.
    #[tokio::test]
    async fn method_gate_passes_the_matching_method_through() {
        use tower::ServiceExt;

        async fn post_only(_gate: MethodGate<Post>) -> &'static str {
            "ok"
        }

        let app = axum::Router::new()
            .route("/gated", axum::routing::any(post_only))
            .with_state(());

        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/gated")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
