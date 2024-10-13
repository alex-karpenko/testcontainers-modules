use crate::{Result, DOCKER_NETWORK_NAME};
use testcontainers::{
    core::{ContainerPort, Mount, WaitFor},
    runners::AsyncRunner as _,
    ContainerAsync, Image, ImageExt as _,
};

pub const GIT_SSH_SERVER_PORT: u16 = 22;
pub const GIT_HTTP_SERVER_PORT: u16 = 443;

pub const GITEA_IMAGE_NAME: &str = "gitea/gitea";
pub const GITEA_IMAGE_TAG: &str = "1.22.2-rootless";
pub const GITEA_SSH_PORT: ContainerPort = ContainerPort::Tcp(2222);
pub const GITEA_HTTPS_PORT: ContainerPort = ContainerPort::Tcp(3000);

const CONTAINER_CONFIG_FOLDER: &str = "/etc/gitea";
const CONTAINER_DATA_FOLDER: &str = "/var/lib/gitea";

#[derive(Debug, Clone)]
pub struct Gitea {
    config_folder: Mount,
    data_folder: Mount,
}

impl Gitea {
    pub fn new(config_folder: impl Into<String>, data_folder: impl Into<String>) -> Self {
        Self {
            config_folder: Mount::bind_mount(config_folder, CONTAINER_CONFIG_FOLDER),
            data_folder: Mount::bind_mount(data_folder, CONTAINER_DATA_FOLDER),
        }
    }
}

impl Image for Gitea {
    fn name(&self) -> &str {
        GITEA_IMAGE_NAME
    }

    fn tag(&self) -> &str {
        GITEA_IMAGE_TAG
    }

    fn ready_conditions(&self) -> Vec<WaitFor> {
        vec![WaitFor::message_on_stdout(format!(
            "Starting new Web server: tcp:0.0.0.0:{}",
            GITEA_HTTPS_PORT.as_u16()
        ))]
    }

    fn mounts(&self) -> impl IntoIterator<Item = &Mount> {
        vec![&self.config_folder, &self.data_folder]
    }

    fn expose_ports(&self) -> &[ContainerPort] {
        &[GITEA_SSH_PORT, GITEA_HTTPS_PORT]
    }
}

pub(crate) async fn run_git_server(gitea_base_dir: &String) -> Result<ContainerAsync<Gitea>> {
    let config_dir = format!("{gitea_base_dir}/config");
    let data_dir = format!("{gitea_base_dir}/data");

    let container = Gitea::new(config_dir, data_dir)
        .with_container_name("git-server")
        .with_mapped_port(GIT_SSH_SERVER_PORT, GITEA_SSH_PORT)
        .with_mapped_port(GIT_HTTP_SERVER_PORT, GITEA_HTTPS_PORT)
        .with_network(DOCKER_NETWORK_NAME)
        .start()
        .await?;

    Ok(container)
}
