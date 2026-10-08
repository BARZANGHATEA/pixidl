//! Developer utility: probe a URL with the same HTTP client the app uses.
//! `cargo run -p pixidl-core --example probe -- <url>`
#[tokio::main]
async fn main() {
    let url = std::env::args().nth(1).expect("usage: probe <url>");
    let client = pixidl_core::engines::http::build_client(&pixidl_core::settings::Settings::default()).unwrap();
    match pixidl_core::engines::http::probe(&client, &url, None).await {
        Ok(p) => println!("{p:#?}"),
        Err(e) => println!("error: {} — {:?} ({:?})", e.message, e.kind, e.detail),
    }
}
