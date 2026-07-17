#![no_main]
use libfuzzer_sys::fuzz_target;
use djangors_sessions::SignedCookieStore;

fuzz_target!(|data: &[u8]| {
    let cookie_val = String::from_utf8_lossy(data);
    let secret = b"super-secret-key-for-testing-purposes-only-32bytes-long";
    let store = SignedCookieStore::new(secret);
    let _ = store.decode(&cookie_val);
});
