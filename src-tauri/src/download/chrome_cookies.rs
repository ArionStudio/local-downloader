use super::BrowserAuthSource;
#[cfg(target_os = "linux")]
use aes::Aes128;
#[cfg(target_os = "linux")]
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
#[cfg(target_os = "linux")]
use dbus::{
    arg::{RefArg, Variant},
    blocking::Connection as DbusConnection,
    Path as DbusPath,
};
#[cfg(target_os = "linux")]
use pbkdf2::pbkdf2_hmac;
#[cfg(target_os = "linux")]
use rusqlite::{Connection, OpenFlags};
#[cfg(target_os = "linux")]
use sha1::Sha1;
#[cfg(target_os = "linux")]
use std::{
    collections::BTreeMap,
    env,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use std::path::PathBuf;
use url::Url;

#[cfg(target_os = "linux")]
type Aes128CbcDec = cbc::Decryptor<Aes128>;
#[cfg(target_os = "linux")]
type SecretValue = (DbusPath<'static>, Vec<u8>, Vec<u8>, String);

#[cfg(target_os = "linux")]
const CHROME_SECRET_APPLICATION: &str = "chrome";
#[cfg(target_os = "linux")]
const CHROME_SECRET_SCHEMA: &str = "chrome_libsecret_os_crypt_password_v2";
#[cfg(target_os = "linux")]
const CHROME_COOKIE_EPOCH_OFFSET_SECONDS: i64 = 11_644_473_600;

pub fn can_export(source: &BrowserAuthSource) -> bool {
    cfg!(target_os = "linux") && source.browser.trim().eq_ignore_ascii_case("chrome")
}

#[cfg(target_os = "linux")]
pub fn export(source: &BrowserAuthSource, target_url: &str) -> Result<PathBuf, String> {
    if !can_export(source) {
        return Err("Targeted cookie export is only available for Chrome on Linux.".to_string());
    }

    let target_host = target_cookie_host(target_url)?;
    let cookie_db = chrome_cookie_db_path(source)?;
    let keyring_secret = chrome_keyring_secret()?;
    let v11_key = derive_linux_key(&keyring_secret);
    let v10_key = derive_linux_key(b"peanuts");
    let empty_key = derive_linux_key(b"");
    let meta_version = chrome_cookie_meta_version(&cookie_db).unwrap_or_default();
    let hash_prefix = meta_version >= 24;

    let output_path = env::temp_dir().join(format!(
        "downloader-chrome-cookies-{}.txt",
        temp_suffix()
    ));
    export_cookie_db(
        &cookie_db,
        &output_path,
        &v10_key,
        &v11_key,
        &empty_key,
        &target_host,
        hash_prefix,
    )?;
    Ok(output_path)
}

#[cfg(not(target_os = "linux"))]
pub fn export(_source: &BrowserAuthSource, _target_url: &str) -> Result<PathBuf, String> {
    Err("Targeted cookie export is only available for Chrome on Linux.".to_string())
}

fn target_cookie_host(target_url: &str) -> Result<String, String> {
    Url::parse(target_url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .ok_or_else(|| "Target URL does not have a valid host.".to_string())
}

#[cfg(target_os = "linux")]
fn chrome_cookie_db_path(source: &BrowserAuthSource) -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or_else(|| "HOME is not set.".to_string())?;
    let profile = source
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Default");
    let path = PathBuf::from(home)
        .join(".config")
        .join("google-chrome")
        .join(profile)
        .join("Cookies");

    if path.exists() {
        Ok(path)
    } else {
        Err(format!(
            "Chrome cookie database was not found at {}.",
            path.display()
        ))
    }
}

#[cfg(target_os = "linux")]
fn chrome_keyring_secret() -> Result<Vec<u8>, String> {
    let connection = DbusConnection::new_session()
        .map_err(|error| format!("Could not connect to Secret Service: {error}"))?;
    let service = connection.with_proxy(
        "org.freedesktop.secrets",
        "/org/freedesktop/secrets",
        Duration::from_secs(5),
    );

    let input = Variant(Box::new(String::new()) as Box<dyn RefArg>);
    let (_, session): (Variant<Box<dyn RefArg>>, DbusPath<'static>) = service
        .method_call(
            "org.freedesktop.Secret.Service",
            "OpenSession",
            ("plain", input),
        )
        .map_err(|error| format!("Could not open Secret Service session: {error}"))?;

    let mut attributes = BTreeMap::new();
    attributes.insert("application", CHROME_SECRET_APPLICATION);
    attributes.insert("xdg:schema", CHROME_SECRET_SCHEMA);
    let (unlocked, _locked): (Vec<DbusPath<'static>>, Vec<DbusPath<'static>>) = service
        .method_call(
            "org.freedesktop.Secret.Service",
            "SearchItems",
            (attributes,),
        )
        .map_err(|error| format!("Could not search Secret Service for Chrome key: {error}"))?;

    let item_path = unlocked
        .into_iter()
        .next()
        .ok_or_else(|| "Chrome Safe Storage key was not found in Secret Service.".to_string())?;
    let item = connection.with_proxy(
        "org.freedesktop.secrets",
        item_path,
        Duration::from_secs(5),
    );
    let (secret,): (SecretValue,) = item
        .method_call("org.freedesktop.Secret.Item", "GetSecret", (session,))
        .map_err(|error| format!("Could not read Chrome Safe Storage key: {error}"))?;

    if secret.2.is_empty() {
        Err("Chrome Safe Storage key is empty.".to_string())
    } else {
        Ok(secret.2)
    }
}

#[cfg(target_os = "linux")]
fn chrome_cookie_meta_version(cookie_db: &Path) -> Result<i64, String> {
    let connection = Connection::open_with_flags(cookie_db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("Could not open Chrome cookie database: {error}"))?;
    connection
        .query_row("SELECT value FROM meta WHERE key = 'version'", [], |row| {
            let value: String = row.get(0)?;
            Ok(value.parse::<i64>().unwrap_or_default())
        })
        .map_err(|error| format!("Could not read Chrome cookie database version: {error}"))
}

#[cfg(target_os = "linux")]
fn export_cookie_db(
    cookie_db: &Path,
    output_path: &Path,
    v10_key: &[u8; 16],
    v11_key: &[u8; 16],
    empty_key: &[u8; 16],
    target_host: &str,
    hash_prefix: bool,
) -> Result<(), String> {
    let connection = Connection::open_with_flags(cookie_db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("Could not open Chrome cookie database: {error}"))?;
    let file = File::create(output_path)
        .map_err(|error| format!("Could not create temporary cookie file: {error}"))?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "# Netscape HTTP Cookie File")
        .map_err(|error| format!("Could not write temporary cookie file: {error}"))?;

    let mut statement = connection
        .prepare(
            "SELECT host_key, name, value, encrypted_value, path, expires_utc, is_secure, is_httponly
             FROM cookies
             ORDER BY host_key, name",
        )
        .map_err(|error| format!("Could not query Chrome cookies: {error}"))?;
    let cookies = statement
        .query_map([], |row| {
            Ok(ChromeCookie {
                host: row.get(0)?,
                name: row.get(1)?,
                value: row.get(2)?,
                encrypted_value: row.get(3)?,
                path: row.get(4)?,
                expires_utc: row.get(5)?,
                secure: row.get::<_, i64>(6)? != 0,
                http_only: row.get::<_, i64>(7)? != 0,
            })
        })
        .map_err(|error| format!("Could not read Chrome cookies: {error}"))?;

    let mut exported = 0usize;
    for cookie in cookies {
        let cookie = cookie.map_err(|error| format!("Could not read Chrome cookie: {error}"))?;
        if !cookie_matches_target_host(&cookie.host, target_host) {
            continue;
        }
        let Some(value) = cookie_value(&cookie, v10_key, v11_key, empty_key, hash_prefix) else {
            continue;
        };
        write_netscape_cookie(&mut writer, &cookie, &value)?;
        exported += 1;
    }

    writer
        .flush()
        .map_err(|error| format!("Could not finish temporary cookie file: {error}"))?;

    if exported == 0 {
        Err("No Chrome cookies could be exported.".to_string())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
struct ChromeCookie {
    host: String,
    name: String,
    value: String,
    encrypted_value: Vec<u8>,
    path: String,
    expires_utc: i64,
    secure: bool,
    http_only: bool,
}

#[cfg(target_os = "linux")]
fn cookie_value(
    cookie: &ChromeCookie,
    v10_key: &[u8; 16],
    v11_key: &[u8; 16],
    empty_key: &[u8; 16],
    hash_prefix: bool,
) -> Option<String> {
    if !cookie.value.is_empty() {
        return Some(cookie.value.clone());
    }

    if cookie.encrypted_value.len() < 3 {
        return None;
    }
    let (version, ciphertext) = cookie.encrypted_value.split_at(3);
    let keys = match version {
        b"v10" => [Some(v10_key), Some(empty_key), None],
        b"v11" => [Some(v11_key), Some(empty_key), None],
        _ => [None, None, None],
    };

    keys.into_iter().flatten().find_map(|key| {
        decrypt_cookie_value(ciphertext, key, hash_prefix)
            .and_then(|value| String::from_utf8(value).ok())
    })
}

#[cfg(target_os = "linux")]
fn decrypt_cookie_value(ciphertext: &[u8], key: &[u8; 16], hash_prefix: bool) -> Option<Vec<u8>> {
    let mut plaintext = Aes128CbcDec::new(key.into(), (&[b' '; 16]).into())
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .ok()?;
    if hash_prefix {
        if plaintext.len() < 32 {
            return None;
        }
        plaintext.drain(..32);
    }
    Some(plaintext)
}

#[cfg(target_os = "linux")]
fn write_netscape_cookie(
    writer: &mut BufWriter<File>,
    cookie: &ChromeCookie,
    value: &str,
) -> Result<(), String> {
    let domain = if cookie.http_only {
        format!("#HttpOnly_{}", cookie.host)
    } else {
        cookie.host.clone()
    };
    let include_subdomains = if cookie.host.starts_with('.') {
        "TRUE"
    } else {
        "FALSE"
    };
    let secure = if cookie.secure { "TRUE" } else { "FALSE" };
    let expires = chrome_time_to_unix(cookie.expires_utc).unwrap_or_default();

    writeln!(
        writer,
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        domain, include_subdomains, cookie.path, secure, expires, cookie.name, value
    )
    .map_err(|error| format!("Could not write temporary cookie file: {error}"))
}

fn cookie_matches_target_host(cookie_host: &str, target_host: &str) -> bool {
    let cookie_host = cookie_host.trim_start_matches('.').to_ascii_lowercase();
    let target_host = target_host.trim_start_matches('.').to_ascii_lowercase();

    target_host == cookie_host || target_host.ends_with(&format!(".{cookie_host}"))
}

#[cfg(target_os = "linux")]
fn chrome_time_to_unix(value: i64) -> Option<i64> {
    if value <= 0 {
        None
    } else {
        Some((value / 1_000_000) - CHROME_COOKIE_EPOCH_OFFSET_SECONDS)
    }
}

#[cfg(target_os = "linux")]
fn derive_linux_key(password: &[u8]) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2_hmac::<Sha1>(password, b"saltysalt", 1, &mut key);
    key
}

#[cfg(target_os = "linux")]
fn temp_suffix() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{now}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "linux")]
    fn converts_chrome_cookie_timestamp_to_unix() {
        assert_eq!(chrome_time_to_unix(0), None);
        assert_eq!(chrome_time_to_unix(13_369_737_600_000_000), Some(1_725_264_000));
    }

    #[test]
    fn detects_linux_chrome_source() {
        assert_eq!(
            can_export(&BrowserAuthSource {
                browser: "chrome".to_string(),
                profile: None,
            }),
            cfg!(target_os = "linux")
        );
        assert!(!can_export(&BrowserAuthSource {
            browser: "firefox".to_string(),
            profile: None,
        }));
    }

    #[test]
    fn matches_cookie_domains_for_target_url() {
        assert_eq!(
            target_cookie_host("https://www.linkedin.com/feed/").as_deref(),
            Ok("www.linkedin.com")
        );
        assert!(cookie_matches_target_host(".linkedin.com", "www.linkedin.com"));
        assert!(cookie_matches_target_host("www.linkedin.com", "www.linkedin.com"));
        assert!(!cookie_matches_target_host(
            "example.com",
            "www.linkedin.com"
        ));
    }
}
