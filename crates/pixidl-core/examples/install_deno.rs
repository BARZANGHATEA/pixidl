//! Developer utility: install Deno with the same verified installer the app uses.
//! `cargo run -p pixidl-core --example install_deno -- <dest-dir>`
#[tokio::main]
async fn main() {
    let dest = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: install_deno <dest-dir>"));
    let client = pixidl_core::engines::http::build_client(&pixidl_core::settings::Settings::default()).unwrap();
    match pixidl_core::tools::install_deno(&client, &dest).await {
        Ok(p) => println!("installed {}", p.display()),
        Err(e) => println!("error: {} ({:?})", e.message, e.detail),
    }
}
