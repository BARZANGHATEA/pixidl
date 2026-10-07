//! Developer utility: probe a URL with the same HTTP client the app uses.
//! `cargo run -p nexa-core --example probe -- <url>`
#[tokio::main]
async fn main() {
    let url = std::env::args().nth(1).expect("usage: probe <url>");
    let client = nexa_core::engines::http::build_client(&nexa_core::settings::Settings::default()).unwrap();
    match nexa_core::engines::http::probe(&client, &url, None).await {
        Ok(p) => println!("{p:#?}"),
        Err(e) => println!("error: {} — {:?} ({:?})", e.message, e.kind, e.detail),
    }
}
