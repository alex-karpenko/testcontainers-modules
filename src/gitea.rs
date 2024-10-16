use std::fs::create_dir_all;

use crate::{get_runtime_folder, Result, DOCKER_NETWORK_NAME};
use testcontainers::{
    core::{CmdWaitFor, ContainerPort, ContainerState, ExecCommand, Mount, WaitFor},
    runners::AsyncRunner as _,
    ContainerAsync, Image, ImageExt as _, TestcontainersError,
};

pub const GIT_SSH_SERVER_PORT: u16 = 22;
pub const GIT_HTTP_SERVER_PORT: u16 = 443;

pub const GITEA_IMAGE_NAME: &str = "gitea/gitea";
pub const GITEA_IMAGE_TAG: &str = "1.22.3-rootless";
pub const GITEA_SSH_PORT: ContainerPort = ContainerPort::Tcp(2222);
pub const GITEA_HTTPS_PORT: ContainerPort = ContainerPort::Tcp(3000);

pub const GITEA_DEFAULT_ADMIN_USERNAME: &str = "git-admin";
pub const GITEA_DEFAULT_ADMIN_PASSWORD: &str = "git-admin";

const CONTAINER_CONFIG_FOLDER: &str = "/etc/gitea";
const CONTAINER_DATA_FOLDER: &str = "/var/lib/gitea";

const RUNTIME_FOLDER_SUFFIX: &str = "gitea-runtime";

#[derive(Debug, Clone)]
pub struct Gitea {
    config_folder: Mount,
    data_folder: Mount,
    admin_username: String,
    admin_password: String,
}

impl Default for Gitea {
    fn default() -> Self {
        let out_dir = get_runtime_folder().unwrap();
        let config_dir = format!("{out_dir}/{RUNTIME_FOLDER_SUFFIX}/config");
        let data_dir = format!("{out_dir}/{RUNTIME_FOLDER_SUFFIX}/data");
        Self {
            config_folder: Mount::bind_mount(config_dir, CONTAINER_CONFIG_FOLDER),
            data_folder: Mount::bind_mount(data_dir, CONTAINER_DATA_FOLDER),
            admin_username: GITEA_DEFAULT_ADMIN_USERNAME.to_string(),
            admin_password: GITEA_DEFAULT_ADMIN_PASSWORD.to_string(),
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
        let mounts = vec![&self.config_folder, &self.data_folder];
        mounts
            .iter()
            .map(|m| m.source().unwrap())
            .try_for_each(create_dir_all)
            .unwrap_or_default();
        mounts.into_iter()
    }

    fn expose_ports(&self) -> &[ContainerPort] {
        &[GITEA_SSH_PORT, GITEA_HTTPS_PORT]
    }

    fn env_vars(
        &self,
    ) -> impl IntoIterator<
        Item = (
            impl Into<std::borrow::Cow<'_, str>>,
            impl Into<std::borrow::Cow<'_, str>>,
        ),
    > {
        vec![(String::from("INSTALL_LOCK"), String::from("true"))]
    }

    fn exec_after_start(&self, _cs: ContainerState) -> std::result::Result<Vec<ExecCommand>, TestcontainersError> {
        let create_admin_cmd = vec![
            "gitea",
            "admin",
            "user",
            "create",
            "--username",
            self.admin_username.as_str(),
            "--password",
            self.admin_password.as_str(),
            "--email",
            format!("{}@localhost", self.admin_username).as_str(),
            "--admin",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<String>>();

        let mut commands = vec![];
        commands.push(ExecCommand::new(create_admin_cmd).with_cmd_ready_condition(CmdWaitFor::exit_code(0)));
        Ok(commands)
    }
}

pub(crate) async fn run_git_server() -> Result<ContainerAsync<Gitea>> {
    let container = Gitea::default()
        .with_container_name("git-server")
        .with_mapped_port(GIT_SSH_SERVER_PORT, GITEA_SSH_PORT)
        .with_mapped_port(GIT_HTTP_SERVER_PORT, GITEA_HTTPS_PORT)
        .with_network(DOCKER_NETWORK_NAME)
        .start()
        .await?;

    Ok(container)
}
