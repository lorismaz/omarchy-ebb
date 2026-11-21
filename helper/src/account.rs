//! Download a strap's auth key from a Zepp account (PROTOCOL.md §3).
//!
//! Zepp's private API, used the way the Zepp app uses it; it can change or
//! stop working without notice. Nothing here prints, logs or stores the
//! password, the tokens or the key.

use std::time::Duration;

use serde_json::Value;
use url::form_urlencoded;

const TOKENS: &str = "https://api-user-us2.zepp.com/v2/registrations/tokens";
const LOGIN: &str = "https://api-mifit-us2.zepp.com/v2/client/login";
const LOGOUT: &str = "https://api-mifit-us2.zepp.com/v1/client/logout";
const DEVICES: &str = "https://api-mifit.zepp.com/users/";
const REDIRECT: &str = "https://s3-us-west-2.amazonaws.com/hm-registration/successsignin.html";

// The Zepp app's own fixed values (PROTOCOL.md §3); not user secrets.
const FORM_KEY: &[u8; 16] = b"xeNtBVqzDc6tuNTh";
const FORM_IV: &[u8; 16] = b"MAAAYAAAAAAAAABg";
const APP_VERSION: &str = "9.12.5";
const CV: &str = "151689_9.12.5";
const CHANNEL: &str = "a100900101016";

const MAX_BODY: u64 = 1024 * 1024;
const MAX_DEVICES: usize = 64;

#[derive(Debug)]
pub struct ZeppError {
    pub message: String,
    pub code: String,
}

fn fail(message: impl Into<String>) -> ZeppError {
    ZeppError { message: message.into(), code: String::new() }
}

pub struct Reply {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

pub trait Transport {
    fn send(&self, method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<Reply, ZeppError>;
}

/// HTTPS with a timeout, no redirects (tokens never follow one), and a cap on
/// the decoded body.
pub struct Https(ureq::Agent);

impl Https {
    pub fn new() -> Https {
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(20)))
            .build();
        Https(ureq::Agent::new_with_config(config))
    }
}

impl Transport for Https {
    fn send(&self, method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<Reply, ZeppError> {
        let unreachable = |e: ureq::Error| fail(format!("Could not reach Zepp: {e}"));
        let mut response = if method == "POST" {
            let mut r = self.0.post(url);
            for (k, v) in headers {
                r = r.header(*k, *v);
            }
            r.send(body.unwrap_or_default()).map_err(unreachable)?
        } else {
            let mut r = self.0.get(url);
            for (k, v) in headers {
                r = r.header(*k, *v);
            }
            r.call().map_err(unreachable)?
        };
        let status = response.status().as_u16();
        let location = response.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_owned);
        let body = response.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(_) => fail("Zepp sent an unexpectedly large response"),
            e => fail(format!("Could not read Zepp's response: {e}")),
        })?;
        Ok(Reply { status, location, body })
    }
}

fn app_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("appname", "com.huami.midong"),
        ("app_name", "com.huami.midong"),
        ("cv", CV),
        ("v", "2.0"),
        ("appplatform", "android_phone"),
        ("vb", "202509151347"),
        ("vn", APP_VERSION),
        ("user-agent", "Zepp/9.12.5 (Pixel 4; Android 12; Density/2.75)"),
    ]
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut f = form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        f.append_pair(k, v);
    }
    f.finish()
}

/// The encrypted credentials form for step 1.
pub fn token_body(email: &str, password: &str) -> Vec<u8> {
    let plain = form(&[
        ("emailOrPhone", email),
        ("state", "REDIRECTION"),
        ("client_id", "HuaMi"),
        ("password", password),
        ("redirect_uri", REDIRECT),
        ("region", "us-west-2"),
        ("token", "access"),
        ("token", "refresh"),
        ("country_code", "US"),
    ]);
    crate::frame::aes_cbc_pkcs7(FORM_KEY, FORM_IV, plain.as_bytes())
}

/// Step 1's answer: a 303 whose Location carries the access token.
pub fn access_token(status: u16, location: Option<&str>) -> Result<String, ZeppError> {
    let query: Vec<(String, String)> = location
        .and_then(|l| url::Url::parse(l).ok())
        .map(|u| u.query_pairs().into_owned().collect())
        .unwrap_or_default();
    let get = |k: &str| query.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let access = get("access");
    if status == 303 && !access.is_empty() && !get("refresh").is_empty() {
        return Ok(access);
    }
    let code: String = get("error").chars().filter(|c| c.is_ascii_alphanumeric() || "._-".contains(*c)).take(32).collect();
    Err(ZeppError { message: "Zepp did not accept this email and password.".into(), code })
}

pub struct Login {
    pub login_token: String,
    pub app_token: String,
    pub user_id: String,
}

pub fn login_reply(v: &Value) -> Result<Login, ZeppError> {
    let info = v.get("token_info").filter(|i| i.is_object()).ok_or_else(|| fail("Zepp login failed (no token info)."))?;
    let text = |k: &str| info.get(k).and_then(Value::as_str).map(str::to_owned);
    let user_id = match info.get("user_id") {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    match (text("login_token"), text("app_token"), user_id) {
        (Some(login_token), Some(app_token), Some(user_id)) => Ok(Login { login_token, app_token, user_id }),
        _ => Err(fail("Zepp login failed (incomplete token info).")),
    }
}

#[derive(Debug, PartialEq)]
pub struct Device {
    pub mac: String,
    pub key: Option<[u8; 16]>,
    pub active: bool,
}

pub fn devices_reply(v: &Value) -> Vec<Device> {
    let Some(items) = v.get("items").and_then(Value::as_array) else { return Vec::new() };
    items
        .iter()
        .take(MAX_DEVICES)
        .filter_map(Value::as_object)
        .map(|item| {
            let mac: String = item.get("macAddress").and_then(Value::as_str).unwrap_or("").to_uppercase().chars().take(17).collect();
            let key = item
                .get("additionalInfo")
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .and_then(|info| info.get("auth_key").and_then(Value::as_str).and_then(crate::keyring::parse_key));
            let active = match item.get("activeStatus") {
                Some(Value::Bool(b)) => *b,
                Some(Value::Number(n)) => n.as_i64() != Some(0),
                _ => false,
            };
            Device { mac, key, active }
        })
        .collect()
}

fn json(body: &[u8]) -> Result<Value, ZeppError> {
    serde_json::from_slice(body).map_err(|_| fail("Zepp sent a response that is not JSON"))
}

fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Log in, list the account's devices with their keys, and always log out.
pub fn devices(http: &dyn Transport, email: &str, password: &str) -> Result<Vec<Device>, ZeppError> {
    let mut headers = app_headers();
    headers.push(("x-hm-ekv", "1"));
    headers.push(("content-type", "application/x-www-form-urlencoded; charset=UTF-8"));
    let reply = http.send("POST", TOKENS, &headers, Some(&token_body(email, password)))?;
    let access = access_token(reply.status, reply.location.as_deref())?;

    let device_id = uuid();
    let source = format!("com.huami.watch.hmwatchmanager:{APP_VERSION}:151689");
    let body = form(&[
        ("code", &access),
        ("device_id", &device_id),
        ("device_model", "android_phone"),
        ("app_version", APP_VERSION),
        ("dn", "api-mifit.zepp.com,api-user.zepp.com,api-mifit.zepp.com,api-watch.zepp.com,app-analytics.zepp.com,auth.zepp.com,api-analytics.zepp.com"),
        ("third_name", "huami"),
        ("source", &source),
        ("app_name", "com.huami.midong"),
        ("country_code", "US"),
        ("grant_type", "access_token"),
        ("allow_registration", "false"),
        ("lang", "en"),
        ("countryState", "US-NY"),
    ]);
    let reply = http.send(
        "POST",
        LOGIN,
        &[
            ("app_name", "com.huami.webapp"),
            ("appname", "com.huami.webapp"),
            ("origin", "https://user.zepp.com"),
            ("referer", "https://user.zepp.com/"),
            ("content-type", "application/x-www-form-urlencoded; charset=UTF-8"),
            ("accept", "application/json, text/plain, */*"),
        ],
        Some(body.as_bytes()),
    )?;
    if reply.status != 200 {
        return Err(fail(format!("Zepp login failed (HTTP {}).", reply.status)));
    }
    let login = login_reply(&json(&reply.body)?)?;

    let result = (|| {
        let r = uuid();
        let app_id = {
            let mut b = [0u8; 8];
            getrandom::fill(&mut b).map_err(|_| fail("no system randomness"))?;
            u64::from_le_bytes(b).to_string()
        };
        let query = form(&[
            ("r", &r),
            ("r", &r),
            ("enableMultiDeviceOnMultiType", "true"),
            ("enableMultiDeviceOnMultiType", "true"),
            ("userid", &login.user_id),
            ("appid", &app_id),
            ("channel", CHANNEL),
            ("country", "US"),
            ("cv", CV),
            ("device", "android_32"),
            ("device_type", "android_phone"),
            ("enableMultiDevice", "true"),
            ("lang", "en_US"),
            ("timezone", "Europe/London"),
            ("v", "2.0"),
        ]);
        let user: String = form_urlencoded::byte_serialize(login.user_id.as_bytes()).collect();
        let url = format!("{DEVICES}{user}/devices?{query}");
        let request_id = uuid();
        let mut headers = app_headers();
        headers.extend([
            ("hm-privacy-diagnostics", "false"),
            ("hm-privacy-ceip", "true"),
            ("country", "US"),
            ("x-request-id", request_id.as_str()),
            ("timezone", "Europe/London"),
            ("channel", CHANNEL),
            ("apptoken", login.app_token.as_str()),
            ("lang", "en_US"),
        ]);
        let reply = http.send("GET", &url, &headers, None)?;
        if reply.status != 200 {
            return Err(fail(format!("Could not list the devices in your Zepp account (HTTP {}).", reply.status)));
        }
        Ok(devices_reply(&json(&reply.body)?))
    })();

    let mut headers = app_headers();
    headers.push(("content-type", "application/x-www-form-urlencoded; charset=UTF-8"));
    let body = form(&[("login_token", &login.login_token), ("os_verison", "vnull")]);
    let _ = http.send("POST", LOGOUT, &headers, Some(body.as_bytes()));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;

    const KEY: &str = "00112233445566778899aabbccddeeff";
    const OK: &str = "https://x/successsignin.html?access=A1&refresh=R1&country_code=US";

    #[test]
    fn token_redirect() {
        assert_eq!(access_token(303, Some(OK)).unwrap(), "A1");
        for (status, loc) in [(200, Some(OK)), (303, None), (303, Some("https://x/?error=0106"))] {
            assert!(access_token(status, loc).is_err());
        }
        assert_eq!(access_token(303, Some("https://x/?error=01<script>06")).unwrap_err().code, "01script06");
    }

    #[test]
    fn login_and_devices() {
        let l = login_reply(&json!({"token_info": {"login_token": "L", "app_token": "T", "user_id": 42}})).unwrap();
        assert_eq!((l.login_token.as_str(), l.app_token.as_str(), l.user_id.as_str()), ("L", "T", "42"));
        for bad in [json!({}), json!({"token_info": "x"}), json!({"token_info": {"login_token": "L"}}), json!([])] {
            assert!(login_reply(&bad).is_err());
        }
        let mut items = vec![
            json!({"macAddress": "a0:b1:c2:d3:e4:f5", "activeStatus": 1, "additionalInfo": json!({"auth_key": KEY}).to_string()}),
            json!({"macAddress": "AA:BB:CC:DD:EE:FF", "additionalInfo": "not json"}),
            json!({"macAddress": "<img src=x>", "additionalInfo": json!({"auth_key": "short"}).to_string()}),
            json!("junk"),
        ];
        items.extend((0..100).map(|i| json!({"macAddress": format!("00:00:00:00:00:{i:02X}")})));
        let d = devices_reply(&json!({"items": items}));
        assert_eq!(d.len(), 63);
        assert_eq!(d[0], Device { mac: "A0:B1:C2:D3:E4:F5".into(), key: crate::keyring::parse_key(KEY), active: true });
        assert!(d[1].key.is_none() && d[2].key.is_none() && d[2].mac == "<IMG SRC=X>");
        assert!(devices_reply(&json!({})).is_empty() && devices_reply(&Value::Null).is_empty());
    }

    #[test]
    fn credentials_are_encrypted() {
        let body = token_body("me@example.com", "p@ss");
        assert_eq!(body.len() % 16, 0);
        assert!(!body.windows(4).any(|w| w == b"p@ss"));
    }

    struct Fake {
        calls: RefCell<Vec<String>>,
        devices_fail: bool,
    }

    impl Transport for Fake {
        fn send(&self, method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<Reply, ZeppError> {
            self.calls.borrow_mut().push(url.split('?').next().unwrap().to_owned());
            let reply = |status, body: Value| Ok(Reply { status, location: None, body: body.to_string().into_bytes() });
            match url {
                TOKENS => Ok(Reply { status: 303, location: Some(OK.into()), body: vec![] }),
                LOGIN => {
                    let form = String::from_utf8(body.unwrap().to_vec()).unwrap();
                    assert!(form.contains("code=A1"));
                    reply(200, json!({"token_info": {"login_token": "L", "app_token": "T", "user_id": "7"}}))
                }
                LOGOUT => reply(200, json!({"result": "ok"})),
                u if u.starts_with("https://api-mifit.zepp.com/users/7/devices?") => {
                    assert_eq!(method, "GET");
                    assert!(headers.contains(&("apptoken", "T")));
                    if self.devices_fail {
                        return reply(500, json!({}));
                    }
                    let item = json!({"macAddress": "a0:b1:c2:d3:e4:f5", "additionalInfo": json!({"auth_key": KEY}).to_string()});
                    reply(200, json!({"items": [item]}))
                }
                other => panic!("unexpected request {other}"),
            }
        }
    }

    #[test]
    fn end_to_end_always_logs_out() {
        let ok = Fake { calls: RefCell::default(), devices_fail: false };
        assert_eq!(devices(&ok, "me@example.com", "p@ss").unwrap()[0].key, crate::keyring::parse_key(KEY));
        assert_eq!(ok.calls.borrow().last().unwrap(), LOGOUT);

        let failing = Fake { calls: RefCell::default(), devices_fail: true };
        assert!(devices(&failing, "me@example.com", "p@ss").is_err());
        assert_eq!(failing.calls.borrow().last().unwrap(), LOGOUT);
    }
}
