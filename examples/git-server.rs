use testcontainers_modules::get_test_gitea_hostname;

#[tokio::main]
async fn main() {
    let git_host = get_test_gitea_hostname().await.unwrap();
    println!("Gitea is running on host {git_host}");

    tokio::signal::ctrl_c().await.unwrap();
    println!("Shutting down");
}
