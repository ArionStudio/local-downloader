use keyring::{Entry, Error};

const SERVICE: &str = "dev.local.downloader.youtube-data-api";

fn entry(id: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, id)
        .map_err(|error| format!("Could not access the OS credential vault: {error}"))
}

pub fn store(id: &str, api_key: &str) -> Result<(), String> {
    entry(id)?
        .set_password(api_key)
        .map_err(|error| format!("Could not save the API key in the OS credential vault: {error}"))
}

pub fn load_optional(id: &str) -> Result<Option<String>, String> {
    match entry(id)?.get_password() {
        Ok(api_key) => Ok(Some(api_key)),
        Err(Error::NoEntry) => Ok(None),
        Err(error) => Err(format!(
            "Could not read an API key from the OS credential vault: {error}"
        )),
    }
}

pub fn remove(id: &str) -> Result<(), String> {
    match entry(id)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(format!(
            "Could not remove the API key from the OS credential vault: {error}"
        )),
    }
}

pub fn load_all(ids: &[String]) -> Result<Vec<String>, String> {
    ids.iter()
        .map(|id| load_optional(id))
        .collect::<Result<Vec<_>, _>>()
        .map(|keys| keys.into_iter().flatten().collect())
}
