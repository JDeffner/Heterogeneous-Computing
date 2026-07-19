//! Small shared helpers: ISO timestamps and id generation (base36, like the TS).
use chrono::Utc;
use rand::Rng;

/// ISO 8601 / RFC3339 with millisecond precision and a trailing Z, matching
/// JavaScript's `new Date().toISOString()`.
pub fn now_iso() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn to_base36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// Generate an id like `evt-lq3k1-a9f2` (prefix + base36 millis + random suffix).
pub fn new_id(prefix: &str) -> String {
    let millis = Utc::now().timestamp_millis().max(0) as u128;
    let mut rng = rand::thread_rng();
    let rand_part: u32 = rng.gen_range(0..36u32.pow(5));
    format!("{prefix}-{}-{}", to_base36(millis), to_base36(rand_part as u128))
}
