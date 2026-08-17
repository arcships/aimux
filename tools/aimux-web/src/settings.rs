//! Console credential store (RFC-0029 §5.5 "Settings 与凭据").
//!
//! `KeyStore` holds one plaintext API key per provider for the lifetime of
//! the process (memory-first, so a key saved without "remember" never touches
//! the disk). When the user opts in with `remember`, the remembered subset is
//! written to a JSON file under the config dir with `0600` permissions on
//! unix (best effort on Windows) and reloaded on the next start.
//!
//! Threat model / invariants:
//! - plaintext keys never appear in API responses (`hints()` only leaks the
//!   last 4 characters), never in logs, and never in recordings (the
//!   recording layer already redacts key material);
//! - mutating the store (`set` / `remove`) is gated to loopback-bound servers
//!   by the API layer (`api::settings`), not here — the store itself is
//!   transport-agnostic.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Stored-key listing entry: provider + masked hint (never the key itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyHint {
    pub provider: String,
    /// Masked hint: last 4 characters, or the length when shorter.
    pub hint: String,
    /// Whether the key is persisted on disk (vs memory-only for this run).
    pub remembered: bool,
}

#[derive(Default)]
struct Inner {
    /// Every key known this run: memory-only entries plus remembered ones.
    keys: HashMap<String, String>,
    /// Providers whose keys are persisted to disk.
    remembered: HashSet<String>,
}

/// Per-provider API key store with optional disk persistence.
pub struct KeyStore {
    inner: Mutex<Inner>,
    /// Persistence target; `None` = memory-only (tests, no usable config dir).
    path: Option<PathBuf>,
}

impl KeyStore {
    /// Store backed by the default config path (loaded when the file exists).
    pub fn load_default() -> Self {
        Self::from_path(default_keys_path())
    }

    /// Store backed by `path` (loaded when the file exists; best effort —
    /// an unreadable/corrupt file logs a warning and starts empty).
    pub fn from_path(path: Option<PathBuf>) -> Self {
        let store = Self {
            inner: Mutex::new(Inner::default()),
            path,
        };
        if let Some(err) = store.load() {
            let shown = store
                .path
                .as_deref()
                .map_or_else(|| "<memory>".to_string(), |p| p.display().to_string());
            eprintln!(
                "aimux-web: warning: failed to load saved keys from {shown} ({err}) — starting empty"
            );
        }
        store
    }

    /// The plaintext key for `provider`, if saved this run or loaded at start.
    pub fn get(&self, provider: &str) -> Option<String> {
        self.inner.lock().unwrap().keys.get(provider).cloned()
    }

    /// Number of providers with a key in memory.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().keys.len()
    }

    /// Whether no key is stored (companion of [`len`](Self::len)).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Save `key` for `provider`.
    ///
    /// Always updates memory; additionally persists to disk when `remember`
    /// is set (a previously remembered key for the same provider is dropped
    /// from disk when `remember` is false, so restarts cannot resurrect it).
    ///
    /// # Errors
    ///
    /// Returns the disk write error when persistence was requested and
    /// failed — the memory entry is still updated.
    pub fn set(&self, provider: &str, key: &str, remember: bool) -> std::io::Result<bool> {
        let disk_dirty;
        {
            let mut inner = self.inner.lock().unwrap();
            inner.keys.insert(provider.to_string(), key.to_string());
            let was_remembered = inner.remembered.contains(provider);
            if remember {
                inner.remembered.insert(provider.to_string());
            } else {
                inner.remembered.remove(provider);
            }
            disk_dirty = was_remembered || remember;
        }
        if disk_dirty {
            self.persist()?;
        }
        Ok(remember)
    }

    /// Remove the key for `provider` from memory and disk.
    ///
    /// # Errors
    ///
    /// Returns the disk write error when a remembered key was removed and
    /// the rewrite failed — the memory entry is still removed.
    pub fn remove(&self, provider: &str) -> std::io::Result<bool> {
        let disk_dirty;
        {
            let mut inner = self.inner.lock().unwrap();
            let existed = inner.keys.remove(provider).is_some();
            disk_dirty = inner.remembered.remove(provider);
            if !existed {
                return Ok(false);
            }
        }
        if disk_dirty {
            self.persist()?;
        }
        Ok(true)
    }

    /// Masked listing of every stored key (sorted by provider; never
    /// contains plaintext).
    pub fn hints(&self) -> Vec<KeyHint> {
        let inner = self.inner.lock().unwrap();
        let mut hints: Vec<KeyHint> = inner
            .keys
            .iter()
            .map(|(provider, key)| KeyHint {
                provider: provider.clone(),
                hint: mask_key(key),
                remembered: inner.remembered.contains(provider),
            })
            .collect();
        hints.sort_by(|a, b| a.provider.cmp(&b.provider));
        hints
    }

    /// Load the persisted file into memory. `Some(err)` on a read/parse
    /// failure worth warning about.
    fn load(&self) -> Option<std::io::Error> {
        let path = self.path.as_ref()?;
        let raw = match fs::read(path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => return Some(e),
        };
        let file: KeysFile = match serde_json::from_slice(&raw) {
            Ok(f) => f,
            Err(e) => return Some(std::io::Error::other(e)),
        };
        let mut inner = self.inner.lock().unwrap();
        inner.remembered = file.keys.keys().cloned().collect();
        inner.keys = file.keys.into_iter().collect();
        None
    }

    /// Rewrite the disk file with the current remembered subset.
    fn persist(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let snapshot: BTreeMap<String, String> = {
            let inner = self.inner.lock().unwrap();
            inner
                .remembered
                .iter()
                .filter_map(|p| inner.keys.get(p).map(|k| (p.clone(), k.clone())))
                .collect()
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Atomic write via temp + rename: two concurrent PUT(remember) calls
        // snapshot-then-write outside the lock, so an in-place truncate could
        // interleave into torn JSON (silently wiping all keys on next load).
        static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = path.with_extension(format!("json.{seq}.tmp"));
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)?;
            set_private(&tmp)?;
            let json = serde_json::to_string_pretty(&KeysFile { keys: snapshot })
                .map_err(std::io::Error::other)?;
            std::io::Write::write_all(&mut file, json.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&tmp, path)
    }
}

/// Masked hint for a stored key: `…abcd` (last 4 chars) when longer than
/// 8 chars; shorter keys reveal only their length — never the full key.
fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    // Only reveal a tail when it cannot reconstruct a meaningful fraction of
    // the key — short keys (≤8 chars) get length only, so a 4-char key never
    // leaks in full via "…abcd".
    if chars.len() > 8 {
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("…{tail}")
    } else {
        format!("({} chars)", chars.len())
    }
}

/// On-disk format: `{"keys": {provider: plaintext}}`.
#[derive(serde::Serialize, serde::Deserialize)]
struct KeysFile {
    keys: BTreeMap<String, String>,
}

/// `0600` on unix; best effort (no-op) elsewhere.
fn set_private(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, perms)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Whether the console is bound to a loopback host (trusted local client).
///
/// Accepts the textual forms axum may see: `127.0.0.1`, `localhost`, `::1`
/// and the bracketed `[::1]`. Anything else (`0.0.0.0`, LAN IPs, hostnames)
/// is treated as non-loopback.
pub fn is_loopback_host(host: &str) -> bool {
    let normalized = host.trim().trim_start_matches('[').trim_end_matches(']');
    matches!(
        normalized.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

/// User home directory (`$HOME` on unix, `%USERPROFILE%` fallback on
/// Windows) — tiny helper instead of pulling in the `dirs`/`home` crates.
fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let dir = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    #[cfg(not(windows))]
    let dir = std::env::var_os("HOME");
    dir.filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// Config directory: `~/.config/aimux-web` (Linux/macOS) or
/// `%APPDATA%/aimux-web` (Windows). `None` when no home is resolvable.
fn config_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA")
            .filter(|s| !s.is_empty())
            .map(|d| PathBuf::from(d).join("aimux-web"))
    } else {
        home_dir().map(|h| h.join(".config").join("aimux-web"))
    }
}

/// Default persistence target: `keys.json` inside the config dir.
fn default_keys_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("keys.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (KeyStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::from_path(Some(dir.path().join("keys.json")));
        (store, dir)
    }

    #[test]
    fn mask_key_last_four_or_length() {
        assert_eq!(mask_key("sk-abcd1234"), "…1234");
        // Short keys reveal length only — a 4-char key must not leak in full
        // via its own tail (threshold: > 8 chars).
        assert_eq!(mask_key("abcdefgh"), "(8 chars)");
        assert_eq!(mask_key("abcd"), "(4 chars)");
        assert_eq!(mask_key("abc"), "(3 chars)");
        assert_eq!(mask_key(""), "(0 chars)");
    }

    #[test]
    fn loopback_detection() {
        for host in [
            "127.0.0.1",
            "localhost",
            "LOCALHOST",
            "::1",
            "[::1]",
            " 127.0.0.1 ",
        ] {
            assert!(is_loopback_host(host), "expected loopback: {host}");
        }
        for host in ["0.0.0.0", "192.168.1.5", "::", "example.com", "127.0.0.2"] {
            assert!(!is_loopback_host(host), "expected non-loopback: {host}");
        }
    }

    #[test]
    fn remember_round_trip_via_disk() {
        let (store, dir) = temp_store();
        store.set("openai", "sk-plain-1234", true).unwrap();

        let path = dir.path().join("keys.json");
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = path.metadata().unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "keys.json must be 0600");
        }

        // A fresh store on the same path picks the key up at "startup".
        let reloaded = KeyStore::from_path(Some(path));
        assert_eq!(reloaded.get("openai").as_deref(), Some("sk-plain-1234"));
        assert_eq!(
            reloaded.hints(),
            vec![KeyHint {
                provider: "openai".into(),
                hint: "…1234".into(),
                remembered: true,
            }]
        );
    }

    #[test]
    fn remember_false_stays_in_memory_and_drops_disk_entry() {
        let (store, dir) = temp_store();
        let path = dir.path().join("keys.json");
        store.set("openai", "sk-first", true).unwrap();
        assert!(path.exists());

        // Re-save without remember: memory updated, disk entry dropped.
        store.set("openai", "sk-second", false).unwrap();
        assert_eq!(store.get("openai").as_deref(), Some("sk-second"));
        let reloaded = KeyStore::from_path(Some(path));
        assert_eq!(reloaded.get("openai"), None, "old key must not resurrect");

        // A never-remembered provider never creates the file.
        let (fresh, dir2) = temp_store();
        fresh.set("deepseek", "sk-mem", false).unwrap();
        assert!(!dir2.path().join("keys.json").exists());
        assert_eq!(fresh.get("deepseek").as_deref(), Some("sk-mem"));
        assert_eq!(fresh.hints().len(), 1);
        assert!(!fresh.hints()[0].remembered);
    }

    #[test]
    fn remove_clears_memory_and_disk() {
        let (store, dir) = temp_store();
        let path = dir.path().join("keys.json");
        store.set("openai", "sk-a", true).unwrap();
        store.set("anthropic", "sk-b", false).unwrap();

        assert!(store.remove("openai").unwrap());
        assert_eq!(store.get("openai"), None);
        assert_eq!(
            KeyStore::from_path(Some(path)).get("openai"),
            None,
            "disk copy must be gone"
        );

        // Memory-only removal + removing an absent provider.
        assert!(store.remove("anthropic").unwrap());
        assert!(!store.remove("anthropic").unwrap());
        assert!(store.is_empty());
    }

    #[test]
    fn startup_loads_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.json");
        std::fs::write(
            &path,
            r#"{"keys": {"openai": "sk-old-9999", "zhipu": "z-new"}}"#,
        )
        .unwrap();

        let store = KeyStore::from_path(Some(path));
        assert_eq!(store.get("openai").as_deref(), Some("sk-old-9999"));
        assert_eq!(store.get("zhipu").as_deref(), Some("z-new"));
        // Sorted by provider; hints never contain the plaintext.
        let hints = store.hints();
        assert_eq!(hints.len(), 2);
        assert_eq!(hints[0].provider, "openai");
        assert_eq!(hints[0].hint, "…9999");
        assert_eq!(hints[1].provider, "zhipu");
        assert_eq!(hints[1].hint, "(5 chars)");
    }

    #[test]
    fn corrupt_file_starts_empty_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.json");
        std::fs::write(&path, "not json at all").unwrap();
        let store = KeyStore::from_path(Some(path));
        assert!(store.is_empty());
    }

    #[test]
    fn missing_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::from_path(Some(dir.path().join("keys.json")));
        assert!(store.is_empty());
    }
}
