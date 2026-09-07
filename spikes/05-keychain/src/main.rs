//! `spike-keychain set NAME VALUE`, `get NAME`, `delete NAME`, `list`.
//! Items are generic passwords with a fixed service, so they show up in
//! Keychain Access under it. `get` prints a marker line before the call
//! so a blocked prompt is visible as a missing result under `timeout`.

use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

const SERVICE: &str = "com.sadburger.switchboard.spike";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
    match arg(0) {
        "set" => match set_generic_password(SERVICE, arg(1), arg(2).as_bytes()) {
            Ok(()) => println!("set {}", arg(1)),
            Err(e) => println!("set failed: {e}"),
        },
        "get" => {
            println!("calling get {} (build 3, different code)", arg(1));
            match get_generic_password(SERVICE, arg(1)) {
                Ok(bytes) => println!("got {}={}", arg(1), String::from_utf8_lossy(&bytes)),
                Err(e) => println!("get failed: {e} (code {})", e.code()),
            }
        }
        "delete" => match delete_generic_password(SERVICE, arg(1)) {
            Ok(()) => println!("deleted {}", arg(1)),
            Err(e) => println!("delete failed: {e}"),
        },
        _ => println!("usage: set NAME VALUE | get NAME | delete NAME"),
    }
}
