use crate::{init_crypto_provider, Error, Result, DOCKER_NETWORK_NAME};
use kube::{
    config::{KubeConfigOptions, Kubeconfig},
    Config,
};
use std::{borrow::Cow, path::Path};
use testcontainers::{
    core::{ContainerPort, Mount, WaitFor},
    runners::AsyncRunner as _,
    ContainerAsync, Image, ImageExt as _,
};

pub const K3S_KUBECONFIG_PORT: u16 = 9443;

pub const K3S_KUBE_API_PORT: ContainerPort = ContainerPort::Tcp(6443);
pub const K3S_TRAEFIK_HTTP_PORT: ContainerPort = ContainerPort::Tcp(80);
pub const K3S_RANCHER_WEBHOOK_PORT: ContainerPort = ContainerPort::Tcp(8443);

pub const K3S_IMAGE_NAME: &str = "rancher/k3s";
pub const KUBE_VERSION_DEFAULT: &str = "1.31";
const AVAILABLE_K3S_IMAGE_TAGS: [(&str, &str); 6] = [
    ("1.31", "v1.31.1-k3s1"),
    ("1.30", "v1.30.5-k3s1"),
    ("1.29", "v1.29.9-k3s1"),
    ("1.28", "v1.28.14-k3s1"),
    ("1.27", "v1.27.16-k3s1"),
    ("1.26", "v1.26.15-k3s1"),
];

const RUNTIME_FOLDER_SUFFIX: &str = "k3s-runtime";

#[derive(Debug, Clone)]
pub struct K3s {
    kubeconfig_mount: Mount,
    tag: String,
    cmd: K3sCmd,
}

impl Default for K3s {
    fn default() -> Self {
        let build_out_dir = crate::get_runtime_folder().unwrap();
        Self {
            kubeconfig_mount: Mount::bind_mount(
                format!("{build_out_dir}/{RUNTIME_FOLDER_SUFFIX}"),
                "/etc/rancher/k3s/",
            ),
            tag: version_to_tag(KUBE_VERSION_DEFAULT).unwrap(),
            cmd: K3sCmd::default(),
        }
    }
}

fn version_to_tag(version: impl Into<String>) -> Result<String> {
    let version = version.into();
    let version = version.strip_prefix("v").map(String::from).unwrap_or(version);
    let version = if version.is_empty() || version == "latest" {
        KUBE_VERSION_DEFAULT
    } else {
        version.as_str()
    };

    AVAILABLE_K3S_IMAGE_TAGS
        .iter()
        .find(|(k, _)| *k == version)
        .map(|(_, v)| *v)
        .ok_or_else(|| Error::RuntimeConfig(format!("Kube version '{}' is not supported", version)))
        .map(String::from)
}

#[derive(Debug, Clone)]
struct K3sCmd {
    snapshotter: String,
    traefik: bool,
    network_policy: bool,
    coredns: bool,
    service_lb: bool,
    local_storage: bool,
    metrics_server: bool,
    helm_controller: bool,
    agent: bool,
}

impl Default for K3sCmd {
    fn default() -> Self {
        Self {
            snapshotter: "native".to_string(),
            traefik: true,
            network_policy: true,
            coredns: true,
            service_lb: true,
            local_storage: true,
            metrics_server: true,
            helm_controller: true,
            agent: true,
        }
    }
}

impl IntoIterator for &K3sCmd {
    type Item = String;
    type IntoIter = <Vec<String> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        let mut cmd = vec![String::from("server"), format!("--snapshotter={}", self.snapshotter)];

        if !self.traefik {
            cmd.push("--disable=traefik".to_string());
        }
        if !self.service_lb {
            cmd.push("--disable=servicelb".to_string());
        }
        if !self.coredns {
            cmd.push("--disable=coredns".to_string());
        }
        if !self.agent {
            cmd.push("--disable-agent".to_string());
        }
        if !self.helm_controller {
            cmd.push("--disable-helm-controller".to_string());
        }
        if !self.local_storage {
            cmd.push("--disable=local-storage".to_string());
        }
        if !self.metrics_server {
            cmd.push("--disable=metrics-server".to_string());
        }
        if !self.network_policy {
            cmd.push("--disable-network-policy".to_string());
        }

        cmd.into_iter()
    }
}

impl Image for K3s {
    fn name(&self) -> &str {
        K3S_IMAGE_NAME
    }

    fn tag(&self) -> &str {
        self.tag.as_str()
    }

    fn ready_conditions(&self) -> Vec<WaitFor> {
        vec![WaitFor::message_on_stderr("Node controller sync successful")]
    }

    fn env_vars(&self) -> impl IntoIterator<Item = (impl Into<Cow<'_, str>>, impl Into<Cow<'_, str>>)> {
        vec![(String::from("K3S_KUBECONFIG_MODE"), String::from("644"))]
    }

    fn mounts(&self) -> impl IntoIterator<Item = &Mount> {
        vec![&self.kubeconfig_mount]
    }

    fn cmd(&self) -> impl IntoIterator<Item = impl Into<Cow<'_, str>>> {
        &self.cmd
    }

    fn expose_ports(&self) -> &[ContainerPort] {
        if self.cmd.traefik {
            &[K3S_KUBE_API_PORT, K3S_RANCHER_WEBHOOK_PORT, K3S_TRAEFIK_HTTP_PORT]
        } else {
            &[K3S_KUBE_API_PORT, K3S_RANCHER_WEBHOOK_PORT]
        }
    }
}

impl K3s {
    pub fn with_kube_version(self, version: impl Into<String>) -> Self {
        Self {
            tag: version_to_tag(version).unwrap(),
            ..self
        }
    }

    pub fn with_snapshotter(self, snapshotter: impl Into<String>) -> Self {
        Self {
            cmd: K3sCmd {
                snapshotter: snapshotter.into(),
                ..self.cmd
            },
            ..self
        }
    }

    pub fn with_traefik(self, traefik: bool) -> Self {
        Self {
            cmd: K3sCmd { traefik, ..self.cmd },
            ..self
        }
    }

    pub fn with_service_lb(self, service_lb: bool) -> Self {
        Self {
            cmd: K3sCmd { service_lb, ..self.cmd },
            ..self
        }
    }

    pub fn with_coredns(self, coredns: bool) -> Self {
        Self {
            cmd: K3sCmd { coredns, ..self.cmd },
            ..self
        }
    }

    pub fn with_agent(self, agent: bool) -> Self {
        Self {
            cmd: K3sCmd { agent, ..self.cmd },
            ..self
        }
    }

    pub fn with_helm_controller(self, helm_controller: bool) -> Self {
        Self {
            cmd: K3sCmd {
                helm_controller,
                ..self.cmd
            },
            ..self
        }
    }

    pub fn with_local_storage(self, local_storage: bool) -> Self {
        Self {
            cmd: K3sCmd {
                local_storage,
                ..self.cmd
            },
            ..self
        }
    }

    pub fn with_metrics_server(self, metrics_server: bool) -> Self {
        Self {
            cmd: K3sCmd {
                metrics_server,
                ..self.cmd
            },
            ..self
        }
    }

    pub fn with_network_policy(self, network_policy: bool) -> Self {
        Self {
            cmd: K3sCmd {
                network_policy,
                ..self.cmd
            },
            ..self
        }
    }

    pub fn with_all_features(self, all_features: bool) -> Self {
        Self {
            cmd: K3sCmd {
                traefik: all_features,
                service_lb: all_features,
                coredns: all_features,
                helm_controller: all_features,
                network_policy: all_features,
                local_storage: all_features,
                metrics_server: all_features,
                ..self.cmd
            },
            ..self
        }
    }

    pub async fn get_kubeconfig(&self) -> Result<String> {
        let kubeconfig_mount = self.kubeconfig_mount.source().unwrap();
        let k3s_conf_file_path = Path::new(&kubeconfig_mount).join("k3s.yaml");
        tokio::fs::read_to_string(k3s_conf_file_path).await.map_err(Error::Io)
    }

    pub async fn get_client(container: &ContainerAsync<K3s>) -> Result<kube::Client> {
        init_crypto_provider();

        let conf_yaml = container.image().get_kubeconfig().await?;
        let mut config = Kubeconfig::from_yaml(&conf_yaml).expect("Error loading kube config");

        let port = container.get_host_port_ipv4(K3S_KUBE_API_PORT).await?;
        config.clusters.iter_mut().for_each(|cluster| {
            if let Some(server) = cluster.cluster.as_mut().and_then(|c| c.server.as_mut()) {
                *server = format!("https://127.0.0.1:{}", port)
            }
        });

        let client_config = Config::from_custom_kubeconfig(config, &KubeConfigOptions::default()).await?;

        Ok(kube::Client::try_from(client_config)?)
    }
}

pub(crate) async fn run_k3s_cluster(k3s_base_dir: &String) -> Result<ContainerAsync<K3s>> {
    tokio::fs::create_dir_all(k3s_base_dir).await?;

    let container = K3s::default()
        .with_all_features(false)
        .with_container_name("k3s")
        .with_userns_mode("host")
        .with_privileged(true)
        .with_mapped_port(K3S_KUBECONFIG_PORT, K3S_KUBE_API_PORT)
        .with_network(DOCKER_NETWORK_NAME)
        .start()
        .await?;

    Ok(container)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_to_tag_correct() {
        assert_eq!(version_to_tag("").unwrap(), AVAILABLE_K3S_IMAGE_TAGS[0].1.to_string());
        assert_eq!(
            version_to_tag("latest").unwrap(),
            AVAILABLE_K3S_IMAGE_TAGS[0].1.to_string()
        );
        assert_eq!(
            version_to_tag(KUBE_VERSION_DEFAULT).unwrap(),
            AVAILABLE_K3S_IMAGE_TAGS[0].1.to_string()
        );

        let v1_26 = AVAILABLE_K3S_IMAGE_TAGS
            .iter()
            .filter(|(k, _)| *k == "1.26")
            .map(|(_, v)| *v)
            .take(1)
            .collect::<Vec<&str>>()[0];
        let v1_27 = AVAILABLE_K3S_IMAGE_TAGS
            .iter()
            .filter(|(k, _)| *k == "1.27")
            .map(|(_, v)| *v)
            .take(1)
            .collect::<Vec<&str>>()[0];

        assert_eq!(version_to_tag("1.26").unwrap(), v1_26);
        assert_eq!(version_to_tag("v1.27").unwrap(), v1_27);
    }

    #[test]
    fn version_to_tag_incorrect() {
        assert!(matches!(version_to_tag("1.10"), Err(Error::RuntimeConfig(_))));
        assert!(matches!(version_to_tag("-"), Err(Error::RuntimeConfig(_))));
    }
}
