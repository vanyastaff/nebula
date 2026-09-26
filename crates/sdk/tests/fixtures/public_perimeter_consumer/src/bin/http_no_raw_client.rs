//! The HTTP transport's client is private: an author cannot reach the raw
//! pool and send around the managed facade, its admission and its limit.

use nebula_sdk::integration::resource::http::{HttpConfig, HttpTransport};

fn main() {
    let transport = HttpTransport::new(&HttpConfig::new("https://api.example.com")).unwrap();
    let _raw = &transport.inner;
}
