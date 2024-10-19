use crate::{get_runtime_folder, Result, DOCKER_NETWORK_NAME};
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
use std::{collections::HashMap, fs::create_dir_all};
use testcontainers::{
    core::{CmdWaitFor, ContainerPort, ContainerState, ExecCommand, Mount, WaitFor},
    runners::AsyncRunner as _,
    ContainerAsync, Image, ImageExt as _, TestcontainersError,
};

pub const GIT_SSH_SERVER_PORT: u16 = 22;
pub const GIT_HTTP_SERVER_PORT: u16 = 80;
pub const GIT_HTTPS_SERVER_PORT: u16 = 443;

pub const GITEA_DEFAULT_ADMIN_USERNAME: &str = "git-admin";
pub const GITEA_DEFAULT_ADMIN_PASSWORD: &str = "git-admin";

const GITEA_IMAGE_NAME: &str = "gitea/gitea";
const GITEA_IMAGE_TAG: &str = "1.22.3-rootless";
const GITEA_SSH_PORT: ContainerPort = ContainerPort::Tcp(2222);
const GITEA_HTTP_PORT: ContainerPort = ContainerPort::Tcp(3000);
const GITEA_HTTP_REDIRECT_PORT: ContainerPort = ContainerPort::Tcp(3080);

const CONTAINER_CONFIG_FOLDER: &str = "/etc/gitea";
const CONTAINER_DATA_FOLDER: &str = "/var/lib/gitea";

const RUNTIME_FOLDER_SUFFIX: &str = "gitea-runtime";

const TLS_CERT_FILE_NAME: &str = "cert.pem";
const TLS_KEY_FILE_NAME: &str = "key.pem";

#[derive(Debug, Clone)]
pub struct Gitea {
    config_folder: Mount,
    data_folder: Mount,
    admin_username: String,
    admin_password: String,
    admin_key: Option<String>,
    admin_commands: Vec<Vec<String>>,
    config_env: HashMap<String, String>,
    tls: Option<GiteaTlsCert>,
    hostname: String,
    repos: Vec<GiteaRepo>,
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
            admin_key: None,
            admin_commands: vec![],
            config_env: HashMap::new(),
            tls: None,
            hostname: "localhost".to_string(),
            repos: vec![],
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
            GITEA_HTTP_PORT.as_u16()
        ))]
    }

    fn mounts(&self) -> impl IntoIterator<Item = &Mount> {
        let mounts = vec![&self.config_folder, &self.data_folder];

        // Create host folders
        mounts
            .iter()
            .map(|m| m.source().unwrap())
            .try_for_each(create_dir_all)
            .unwrap_or_default();

        // Create app.ini form "env"-based template
        let ini_template = include_str!("../assets/gitea/app.ini").to_string();
        let template_context = HashMap::from([
            ("CONTAINER_DATA_FOLDER", CONTAINER_DATA_FOLDER.to_string()),
            ("CONTAINER_CONFIG_FOLDER", CONTAINER_CONFIG_FOLDER.to_string()),
            ("GITEA_SSH_PORT", GITEA_SSH_PORT.as_u16().to_string()),
            ("GITEA_HTTP_PORT", GITEA_HTTP_PORT.as_u16().to_string()),
            ("PROTOCOL", self.protocol().to_string()),
            ("HOSTNAME", self.hostname.clone()),
        ]);

        let mut app_ini =
            shellexpand::env_with_context_no_errors(&ini_template, |v| template_context.get(v)).to_string();

        // If TLS is enabled:
        // - store TLS cert and key to the config folder,
        // - add TLS-related config to app.ini
        let config_folder = self.config_folder.source().unwrap();
        if let Some(tls_config) = &self.tls {
            tls_config.store_to(config_folder).unwrap();

            let tls_config = format!(
                "\nCERT_FILE = {}/{}\nKEY_FILE = {}/{}\nREDIRECT_OTHER_PORT = true\nPORT_TO_REDIRECT = {}\n",
                CONTAINER_CONFIG_FOLDER,
                TLS_CERT_FILE_NAME,
                CONTAINER_CONFIG_FOLDER,
                TLS_KEY_FILE_NAME,
                GITEA_HTTP_REDIRECT_PORT.as_u16()
            );
            app_ini.push_str(&tls_config);
        }
        std::fs::write(format!("{}/app.ini", config_folder), app_ini.as_bytes()).unwrap();

        mounts.into_iter()
    }

    fn expose_ports(&self) -> &[ContainerPort] {
        if self.tls.is_some() {
            &[GITEA_SSH_PORT, GITEA_HTTP_PORT, GITEA_HTTP_REDIRECT_PORT]
        } else {
            &[GITEA_SSH_PORT, GITEA_HTTP_REDIRECT_PORT]
        }
    }

    fn env_vars(
        &self,
    ) -> impl IntoIterator<
        Item = (
            impl Into<std::borrow::Cow<'_, str>>,
            impl Into<std::borrow::Cow<'_, str>>,
        ),
    > {
        self.config_env.iter()
    }

    fn exec_after_start(&self, _cs: ContainerState) -> std::result::Result<Vec<ExecCommand>, TestcontainersError> {
        let mut start_commands = vec![self.create_admin_user_cmd()];
        if let Some(key) = &self.admin_key {
            start_commands.push(self.create_admin_key_cmd(key));
        }
        self.repos.iter().for_each(|r| {
            start_commands.push(self.create_repo_cmd(r));
        });

        let admin_commands: Vec<Vec<String>> = self
            .admin_commands
            .clone()
            .into_iter()
            .map(|v| {
                vec!["gitea".to_string(), "admin".to_string()]
                    .into_iter()
                    .chain(v)
                    .collect::<Vec<String>>()
            })
            .collect();

        start_commands.extend(admin_commands);

        let commands: Vec<ExecCommand> = start_commands
            .iter()
            .map(|v| ExecCommand::new(v).with_cmd_ready_condition(CmdWaitFor::exit_code(0)))
            .collect();

        Ok(commands)
    }
}

impl Gitea {
    pub fn with_admin_account(
        self,
        username: impl Into<String>,
        password: impl Into<String>,
        key: Option<String>,
    ) -> Self {
        Self {
            admin_username: username.into(),
            admin_password: password.into(),
            admin_key: key,
            ..self
        }
    }

    pub fn with_hostname(self, hostname: impl Into<String>) -> Self {
        Self {
            hostname: hostname.into(),
            ..self
        }
    }

    pub fn with_repo(self, repo: GiteaRepo) -> Self {
        let mut repos = self.repos;
        repos.push(repo);
        Self { repos, ..self }
    }

    pub fn with_config_env(self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let mut config_env = self.config_env;
        config_env.insert(key.into(), value.into());
        Self { config_env, ..self }
    }

    pub fn with_admin_command(self, command: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let command = command.into_iter().map(|s| s.into()).collect::<Vec<String>>();
        let mut admin_commands = self.admin_commands;
        admin_commands.push(command);
        Self { admin_commands, ..self }
    }

    pub fn with_tls(self, enabled: bool) -> Self {
        Self {
            tls: if enabled { Some(GiteaTlsCert::default()) } else { None },
            ..self
        }
    }

    pub fn with_tls_certs(self, cert: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            tls: Some(GiteaTlsCert::from_pem(cert.into(), key.into())),
            ..self
        }
    }

    pub fn tls_ca(&self) -> Option<&str> {
        self.tls.as_ref().and_then(|t| t.ca())
    }

    fn create_admin_user_cmd(&self) -> Vec<String> {
        vec![
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
        .collect::<Vec<String>>()
    }

    fn create_admin_key_cmd(&self, key: &String) -> Vec<String> {
        vec![
            "curl",
            "-sk",
            "-X",
            "POST",
            "-H",
            "accept: application/json",
            "-H",
            "Content-Type: application/json",
            "-u",
            format!("{}:{}", self.admin_username, self.admin_password).as_str(),
            "-d",
            format!(r#"{{"title":"default","key":"{}","read_only":false}}"#, key).as_str(),
            self.api_url("/user/keys").as_str(),
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<String>>()
    }

    fn create_repo_cmd(&self, repo: &GiteaRepo) -> Vec<String> {
        let (repo, private) = match repo {
            GiteaRepo::Private(name) => (name, "true"),
            GiteaRepo::Public(name) => (name, "false"),
        };

        vec![
            "curl",
            "-sk",
            "-X",
            "POST",
            "-H",
            "accept: application/json",
            "-H",
            "Content-Type: application/json",
            "-u",
            format!("{}:{}", self.admin_username, self.admin_password).as_str(),
            "-d",
            format!(
                r#"{{"name":"{}","readme":"Default","auto_init":true,"private":{}}}"#,
                repo, private
            )
            .as_str(),
            self.api_url("/user/repos").as_str(),
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<String>>()
    }

    fn protocol(&self) -> &str {
        if self.tls.is_some() {
            "https"
        } else {
            "http"
        }
    }

    fn api_url(&self, api: &str) -> String {
        let api = api.strip_prefix('/').unwrap_or(api);
        format!(
            "{}://localhost:{}/api/v1/{api}",
            self.protocol(),
            GITEA_HTTP_PORT.as_u16()
        )
    }
}

#[derive(Debug, Clone)]
pub struct GiteaTlsCert {
    cert: String,
    key: String,
    ca: Option<String>,
}

impl Default for GiteaTlsCert {
    fn default() -> Self {
        Self::new("localhost")
    }
}

impl GiteaTlsCert {
    pub fn new(hostname: impl Into<String>) -> Self {
        let ca_key = KeyPair::generate().unwrap();
        let mut ca_cert = CertificateParams::new(vec!["Gitea root CA".to_string()]).unwrap();
        ca_cert.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca_cert = ca_cert.self_signed(&ca_key).unwrap();

        let mut hostnames = vec!["localhost".to_string(), "127.0.0.1".to_string(), "::1".to_string()];
        let hostname = hostname.into();
        if hostname != "localhost" {
            hostnames.insert(0, hostname);
        }

        let key = KeyPair::generate().unwrap();
        let cert = CertificateParams::new(hostnames)
            .unwrap()
            .signed_by(&key, &ca_cert, &ca_key)
            .unwrap();

        Self {
            cert: cert.pem(),
            key: key.serialize_pem(),
            ca: Some(ca_cert.pem()),
        }
    }

    pub fn from_pem(cert: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            cert: cert.into(),
            key: key.into(),
            ca: None,
        }
    }

    pub fn ca(&self) -> Option<&str> {
        self.ca.as_deref()
    }

    fn store_to(&self, out_dir: &str) -> Result<()> {
        let cert_path = format!("{out_dir}/{TLS_CERT_FILE_NAME}");
        let key_path = format!("{out_dir}/{TLS_KEY_FILE_NAME}");

        std::fs::write(cert_path, &self.cert)?;
        std::fs::write(key_path, &self.key)?;

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum GiteaRepo {
    Private(String),
    Public(String),
}

pub(crate) async fn run_git_server() -> Result<ContainerAsync<Gitea>> {
    let container = Gitea::default()
        .with_tls(true)
        .with_repo(GiteaRepo::Private("private-1".to_string()))
        .with_repo(GiteaRepo::Public("public-1".to_string()))
        .with_container_name("git-server")
        .with_mapped_port(GIT_SSH_SERVER_PORT, GITEA_SSH_PORT)
        .with_mapped_port(GIT_HTTPS_SERVER_PORT, GITEA_HTTP_PORT)
        .with_mapped_port(GIT_HTTP_SERVER_PORT, GITEA_HTTP_REDIRECT_PORT)
        .with_network(DOCKER_NETWORK_NAME)
        .start()
        .await?;

    Ok(container)
}
