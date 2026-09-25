//! An in-process worker whose transport still runs the shipped `serve` loop.
//!
//! Loopback TCP is available on both target hosts and on the Rust 1.80 minimum version, unlike
//! the newer standard-library pipe API or a Unix-only paired socket.

use crate::wsl::conn::{WorkerIo, WorkerLauncher, WslError};
use crate::wsl::serve::{serve, WorkerContext};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

type MakeContext = dyn Fn(&str) -> WorkerContext + Send + Sync;

pub struct LoopbackLauncher {
    make: Arc<MakeContext>,
    launched: Mutex<Vec<String>>,
}

impl std::fmt::Debug for LoopbackLauncher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopbackLauncher").finish_non_exhaustive()
    }
}

impl LoopbackLauncher {
    #[must_use]
    pub fn new(make: impl Fn(&str) -> WorkerContext + Send + Sync + 'static) -> Self {
        Self {
            make: Arc::new(make),
            launched: Mutex::new(Vec::new()),
        }
    }

    /// Every distro a worker was started for, in order.
    #[must_use]
    pub fn launched(&self) -> Vec<String> {
        match self.launched.lock() {
            Ok(launched) => launched.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl WorkerLauncher for LoopbackLauncher {
    fn launch(&self, distro: &str) -> Result<WorkerIo, WslError> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|err| launch_error(distro, err.to_string()))?;
        let address = listener
            .local_addr()
            .map_err(|err| launch_error(distro, err.to_string()))?;
        let client =
            TcpStream::connect(address).map_err(|err| launch_error(distro, err.to_string()))?;
        let writer = client
            .try_clone()
            .map_err(|err| launch_error(distro, err.to_string()))?;
        let stop_socket = client
            .try_clone()
            .map_err(|err| launch_error(distro, err.to_string()))?;

        let make = Arc::clone(&self.make);
        let owned_distro = distro.to_owned();
        let server = std::thread::Builder::new()
            .name("wsl-loopback-worker".to_owned())
            .spawn(move || {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let Ok(mut output) = stream.try_clone() else {
                    return;
                };
                let mut input = stream;
                let context = make(&owned_distro);
                let _ = serve(&context, &mut input, &mut output, std::process::id());
                let _ = output.shutdown(Shutdown::Both);
            })
            .map_err(|err| launch_error(distro, err.to_string()))?;
        let server = Arc::new(Mutex::new(Some(server)));

        match self.launched.lock() {
            Ok(mut launched) => launched.push(distro.to_owned()),
            Err(poisoned) => poisoned.into_inner().push(distro.to_owned()),
        }

        let stop_server = Arc::clone(&server);
        Ok(WorkerIo {
            reader: Box::new(client),
            writer: Box::new(writer),
            stop: Box::new(move || {
                let _ = stop_socket.shutdown(Shutdown::Both);
                let server = match stop_server.lock() {
                    Ok(mut server) => server.take(),
                    Err(poisoned) => poisoned.into_inner().take(),
                };
                if let Some(server) = server {
                    let _ = server.join();
                }
            }),
        })
    }
}

fn launch_error(distro: &str, detail: String) -> WslError {
    WslError::Launch {
        distro: distro.to_owned(),
        detail,
    }
}
