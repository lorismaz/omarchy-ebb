//! `helio-strap setup [name-filter]`: save the strap's auth key to the keyring,
//! fetched from a Zepp account or pasted. Opened in a terminal by the panel's
//! "Set up steps" button. The password and the key are never shown or stored
//! anywhere but the keyring.

use std::io::{BufRead, Write};

use crate::{account, keyring};

const ZEPP_SERVICES: [u16; 2] = [0xfee0, 0xfee1];

/// Bluetooth names are untrusted: drop control characters (terminal escapes).
pub fn printable(text: &str, limit: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(limit).collect()
}

struct Strap {
    name: String,
    mac: String,
}

async fn paired_straps(filter: &str) -> bluer::Result<Vec<Strap>> {
    let adapter = bluer::Session::new().await?.default_adapter().await?;
    let mut out = Vec::new();
    for addr in adapter.device_addresses().await? {
        let dev = adapter.device(addr)?;
        if !dev.is_paired().await.unwrap_or(false) {
            continue;
        }
        let uuids = dev.uuids().await.ok().flatten().unwrap_or_default();
        let zepp = ZEPP_SERVICES
            .iter()
            .any(|s| uuids.contains(&bluer::Uuid::from_u128(0x0000_0000_0000_1000_8000_0080_5f9b_34fb | ((*s as u128) << 96))));
        let name = printable(&dev.alias().await.unwrap_or_default(), 64);
        if zepp && (filter.is_empty() || name.to_lowercase().contains(&filter.to_lowercase())) {
            out.push(Strap { name, mac: addr.to_string().to_uppercase() });
        }
    }
    Ok(out)
}

fn ask(prompt: &str) -> Option<String> {
    print!("{prompt}");
    std::io::stdout().flush().ok()?;
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim().to_owned()),
    }
}

fn choose(options: &[String]) -> Option<usize> {
    for (i, o) in options.iter().enumerate() {
        println!("  {}) {o}", i + 1);
    }
    loop {
        let answer = ask("> ")?.to_lowercase();
        if matches!(answer.as_str(), "" | "q" | "quit") {
            return None;
        }
        match answer.parse::<usize>() {
            Ok(n) if (1..=options.len()).contains(&n) => return Some(n - 1),
            _ => println!("  Type a number from the list, or q to quit."),
        }
    }
}

fn from_account(mac: &str) -> Option<[u8; 16]> {
    println!("\nYour email and password go only to Zepp's servers, the way the Zepp app sends them.");
    println!("This uses Zepp's private API and only works for accounts with an email and a password.\n");
    let email = ask("Zepp email: ")?;
    let password = rpassword::prompt_password("Zepp password: ").ok()?;
    println!("Logging in to Zepp…");
    let result = account::devices(&account::Https::new(), &email, &password);
    drop(password);
    let devices = match result {
        Ok(d) => d,
        Err(e) => {
            let code = if e.code.is_empty() { String::new() } else { format!(" (Zepp error code {})", e.code) };
            println!("\n{}{code}", printable(&e.message, 200));
            println!("If you sign in to Zepp with Google, Apple or a phone number, add a password");
            println!("to your account in the Zepp app first, then run this again.");
            return None;
        }
    };
    let Some(device) = devices.iter().find(|d| d.mac == mac) else {
        println!("\nThis Zepp account has no device with address {mac}.");
        if !devices.is_empty() {
            let macs: Vec<String> = devices.iter().map(|d| printable(&d.mac, 17)).collect();
            println!("Devices in the account: {}", macs.join(", "));
        }
        return None;
    };
    if device.key.is_none() {
        println!("\nZepp returned no auth key for {mac}.");
    }
    device.key
}

fn from_paste() -> Option<[u8; 16]> {
    println!("\nPaste the 32-character auth key (for example from Gadgetbridge). It will not be shown.");
    let key = keyring::parse_key(&rpassword::prompt_password("Auth key: ").ok()?);
    if key.is_none() {
        println!("That is not a 32-character hexadecimal key.");
    }
    key
}

async fn steps(filter: &str) -> i32 {
    println!("Helio Strap · set up steps\n");
    println!("Steps come over the strap's encrypted Zepp channel, which needs the strap's");
    println!("auth key. It is saved to your keyring, not to a file.\n");

    let mut straps = match paired_straps(filter).await {
        Ok(s) => s,
        Err(e) => {
            println!("Could not read Bluetooth devices: {e}");
            return 1;
        }
    };
    if straps.is_empty() && !filter.is_empty() {
        straps = paired_straps("").await.unwrap_or_default();
    }
    let strap = match straps.len() {
        0 => {
            println!("No paired Zepp/Amazfit device found. Pair the strap in Bluetooth settings first.");
            return 1;
        }
        1 => straps.remove(0),
        _ => {
            println!("Which device?");
            let labels: Vec<String> = straps.iter().map(|s| format!("{} ({})", s.name, s.mac)).collect();
            let Some(i) = choose(&labels) else { return 1 };
            straps.remove(i)
        }
    };
    println!("Device: {} ({})\n", strap.name, strap.mac);

    println!("How do you want to add the key?");
    let methods = ["Log in with my Zepp account (email + password)".to_owned(), "Paste a key I already have".to_owned()];
    let key = match choose(&methods) {
        Some(0) => from_account(&strap.mac),
        Some(_) => from_paste(),
        None => return 1,
    };
    let Some(key) = key else { return 1 };
    if let Err(e) = keyring::store(&strap.mac, &key) {
        println!("\nCould not save to the keyring: {e}");
        return 1;
    }
    println!("\nSaved. Steps appear in the bar within a few seconds while the strap is connected.");
    0
}

pub async fn run(filter: String) -> i32 {
    let code = steps(&filter).await;
    let _ = ask("\nPress Enter to close.");
    code
}

#[cfg(test)]
mod tests {
    #[test]
    fn printable_strips_escapes() {
        assert_eq!(super::printable("Helio\x1b]0;title\x07 Strap\n", 64), "Helio]0;title Strap");
        assert_eq!(super::printable(&"x".repeat(500), 64).len(), 64);
    }
}
