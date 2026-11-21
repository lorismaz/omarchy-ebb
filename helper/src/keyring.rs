//! The strap's auth key in the desktop keyring (PROTOCOL.md §4), through
//! libsecret's `secret-tool`. The key goes over stdin, never on a command line.

use std::io::Write;
use std::process::{Command, Stdio};

const SERVICE: &str = "omarchy-helio-strap";

/// '0x00AB…' / 'aa:bb:…' / '00ab…' -> 16 bytes
pub fn parse_key(text: &str) -> Option<[u8; 16]> {
    let lower = text.trim().to_ascii_lowercase();
    let digits: String = lower.strip_prefix("0x").unwrap_or(&lower).chars().filter(|c| !matches!(c, ':' | ' ' | '-')).collect();
    if digits.len() != 32 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut key = [0u8; 16];
    for (i, k) in key.iter_mut().enumerate() {
        *k = u8::from_str_radix(&digits[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(key)
}

pub fn lookup(address: &str) -> Option<[u8; 16]> {
    let out = Command::new("secret-tool")
        .args(["lookup", "service", SERVICE, "address", &address.to_ascii_uppercase()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    parse_key(std::str::from_utf8(&out.stdout).ok()?)
}

pub fn store(address: &str, key: &[u8; 16]) -> std::io::Result<()> {
    let address = address.to_ascii_uppercase();
    let mut child = Command::new("secret-tool")
        .args(["store", &format!("--label=Helio Strap auth key ({address})"), "service", SERVICE, "address", &address])
        .stdin(Stdio::piped())
        .spawn()?;
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    child.stdin.take().expect("piped stdin").write_all(hex.as_bytes())?;
    let status = child.wait()?;
    if status.success() { Ok(()) } else { Err(std::io::Error::other(format!("secret-tool exited with {status}"))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_formats() {
        let k = "00112233445566778899aabbccddeeff";
        let want: [u8; 16] = core::array::from_fn(|i| (i as u8) * 0x11);
        assert_eq!(parse_key(k), Some(want));
        assert_eq!(parse_key(&format!("0x{}", k.to_uppercase())), Some(want));
        let colons: Vec<&str> = (0..16).map(|i| &k[2 * i..2 * i + 2]).collect();
        assert_eq!(parse_key(&colons.join(":")), Some(want));
        assert_eq!(parse_key(&format!("{k}\n")), Some(want));
        for bad in ["", "??", &k[1..], &format!("{k}0"), &"zz".repeat(16), &format!("<b>{k}</b>")] {
            assert_eq!(parse_key(bad), None, "{bad}");
        }
    }
}
