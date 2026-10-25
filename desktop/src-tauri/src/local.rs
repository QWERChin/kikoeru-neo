//! The local-library mode: the backend in this process and its admin
//! account in the system keychain.

use std::path::PathBuf;

use anyhow::Result;
use neo_server::embed::{self, Options, Server, WebSource};

/// The account the shell signs the local pages in with.
pub const ADMIN: &str = "local";
const KEYRING_SERVICE: &str = "kikoeru-neo";

/// Starts the backend, makes sure the local admin exists, and returns a
/// fresh session token for it.
pub async fn start(data_dir: PathBuf, web: WebSource) -> Result<(Server, String)> {
    let server = embed::start(Options {
        data_dir,
        ffmpeg: crate::tools::bundled("ffmpeg"),
        ffprobe: crate::tools::bundled("ffprobe"),
        web: Some(web),
    })
    .await?;
    match sign_in(&server).await {
        Ok(token) => Ok((server, token)),
        Err(e) => {
            // Dropping a server does not stop it, and a retry would start a
            // second one on the same database.
            if let Err(stop) = server.stop().await {
                tracing::warn!(error = %stop, "the local backend did not stop cleanly");
            }
            Err(e)
        }
    }
}

async fn sign_in(server: &Server) -> Result<String> {
    let password = tokio::task::spawn_blocking(keychain_password).await??;
    server.ensure_admin(ADMIN, &password).await?;
    server.session_token(ADMIN).await
}

/// The local admin's password from the keychain, created on first use.
/// Signing in never needs it, because the shell issues session tokens
/// directly, so a keychain that cannot be reached only costs the password.
fn keychain_password() -> Result<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, ADMIN);
    match entry.as_ref().map(|e| e.get_password()) {
        Ok(Ok(password)) => return Ok(password),
        Ok(Err(keyring::Error::NoEntry)) => {}
        Ok(Err(e)) => tracing::warn!(error = %e, "keychain unavailable"),
        Err(e) => tracing::warn!(error = %e, "keychain unavailable"),
    }
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no system randomness: {e}"))?;
    let password: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if let Ok(entry) = entry
        && let Err(e) = entry.set_password(&password)
    {
        tracing::warn!(error = %e, "cannot store the local password in the keychain");
    }
    Ok(password)
}

/// Run in each local page before its own scripts: stores the session token
/// where the frontend reads it, but only on the local backend's origin, so
/// a page the window navigates to elsewhere never sees it. A token the
/// shell renewed later (see [`renew_script`]) expires later than this one
/// and is kept, so reloading the page never brings back an older token.
pub fn init_script(port: u16, token: &str) -> String {
    let origin = serde_json::to_string(&format!("http://127.0.0.1:{port}")).expect("a string");
    let token = serde_json::to_string(token).expect("a string");
    format!(
        "(function () {{ if (location.origin !== {origin}) return; \
         var key = 'kikoeru-neo.token', token = {token}; \
         function exp(t) {{ try {{ return JSON.parse(atob(t.split('.')[1] \
         .replace(/-/g, '+').replace(/_/g, '/'))).exp || 0; }} catch (e) {{ return 0; }} }} \
         try {{ var stored = localStorage.getItem(key); \
         if (!stored || exp(stored) < exp(token)) localStorage.setItem(key, token); }} \
         catch (e) {{}} }})();"
    )
}

/// Run in the current local page when the shell renews the session. The
/// frontend picks the token up on its next load, or when it rereads the
/// stored token after a 401 (book/src/desktop.md).
pub fn renew_script(token: &str) -> String {
    let token = serde_json::to_string(token).expect("a string");
    format!("try {{ localStorage.setItem('kikoeru-neo.token', {token}); }} catch (e) {{}}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_written_only_on_the_local_origin() {
        let script = init_script(4321, "a\"b'</script>");
        assert!(script.contains(r#"location.origin !== "http://127.0.0.1:4321""#));
        assert!(script.contains(r#"token = "a\"b'</script>""#), "{script}");
        assert!(renew_script("x\"y").contains(r#"'kikoeru-neo.token', "x\"y""#));
    }
}
