// #![deny(unsafe_code, warnings, missing_docs)]

#[cfg(feature = "gitea")]
pub mod gitea;
#[cfg(feature = "k3s")]
pub mod k3s;

use thiserror::Error;

#[cfg(all(feature = "destructor", any(feature = "k3s", feature = "gitea")))]
use ctor::dtor;
#[cfg(feature = "gitea")]
use gitea::Gitea;
#[cfg(feature = "k3s")]
use k3s::K3s;
#[cfg(feature = "k3s")]
use kube::Client;
#[cfg(feature = "k3s")]
use rustls::crypto::{aws_lc_rs, CryptoProvider};
#[cfg(any(feature = "k3s", feature = "gitea"))]
use std::env;
#[cfg(all(feature = "destructor", any(feature = "k3s", feature = "gitea")))]
use std::thread;
#[cfg(any(feature = "k3s", feature = "gitea"))]
use testcontainers::ContainerAsync;
#[cfg(all(feature = "destructor", any(feature = "k3s", feature = "gitea")))]
use tokio::runtime;
#[cfg(any(feature = "k3s", feature = "gitea"))]
use tokio::sync;

/// Convenient alias for `Result`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

pub const DOCKER_NETWORK_NAME: &str = "testcontainers";

#[cfg(feature = "k3s")]
const USE_EXISTING_K8S_CONTEXT: &str = "CARGO_USE_EXISTING_K8S_CONTEXT";

#[cfg(feature = "gitea")]
static GIT_SERVER_CONTAINER: sync::OnceCell<sync::RwLock<Option<ContainerAsync<Gitea>>>> = sync::OnceCell::const_new();
#[cfg(feature = "k3s")]
static K3S_CLUSTER_CONTAINER: sync::OnceCell<sync::RwLock<Option<ContainerAsync<K3s>>>> = sync::OnceCell::const_new();

/// Represents crate-specific errors.
#[derive(Debug, Error)]
pub enum Error {
    #[cfg(any(feature = "k3s", feature = "gitea"))]
    /// Error during testcontainers operations.
    #[error("Testcontainers error: {0}")]
    Testcontainers(#[from] testcontainers::TestcontainersError),

    #[cfg(feature = "k3s")]
    /// Error during kube operations.
    #[error("Kube error: {0}")]
    Kube(#[from] kube::Error),

    #[cfg(feature = "k3s")]
    /// Error during kube operations.
    #[error("Kube error: {0}")]
    KubeConfig(#[from] kube::config::KubeconfigError),

    #[cfg(feature = "destructor")]
    /// Error during tokio operations.
    #[error("Tokio error: {0}")]
    Tokio(#[from] tokio::task::JoinError),

    /// Input/output error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Runtime configuration error
    #[error("Runtime configuration error: {0}")]
    RuntimeConfig(String),
}

#[cfg(feature = "gitea")]
pub async fn get_test_gitea_hostname() -> Result<String> {
    let host = start_git_server()
        .await
        .read()
        .await
        .as_ref()
        .unwrap()
        .get_host()
        .await?;
    Ok(host.to_string())
}

#[cfg(feature = "k3s")]
pub async fn get_test_kube_client() -> Result<Client> {
    if std::env::var(USE_EXISTING_K8S_CONTEXT).is_ok() {
        init_crypto_provider();
        let client = Client::try_default().await?;
        return Ok(client);
    }

    let guard = start_k3s_cluster().await.read().await;
    let cluster = guard.as_ref().unwrap();
    K3s::get_client(cluster).await
}

#[cfg(feature = "gitea")]
async fn start_git_server() -> &'static sync::RwLock<Option<ContainerAsync<Gitea>>> {
    GIT_SERVER_CONTAINER
        .get_or_init(|| async {
            let container = gitea::run_git_server().await.unwrap();
            sync::RwLock::new(Some(container))
        })
        .await
}

#[cfg(feature = "k3s")]
async fn start_k3s_cluster() -> &'static sync::RwLock<Option<ContainerAsync<K3s>>> {
    K3S_CLUSTER_CONTAINER
        .get_or_init(|| async {
            init_crypto_provider();
            // Create k3s container
            let container = k3s::run_k3s_cluster().await.unwrap();

            sync::RwLock::new(Some(container))
        })
        .await
}

#[cfg(any(feature = "k3s", feature = "gitea"))]
fn get_runtime_folder() -> Result<String> {
    env::var("OUT_DIR")
        .map_err(|_| Error::RuntimeConfig("`OUT_DIR` environment variable isn`t set, use Cargo to run build".into()))
}

#[cfg(feature = "k3s")]
fn init_crypto_provider() {
    if CryptoProvider::get_default().is_none() {
        aws_lc_rs::default_provider()
            .install_default()
            .expect("Error initializing rustls provider");
    }
}

#[cfg(all(feature = "destructor", any(feature = "k3s", feature = "gitea")))]
#[dtor]
fn shutdown_test_containers() {
    eprintln!("destructor");
    static LOCK: sync::Mutex<()> = sync::Mutex::const_new(());

    let _ = thread::spawn(|| {
        runtime::Runtime::new().unwrap().block_on(async {
            let _guard = LOCK.lock().await;

            #[cfg(feature = "k3s")]
            if let Some(k3s) = K3S_CLUSTER_CONTAINER.get() {
                let mut k3s = k3s.write().await;
                if k3s.is_some() {
                    let old = (*k3s).take().unwrap();
                    old.stop().await.unwrap();
                    old.rm().await.unwrap();
                    *k3s = None;
                }
            }

            #[cfg(feature = "gitea")]
            if let Some(git) = GIT_SERVER_CONTAINER.get() {
                let mut git = git.write().await;
                if git.is_some() {
                    let old = (*git).take().unwrap();
                    old.stop().await.unwrap();
                    old.rm().await.unwrap();
                    *git = None;
                }
            }
        });
    })
    .join();
}
