//! §24.1c: how a credential reaches git, and how it does not.
//!
//! **`credential.helper` takes exactly one explicit value and is never absent**, because absence
//! means inheritance: git would fall back to the user's configured helper, and a `repo`-scoped
//! token sitting in their OS credential manager would silently give a clone push rights the tier
//! design exists to withhold. The audit therefore **counts** the occurrences of
//! `-c credential.helper=` rather than searching for one — a test that only rejects a wrong value
//! passes on inheritance, which is the whole hazard.
//!
//! The four mechanisms §24.1c rejects, and why, so nobody reintroduces one:
//!
//! | Rejected | Why |
//! |---|---|
//! | URL userinfo, `https://<token>@host/…` | In argv, in git's own error output, and **persisted in the clone's `.git/config`** after the operation ends. The worst of the four |
//! | `-c http.extraHeader=Authorization: …` | In argv |
//! | `GIT_ASKPASS` / `SSH_ASKPASS` | Names a program, so the secret must still reach it — and setting either undoes §3.2's stripping for the whole child tree |
//! | Inheriting the user's helper | Borrows credentials this product was never granted |

use std::collections::hash_map::RandomState;
use std::ffi::OsString;
use std::fmt;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(windows)]
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::accounts::keychain::SecretToken;

/// The argv marker that makes the core run as Git's one-shot credential helper.
pub const CREDENTIAL_MODE_FLAG: &str = "--credential-mode";

const MAX_CREDENTIAL_PROTOCOL_BYTES: u64 = 64 * 1024;
const CLIENT_IO_TIMEOUT: Duration = Duration::from_secs(2);
const ACCEPT_POLL: Duration = Duration::from_millis(5);
const HELPER_USERNAME: &str = "token";
static NEXT_CHANNEL_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(windows)]
static CREDENTIAL_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The `-c credential.helper=` value for one invocation.
///
/// Renders **exactly one** `-c credential.helper=<value>` pair, always.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialChannel {
    /// No credential at all.
    ///
    /// §24.1c: *"A public clone carries no credential at all."* Anonymous HTTPS is the whole of
    /// the default tier's clone surface, so the `public`-tier token never reaches git. This is a
    /// complete behaviour rather than a placeholder — it is what the shipped default tier does.
    Anonymous,
    /// One authenticated invocation, backed by an already-listening local channel.
    ///
    /// [`OneShotCredential`] has no public constructor, so a helper value without its nonce file
    /// and listener cannot be constructed. The token is held only by the server thread; this
    /// variant carries paths and an address, never the token.
    Helper(OneShotCredential),
}

impl CredentialChannel {
    /// A clone that authenticates with nothing.
    #[must_use]
    pub const fn anonymous() -> Self {
        Self::Anonymous
    }

    /// Create the one-shot channel that supplies `token` only for `host`.
    ///
    /// The token is copied into the server thread's memory. It is never placed in argv, an
    /// environment variable, or a file. The nonce is a different value: it is stored in a private
    /// file, and only that file's path is rendered into the helper command.
    ///
    /// # Errors
    /// `InvalidInput` when the token is empty or holds a NUL, CR or LF, or the host is empty or
    /// holds whitespace or a control character, and also when a path the helper command is built
    /// from is not Unicode or this executable's path is not absolute. Otherwise any I/O failure
    /// locating this executable, creating the nonce file (and on Unix its private directory, with
    /// their modes read back), binding the listener or starting the server thread — and, on
    /// Windows outside `testkit`, a data directory [`configure_data_dir`] was never given.
    pub fn one_shot(token: &SecretToken, host: &str) -> io::Result<Self> {
        validate_protocol_value("token", token.expose())?;
        validate_host(host)?;
        let nonce = fresh_nonce();
        let helper_exe = std::env::current_exe()?;
        let helper = platform::start_channel(&helper_exe, &nonce, token.expose(), host)?;
        Ok(Self::Helper(helper))
    }

    /// The single `-c credential.helper=<value>` pair this channel renders.
    ///
    /// Returns the `-c` and its value as two elements, because argv is a list and a config option
    /// is two elements of it — joining them into one string is how a value with a space stops
    /// being one argument.
    #[must_use]
    pub fn helper_args(&self) -> Vec<OsString> {
        match self {
            Self::Anonymous => {
                vec![OsString::from("-c"), OsString::from("credential.helper=")]
            }
            Self::Helper(helper) => {
                let mut value = OsString::from("credential.helper=");
                value.push(&helper.0.helper_command);
                vec![OsString::from("-c"), value]
            }
        }
    }
}

/// The live half of [`CredentialChannel::Helper`].
///
/// Its fields are private deliberately: only [`CredentialChannel::one_shot`] can create one, so
/// the helper command, nonce file and listening server cannot drift into a dishonest combination.
#[doc(hidden)]
#[derive(Clone)]
pub struct OneShotCredential(Arc<ChannelState>);

impl fmt::Debug for OneShotCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OneShotCredential")
            .field("channel", &self.0.channel)
            .field("nonce_path", &self.0.nonce_path)
            .finish_non_exhaustive()
    }
}

impl PartialEq for OneShotCredential {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for OneShotCredential {}

struct ChannelState {
    helper_command: OsString,
    channel: String,
    nonce_path: PathBuf,
    #[cfg(all(unix, test))]
    private_dir: PathBuf,
    stop: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for ChannelState {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(slot) = self.worker.get_mut() {
            if let Some(worker) = slot.take() {
                let _ = worker.join();
            }
        }
    }
}

/// Record the core data directory used for Windows nonce files.
///
/// On Unix, the channel has its own mode-`0700` temporary directory and this is a no-op. On
/// Windows, std has no ACL API: the nonce file **inherits the data directory's ACL, which this
/// code does not set and cannot verify**. The normal binary calls this after parsing `--data-dir`.
///
/// # Errors
/// Never on Unix. On Windows, when the directory cannot be created, when a different directory
/// was already configured, or when a concurrent call configured one first.
// The Unix body is the `cfg(not(windows))` no-op, which is all clippy sees on Linux; the
// Windows body creates a directory and cannot be `const`.
#[allow(clippy::missing_const_for_fn)]
pub fn configure_data_dir(data_dir: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        std::fs::create_dir_all(data_dir)?;
        if let Some(existing) = CREDENTIAL_DATA_DIR.get() {
            if existing != data_dir {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "credential data directory was already configured",
                ));
            }
            return Ok(());
        }
        CREDENTIAL_DATA_DIR
            .set(data_dir.to_path_buf())
            .map_err(|_| io::Error::other("credential data directory raced initialization"))?;
    }
    #[cfg(not(windows))]
    let _ = data_dir;
    Ok(())
}

/// Answer one Git credential `get` over `out`.
///
/// `nonce` is the nonce read from the mode-`0600` file by the helper entry point; `channel` is the
/// public local address. The request itself is read from stdin using Git's credential protocol.
/// A nonce or host mismatch produces no output.
///
/// # Errors
/// `InvalidData` when the request or the channel's response exceeds 64 KiB; `InvalidInput` when
/// the requested host holds whitespace or a control character or `channel` is malformed; and any
/// I/O failure reading stdin, reaching the channel within its timeout, or writing `out`.
pub fn run_credential_helper(nonce: &str, channel: &str, out: &mut dyn Write) -> io::Result<()> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    run_credential_helper_with_input(nonce, channel, &mut input, out)
}

fn run_credential_helper_with_input(
    nonce: &str,
    channel: &str,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> io::Result<()> {
    let Some(host) = read_requested_host(input)? else {
        return Ok(());
    };
    let mut stream = platform::connect(channel)?;
    stream.set_io_timeout(CLIENT_IO_TIMEOUT)?;
    write!(stream, "nonce={nonce}\nhost={host}\n\n")?;
    stream.finish_request()?;

    let mut response = Vec::new();
    Read::take(&mut stream, MAX_CREDENTIAL_PROTOCOL_BYTES + 1).read_to_end(&mut response)?;
    if response.len() > usize::try_from(MAX_CREDENTIAL_PROTOCOL_BYTES).unwrap_or(usize::MAX) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "credential channel response exceeded its bound",
        ));
    }
    if !response.is_empty() {
        out.write_all(&response)?;
        out.flush()?;
    }
    Ok(())
}

fn read_requested_host(input: &mut dyn BufRead) -> io::Result<Option<String>> {
    let mut total = 0_u64;
    let mut host = None;
    loop {
        let mut line = String::new();
        let read = input.read_line(&mut line)?;
        if read == 0 || matches!(line.as_str(), "\n" | "\r\n") {
            break;
        }
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if total > MAX_CREDENTIAL_PROTOCOL_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "credential request exceeded its bound",
            ));
        }
        let value = line.trim_end_matches(['\r', '\n']);
        if let Some(found) = value.strip_prefix("host=") {
            validate_host(found)?;
            host = Some(found.to_owned());
        }
    }
    Ok(host)
}

fn validate_host(host: &str) -> io::Result<()> {
    if host.is_empty()
        || host.chars().any(char::is_whitespace)
        || host.chars().any(char::is_control)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "credential host is empty or contains protocol separators",
        ));
    }
    Ok(())
}

fn validate_protocol_value(name: &str, value: &str) -> io::Result<()> {
    if value.is_empty() || value.bytes().any(|byte| matches!(byte, 0 | b'\r' | b'\n')) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} is empty or contains credential protocol separators"),
        ));
    }
    Ok(())
}

fn fresh_nonce() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0_u128, |duration| duration.as_nanos());
    let id = NEXT_CHANNEL_ID.fetch_add(1, Ordering::Relaxed);
    let mut nonce = String::with_capacity(64);
    for lane in 0_u64..4 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u32(std::process::id());
        hasher.write_u128(now);
        hasher.write_u64(id);
        hasher.write_u64(lane);
        let _ = write!(nonce, "{:016x}", hasher.finish());
    }
    nonce
}

fn helper_command(helper_exe: &Path, nonce_path: &Path, channel: &str) -> io::Result<OsString> {
    let executable = absolute_helper_token(helper_exe)?;
    let nonce = path_text(nonce_path)?;
    Ok(OsString::from(format!(
        "{executable} {} {} {}",
        shell_quote(CREDENTIAL_MODE_FLAG),
        shell_quote(&nonce),
        shell_quote(channel)
    )))
}

fn path_text(path: &Path) -> io::Result<String> {
    path.to_str().map(ToOwned::to_owned).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "credential helper paths must be valid Unicode",
        )
    })
}

/// Keep the absolute-path prefix visible to Git's helper parser, then quote the remainder as one
/// shell word. Quoting the whole path would hide the prefix, making Git prepend
/// `git credential-`; leaving the remainder raw would split an ordinary installation path that
/// contains a space. Git documents that every resulting helper command is executed by its shell.
fn absolute_helper_token(path: &Path) -> io::Result<String> {
    let text = path_text(path)?;
    #[cfg(windows)]
    {
        let text = text.replace('\\', "/");
        let prefix_len = if text.starts_with("//") {
            2
        } else if text.as_bytes().get(1) == Some(&b':') && text.as_bytes().get(2) == Some(&b'/') {
            3
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "credential helper executable is not an absolute Windows path",
            ));
        };
        let (prefix, rest) = text.split_at(prefix_len);
        Ok(format!("{prefix}{}", shell_quote(rest)))
    }
    #[cfg(unix)]
    {
        let Some(rest) = text.strip_prefix('/') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "credential helper executable is not an absolute Unix path",
            ));
        };
        Ok(format!("/{}", shell_quote(rest)))
    }
}

fn shell_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

fn channel_id() -> String {
    format!(
        "cdt-cred-{:x}-{:x}",
        std::process::id(),
        NEXT_CHANNEL_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn serve_client<S: Read + Write>(
    stream: &mut S,
    expected_nonce: &str,
    expected_host: &str,
    token: &str,
) -> io::Result<()> {
    let mut request = Vec::new();
    Read::take(&mut *stream, MAX_CREDENTIAL_PROTOCOL_BYTES + 1).read_to_end(&mut request)?;
    if request.len() > usize::try_from(MAX_CREDENTIAL_PROTOCOL_BYTES).unwrap_or(usize::MAX) {
        return Ok(());
    }
    let Ok(text) = std::str::from_utf8(&request) else {
        return Ok(());
    };
    let mut nonce = None;
    let mut host = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("nonce=") {
            nonce = Some(value);
        } else if let Some(value) = line.strip_prefix("host=") {
            host = Some(value);
        }
    }
    let matches = nonce == Some(expected_nonce)
        && host.is_some_and(|value| value.eq_ignore_ascii_case(expected_host));
    if !matches {
        return Ok(());
    }
    stream.write_all(b"username=")?;
    stream.write_all(HELPER_USERNAME.as_bytes())?;
    stream.write_all(b"\npassword=")?;
    stream.write_all(token.as_bytes())?;
    stream.write_all(b"\n\n")?;
    stream.flush()
}

trait CredentialStream: Read + Write {
    fn set_io_timeout(&self, timeout: Duration) -> io::Result<()>;
    fn finish_request(&self) -> io::Result<()>;
}

mod platform {
    // Named rather than glob-imported. A wildcard here would re-export whatever the parent module
    // grows next into a module whose whole job is the platform-specific handling of a credential,
    // which is the one place a surprise import is least welcome.
    // **Split by target, because the two branches need different names.** An earlier revision
    // listed only what the Unix branch uses and compiled clean on Linux while failing to build on
    // Windows — `AtomicBool` and `CREDENTIAL_DATA_DIR` reached the Windows arm through the glob
    // this replaced. That is `CLAUDE.md`'s rule arriving inside an import list: **a check on one
    // target is not a check on the other**, and this module is the one place in the lane where
    // the two arms genuinely differ.
    use super::{
        channel_id, helper_command, io, serve_client, thread, Arc, ChannelState, CredentialStream,
        Duration, Mutex, OneShotCredential, Ordering, Path, PathBuf, Write as _, ACCEPT_POLL,
        CLIENT_IO_TIMEOUT,
    };
    use std::sync::atomic::AtomicBool;

    #[cfg(unix)]
    use super::path_text;
    #[cfg(unix)]
    use std::fs::{DirBuilder, OpenOptions};
    #[cfg(unix)]
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    #[cfg(unix)]
    use std::os::unix::net::{UnixListener, UnixStream};

    #[cfg(windows)]
    use super::CREDENTIAL_DATA_DIR;

    #[cfg(windows)]
    use std::fs::OpenOptions;
    #[cfg(windows)]
    use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};

    #[cfg(unix)]
    struct Cleanup {
        private_dir: PathBuf,
        nonce_path: PathBuf,
        socket_path: PathBuf,
    }

    #[cfg(unix)]
    impl Cleanup {
        fn unlink(&self) {
            let _ = std::fs::remove_file(&self.socket_path);
            let _ = std::fs::remove_file(&self.nonce_path);
            let _ = std::fs::remove_dir(&self.private_dir);
        }
    }

    #[cfg(unix)]
    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.unlink();
        }
    }

    #[cfg(windows)]
    struct Cleanup {
        nonce_path: PathBuf,
    }

    #[cfg(windows)]
    impl Cleanup {
        fn unlink(&self) {
            let _ = std::fs::remove_file(&self.nonce_path);
        }
    }

    #[cfg(windows)]
    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.unlink();
        }
    }

    #[cfg(unix)]
    pub(super) type Stream = UnixStream;
    #[cfg(windows)]
    pub(super) type Stream = TcpStream;

    #[cfg(unix)]
    pub(super) fn start_channel(
        helper_exe: &Path,
        nonce: &str,
        token: &str,
        host: &str,
    ) -> io::Result<OneShotCredential> {
        let private_dir = create_private_dir()?;
        let nonce_path = private_dir.join("nonce");
        let socket_path = private_dir.join("channel.sock");
        #[cfg(test)]
        let test_private_dir = private_dir.clone();
        let cleanup = Cleanup {
            private_dir,
            nonce_path: nonce_path.clone(),
            socket_path: socket_path.clone(),
        };
        write_private_nonce(&nonce_path, nonce)?;
        let listener = UnixListener::bind(&socket_path)?;
        listener.set_nonblocking(true)?;
        let channel = format!("unix:{}", path_text(&socket_path)?);
        let helper_command = helper_command(helper_exe, &nonce_path, &channel)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let expected_nonce = nonce.to_owned();
        let expected_host = host.to_owned();
        let secret = token.to_owned();
        let worker = thread::Builder::new()
            .name("credential-one-shot".to_owned())
            .spawn(move || {
                serve_unix(
                    listener,
                    cleanup,
                    &thread_stop,
                    &expected_nonce,
                    &expected_host,
                    &secret,
                );
            })?;
        Ok(OneShotCredential(Arc::new(ChannelState {
            helper_command,
            channel,
            nonce_path,
            #[cfg(test)]
            private_dir: test_private_dir,
            stop,
            worker: Mutex::new(Some(worker)),
        })))
    }

    #[cfg(unix)]
    fn create_private_dir() -> io::Result<PathBuf> {
        let root = std::env::temp_dir();
        for _ in 0..64 {
            let path = root.join(channel_id());
            let mut builder = DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
                    assert_mode(&path, 0o700)?;
                    return Ok(path);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique credential channel directory",
        ))
    }

    #[cfg(unix)]
    fn write_private_nonce(path: &Path, nonce: &str) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(nonce.as_bytes())?;
        file.flush()?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        assert_mode(path, 0o600)
    }

    #[cfg(unix)]
    fn assert_mode(path: &Path, expected: u32) -> io::Result<()> {
        let actual = std::fs::metadata(path)?.permissions().mode() & 0o777;
        if actual != expected {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "credential channel mode verification failed: expected {expected:04o}, got \
                     {actual:04o}"
                ),
            ));
        }
        Ok(())
    }

    /// `cleanup` is taken **by value on purpose**: its `Drop` unlinks the socket, the nonce file
    /// and the private directory, so owning it here ties that removal to the life of the serving
    /// thread rather than to a caller that may already have returned. It is also unlinked eagerly
    /// on the accepted path — the nonce has been spent by then and the file has no further use.
    #[cfg(unix)]
    // Owning the guard IS the consumption: its Drop performs the cleanup.
    #[allow(clippy::needless_pass_by_value)]
    fn serve_unix(
        listener: UnixListener,
        cleanup: Cleanup,
        stop: &AtomicBool,
        nonce: &str,
        host: &str,
        token: &str,
    ) {
        loop {
            if stop.load(Ordering::Acquire) {
                return;
            }
            match listener.accept() {
                Ok((mut stream, _address)) => {
                    drop(listener);
                    cleanup.unlink();
                    let _ = stream.set_read_timeout(Some(CLIENT_IO_TIMEOUT));
                    let _ = stream.set_write_timeout(Some(CLIENT_IO_TIMEOUT));
                    let _ = serve_client(&mut stream, nonce, host, token);
                    return;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(ACCEPT_POLL);
                }
                Err(_) => return,
            }
        }
    }

    #[cfg(unix)]
    pub(super) fn connect(channel: &str) -> io::Result<Stream> {
        let path = channel.strip_prefix("unix:").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid Unix credential channel",
            )
        })?;
        UnixStream::connect(path)
    }

    #[cfg(unix)]
    impl CredentialStream for UnixStream {
        fn set_io_timeout(&self, timeout: Duration) -> io::Result<()> {
            self.set_read_timeout(Some(timeout))?;
            self.set_write_timeout(Some(timeout))
        }

        fn finish_request(&self) -> io::Result<()> {
            self.shutdown(std::net::Shutdown::Write)
        }
    }

    #[cfg(windows)]
    pub(super) fn start_channel(
        helper_exe: &Path,
        nonce: &str,
        token: &str,
        host: &str,
    ) -> io::Result<OneShotCredential> {
        let data_dir = configured_data_dir()?;
        let nonce_path = create_nonce_file(&data_dir, nonce)?;
        let cleanup = Cleanup {
            nonce_path: nonce_path.clone(),
        };
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let channel = format!("tcp:{address}");
        let helper_command = helper_command(helper_exe, &nonce_path, &channel)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let expected_nonce = nonce.to_owned();
        let expected_host = host.to_owned();
        let secret = token.to_owned();
        let worker = thread::Builder::new()
            .name("credential-one-shot".to_owned())
            .spawn(move || {
                // The closure owns `cleanup`, so its `Drop` (the unlink) still runs as the
                // thread ends, right after `serve_tcp` returns.
                serve_tcp(
                    listener,
                    &cleanup,
                    &thread_stop,
                    &expected_nonce,
                    &expected_host,
                    &secret,
                );
            })?;
        Ok(OneShotCredential(Arc::new(ChannelState {
            helper_command,
            channel,
            nonce_path,
            stop,
            worker: Mutex::new(Some(worker)),
        })))
    }

    #[cfg(windows)]
    fn configured_data_dir() -> io::Result<PathBuf> {
        if let Some(path) = CREDENTIAL_DATA_DIR.get() {
            return Ok(path.clone());
        }
        #[cfg(feature = "testkit")]
        {
            let path = std::env::temp_dir()
                .join(format!("codotheca-credential-test-{}", std::process::id()));
            std::fs::create_dir_all(&path)?;
            Ok(path)
        }
        #[cfg(not(feature = "testkit"))]
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "credential data directory was not configured",
        ))
    }

    #[cfg(windows)]
    fn create_nonce_file(data_dir: &Path, nonce: &str) -> io::Result<PathBuf> {
        // The nonce file inherits the data directory's ACL, which this code does not set and
        // cannot verify. std exposes no Windows ACL API, so this is materially weaker than the
        // mode-checked Unix directory and file.
        for _ in 0..64 {
            let path = data_dir.join(format!("{}.nonce", channel_id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(nonce.as_bytes())?;
                    file.flush()?;
                    return Ok(path);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique credential nonce file",
        ))
    }

    #[cfg(windows)]
    fn serve_tcp(
        listener: TcpListener,
        cleanup: &Cleanup,
        stop: &AtomicBool,
        nonce: &str,
        host: &str,
        token: &str,
    ) {
        loop {
            if stop.load(Ordering::Acquire) {
                return;
            }
            match listener.accept() {
                Ok((mut stream, _address)) => {
                    drop(listener);
                    cleanup.unlink();
                    let _ = stream.set_read_timeout(Some(CLIENT_IO_TIMEOUT));
                    let _ = stream.set_write_timeout(Some(CLIENT_IO_TIMEOUT));
                    let _ = serve_client(&mut stream, nonce, host, token);
                    return;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(ACCEPT_POLL);
                }
                Err(_) => return,
            }
        }
    }

    #[cfg(windows)]
    pub(super) fn connect(channel: &str) -> io::Result<Stream> {
        let address = channel.strip_prefix("tcp:").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid Windows credential channel",
            )
        })?;
        TcpStream::connect(address)
    }

    #[cfg(windows)]
    impl CredentialStream for TcpStream {
        fn set_io_timeout(&self, timeout: Duration) -> io::Result<()> {
            self.set_read_timeout(Some(timeout))?;
            self.set_write_timeout(Some(timeout))
        }

        fn finish_request(&self) -> io::Result<()> {
            self.shutdown(Shutdown::Write)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::io::Cursor;

    use crate::accounts::keychain::SecretToken;

    use super::{
        run_credential_helper_with_input, shell_quote, CredentialChannel, OneShotCredential,
    };

    #[test]
    fn an_anonymous_channel_renders_one_empty_helper_and_not_zero() {
        let argv = CredentialChannel::anonymous().helper_args();
        let rendered: Vec<String> = argv
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            rendered,
            vec!["-c", "credential.helper="],
            "an empty value is not the same as no value: absence is inheritance"
        );
    }

    #[test]
    fn a_one_shot_channel_answers_once_for_its_nonce_and_host() {
        let token = SecretToken::new("credential-sentinel-do-not-leak".to_owned());
        let channel = CredentialChannel::one_shot(&token, "forge.example").unwrap();
        let CredentialChannel::Helper(OneShotCredential(state)) = &channel else {
            panic!("one_shot returned an anonymous channel");
        };
        let nonce = std::fs::read_to_string(&state.nonce_path).unwrap();
        let mut input = Cursor::new(b"protocol=https\nhost=forge.example\n\n");
        let mut out = Vec::new();
        run_credential_helper_with_input(&nonce, &state.channel, &mut input, &mut out).unwrap();
        let answer = String::from_utf8(out).unwrap();
        assert_eq!(
            answer,
            "username=token\npassword=credential-sentinel-do-not-leak\n\n"
        );
    }

    #[test]
    fn a_nonce_mismatch_is_answered_with_nothing() {
        let token = SecretToken::new("credential-sentinel-do-not-leak".to_owned());
        let channel = CredentialChannel::one_shot(&token, "forge.example").unwrap();
        let CredentialChannel::Helper(OneShotCredential(state)) = &channel else {
            panic!("one_shot returned an anonymous channel");
        };
        let mut input = Cursor::new(b"protocol=https\nhost=forge.example\n\n");
        let mut out = Vec::new();
        run_credential_helper_with_input("wrong-nonce", &state.channel, &mut input, &mut out)
            .unwrap();
        assert!(out.is_empty(), "a nonce mismatch disclosed a credential");
    }

    #[test]
    fn shell_quoting_keeps_one_argument_and_escapes_apostrophes() {
        assert_eq!(
            shell_quote("directory with space"),
            "'directory with space'"
        );
        assert_eq!(shell_quote("it's here"), "'it'\\''s here'");
    }

    #[cfg(unix)]
    #[test]
    fn unix_channel_modes_are_set_and_read_back() {
        use std::os::unix::fs::PermissionsExt as _;

        let token = SecretToken::new("credential-sentinel-do-not-leak".to_owned());
        let channel = CredentialChannel::one_shot(&token, "forge.example").unwrap();
        let CredentialChannel::Helper(OneShotCredential(state)) = &channel else {
            panic!("one_shot returned an anonymous channel");
        };
        assert_eq!(
            std::fs::metadata(&state.private_dir)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&state.nonce_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
