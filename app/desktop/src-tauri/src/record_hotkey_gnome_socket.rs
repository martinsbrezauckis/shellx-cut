//! Private Unix callback socket for the GNOME F9 forwarder.

use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

const SOCKET_DIRECTORY: &str = "shellx-cut-record-hotkey-v1";
const SOCKET_NAME: &str = "record-toggle.sock";
const LOCK_NAME: &str = "record-toggle.lock";
const FORWARDER_BYTE: u8 = 0xf9;

#[derive(Clone, Copy)]
struct SocketIdentity {
    dev: u64,
    ino: u64,
}

/// An advisory lock survives a crash as a harmless private file, but the
/// kernel releases the exclusive ownership. That lets a later Cut process
/// prove it is the sole owner before it considers removing a stale socket.
struct SocketLock {
    _file: File,
}

pub(crate) struct Service {
    socket_path: PathBuf,
    socket_identity: SocketIdentity,
    _lock: SocketLock,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Service {
    pub(crate) fn start<F>(callback: F) -> Result<Self, String>
    where
        F: Fn() + Send + 'static,
    {
        Self::start_in(runtime_root()?, callback, |listener, stop, callback| {
            thread::Builder::new()
                .name("shellx-cut-record-hotkey".to_string())
                .spawn(move || serve(listener, stop, callback))
        })
    }

    fn start_in<F, S>(root: PathBuf, callback: F, spawn: S) -> Result<Self, String>
    where
        F: Fn() + Send + 'static,
        S: FnOnce(UnixListener, Arc<AtomicBool>, F) -> std::io::Result<JoinHandle<()>>,
    {
        let (listener, socket_path, socket_identity, lock) = bind_owned_socket_in(&root)?;
        let stop = Arc::new(AtomicBool::new(false));
        let worker = match spawn(listener, stop.clone(), callback) {
            Ok(worker) => worker,
            Err(error) => {
                remove_owned_socket(&socket_path, socket_identity);
                return Err(format!("could not start the Global F9 callback: {error}"));
            }
        };
        Ok(Self {
            socket_path,
            socket_identity,
            _lock: lock,
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = UnixStream::connect(&self.socket_path);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        remove_owned_socket(&self.socket_path, self.socket_identity);
    }
}

pub(crate) fn forward_fixed_event() -> Result<(), String> {
    let path = forwarding_socket_path(runtime_root()?)?;
    let mut stream = UnixStream::connect(&path)
        .map_err(|error| format!("Global F9 callback is unavailable: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_millis(250)))
        .map_err(|error| format!("could not bound Global F9 callback: {error}"))?;
    stream
        .write_all(&[FORWARDER_BYTE])
        .map_err(|error| format!("could not forward Global F9: {error}"))
}

fn serve<F>(listener: UnixListener, stop: Arc<AtomicBool>, callback: F)
where
    F: Fn(),
{
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
                let mut byte = [0_u8; 1];
                let valid = stream.read_exact(&mut byte).is_ok()
                    && byte[0] == FORWARDER_BYTE
                    && stream.read(&mut [0_u8; 1]).unwrap_or(1) == 0;
                if valid && !stop.load(Ordering::Acquire) {
                    callback();
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) if stop.load(Ordering::Acquire) => break,
            Err(error) => {
                eprintln!("[shellx-cut] Global F9 callback stopped: {error}");
                break;
            }
        }
    }
}

fn runtime_root() -> Result<PathBuf, String> {
    let configured = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "XDG_RUNTIME_DIR is required for GNOME global F9.".to_string())?;
    let link = fs::symlink_metadata(&configured)
        .map_err(|error| format!("could not inspect XDG_RUNTIME_DIR: {error}"))?;
    if link.file_type().is_symlink() {
        return Err("XDG_RUNTIME_DIR must be a physical directory, not a symlink.".to_string());
    }
    let path = configured
        .canonicalize()
        .map_err(|error| format!("could not resolve physical XDG_RUNTIME_DIR: {error}"))?;
    validate_private_directory(&path, "XDG_RUNTIME_DIR")?;
    Ok(path)
}

fn validate_private_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {label}: {error}"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(format!(
            "{label} is not a private directory owned by this user."
        ));
    }
    Ok(())
}

fn private_directory(root: &Path) -> Result<PathBuf, String> {
    let directory = root.join(SOCKET_DIRECTORY);
    match fs::create_dir(&directory) {
        Ok(()) => {
            if let Err(error) = fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)) {
                let _ = fs::remove_dir(&directory);
                return Err(format!(
                    "could not protect Global F9 runtime directory: {error}"
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(format!(
                "could not create Global F9 runtime directory: {error}"
            ))
        }
    }
    validate_private_directory(&directory, "Global F9 runtime directory")?;
    Ok(directory)
}

fn validate_private_socket(path: &Path) -> Result<SocketIdentity, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect Global F9 callback socket: {error}"))?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err("Global F9 callback socket is not private to this user.".to_string());
    }
    Ok(SocketIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    })
}

fn acquire_lock(directory: &Path) -> Result<SocketLock, String> {
    let lock_path = directory.join(LOCK_NAME);
    match fs::symlink_metadata(&lock_path) {
        Ok(metadata)
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0 =>
        {
            return Err("Global F9 ownership lock is not private to this user.".to_string())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "could not inspect Global F9 ownership lock: {error}"
            ))
        }
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("could not open Global F9 ownership lock: {error}"))?;
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("could not protect Global F9 ownership lock: {error}"))?;
    let metadata = fs::symlink_metadata(&lock_path)
        .map_err(|error| format!("could not inspect Global F9 ownership lock: {error}"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err("Global F9 ownership lock is not private to this user.".to_string());
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(format!(
            "another ShellX Cut instance owns Global F9: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(SocketLock { _file: file })
}

fn recover_stale_socket(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "could not inspect existing Global F9 callback socket: {error}"
            ))
        }
    };
    let identity = validate_private_socket(path)?;
    match UnixStream::connect(path) {
        Ok(_) => Err("another ShellX Cut instance owns Global F9.".to_string()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            remove_owned_socket(path, identity);
            if fs::symlink_metadata(path).is_ok() {
                Err("could not remove the stale private Global F9 socket.".to_string())
            } else {
                Ok(())
            }
        }
        Err(error) => Err(format!(
            "could not verify existing Global F9 callback ownership: {error}"
        )),
    }
}

fn bind_owned_socket_in(
    root: &Path,
) -> Result<(UnixListener, PathBuf, SocketIdentity, SocketLock), String> {
    validate_private_directory(root, "XDG_RUNTIME_DIR")?;
    let directory = private_directory(root)?;
    let lock = acquire_lock(&directory)?;
    let socket_path = directory.join(SOCKET_NAME);
    recover_stale_socket(&socket_path)?;
    let listener = UnixListener::bind(&socket_path)
        .map_err(|error| format!("could not bind Global F9 callback socket: {error}"))?;
    if let Err(error) = fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600)) {
        // This process just created the endpoint below its flock-protected,
        // private directory, so no untrusted endpoint can be removed here.
        let _ = fs::remove_file(&socket_path);
        return Err(format!(
            "could not protect Global F9 callback socket: {error}"
        ));
    }
    if let Err(error) = listener.set_nonblocking(true) {
        let _ = fs::remove_file(&socket_path);
        return Err(format!(
            "could not make Global F9 callback bounded: {error}"
        ));
    }
    let socket_identity = match validate_private_socket(&socket_path) {
        Ok(identity) => identity,
        Err(error) => {
            let _ = fs::remove_file(&socket_path);
            return Err(error);
        }
    };
    Ok((listener, socket_path, socket_identity, lock))
}

fn forwarding_socket_path(root: PathBuf) -> Result<PathBuf, String> {
    validate_private_directory(&root, "XDG_RUNTIME_DIR")?;
    let directory = root.join(SOCKET_DIRECTORY);
    validate_private_directory(&directory, "Global F9 runtime directory")?;
    let path = directory.join(SOCKET_NAME);
    validate_private_socket(&path)?;
    Ok(path)
}

fn remove_owned_socket(path: &Path, expected: SocketIdentity) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_socket()
        && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.dev() == expected.dev
        && metadata.ino() == expected.ino
    {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    static TEST_ROOT_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    fn private_root() -> PathBuf {
        // Unix-domain socket names have a small, platform-defined bound. Keep
        // the test-only parent concise while retaining exclusive creation.
        for _ in 0..1024 {
            let sequence = TEST_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let root =
                std::env::temp_dir().join(format!("cut-f9-{}-{sequence}", std::process::id()));
            match fs::create_dir(&root) {
                Ok(()) => {
                    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
                    return root;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("could not create short private test root: {error}"),
            }
        }
        panic!("could not create an exclusive short private test root");
    }

    fn remove_root(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovers_a_private_stale_socket_but_never_a_live_one() {
        let root = private_root();
        let directory = private_directory(&root).unwrap();
        let socket_path = directory.join(SOCKET_NAME);
        assert!(
            socket_path.as_os_str().len() < 108,
            "test socket path must remain within the usual Unix-domain limit"
        );
        let stale = UnixListener::bind(&socket_path).unwrap();
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600)).unwrap();
        drop(stale);

        let (listener, current_path, identity, lock) = bind_owned_socket_in(&root).unwrap();
        assert_eq!(current_path, socket_path);
        assert!(listener.local_addr().is_ok());
        let error = match bind_owned_socket_in(&root) {
            Err(error) => error,
            Ok(_) => panic!("a live Global F9 listener must keep exclusive ownership"),
        };
        assert!(error.contains("another ShellX Cut instance owns Global F9"));
        drop(listener);
        remove_owned_socket(&socket_path, identity);
        drop(lock);
        remove_root(&root);
    }

    #[test]
    fn spawn_failure_removes_the_bound_socket_and_releases_the_lock() {
        let root = private_root();
        let socket_path = root.join(SOCKET_DIRECTORY).join(SOCKET_NAME);
        let error = match Service::start_in(
            root.clone(),
            || {},
            |_listener, _stop, _callback| Err(std::io::Error::other("synthetic thread failure")),
        ) {
            Err(error) => error,
            Ok(_) => panic!("the synthetic worker start must fail"),
        };
        assert!(error.contains("synthetic thread failure"));
        assert!(!socket_path.exists());
        let (listener, path, identity, lock) = bind_owned_socket_in(&root).unwrap();
        drop(listener);
        remove_owned_socket(&path, identity);
        drop(lock);
        remove_root(&root);
    }

    #[test]
    fn forwarder_requires_the_exact_private_endpoint_and_reaches_the_listener() {
        let root = private_root();
        let calls = Arc::new(AtomicUsize::new(0));
        let callback_calls = calls.clone();
        let service = Service::start_in(
            root.clone(),
            move || {
                callback_calls.fetch_add(1, Ordering::SeqCst);
            },
            |listener, stop, callback| {
                thread::Builder::new().spawn(move || serve(listener, stop, callback))
            },
        )
        .unwrap();
        let path = forwarding_socket_path(root.clone()).unwrap();
        assert_eq!(path, root.join(SOCKET_DIRECTORY).join(SOCKET_NAME));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(forwarding_socket_path(root.clone()).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let mut stream = UnixStream::connect(&path).unwrap();
        stream.write_all(&[FORWARDER_BYTE]).unwrap();
        for _ in 0..20 {
            if calls.load(Ordering::SeqCst) == 1 {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(service);
        remove_root(&root);
    }
}
