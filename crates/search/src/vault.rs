use crate::{AuthMode, ServiceKind, ServiceView, ServiceWrite, SearchError};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs::{self,File,OpenOptions}, io::{Read,Write}, path::{Path,PathBuf}};

#[derive(Clone,Serialize,Deserialize)]
pub(crate) struct Service {
    pub id: String,
    pub name: String,
    pub kind: ServiceKind,
    pub enabled: bool,
    pub auth_mode: AuthMode,
    pub model: Option<String>,
    pub api_key: Option<String>,
}
impl Service {
    pub fn view(&self) -> ServiceView {
        let has_api_key = self.auth_mode == AuthMode::ApiKey && self.api_key.is_some();
        ServiceView { id: self.id.clone(), name: self.name.clone(), kind: self.kind, enabled: self.enabled, auth_mode: self.auth_mode, model: self.model.clone(), has_api_key, connected: has_api_key, supports_fetch: true }
    }
    pub fn apply(&mut self, value: ServiceWrite) -> Result<(),SearchError> {
        if value.name.trim().is_empty() || value.name.chars().count() > 100 || value.name.chars().any(char::is_control) { return Err(SearchError::new(400,"invalid_service_name")); }
        if value.kind == ServiceKind::Deepseek && value.auth_mode != AuthMode::ApiKey { return Err(SearchError::new(400,"deepseek_requires_api_key_mode")); }
        if value.model.as_ref().is_some_and(|model| model.is_empty() || model.len() > 100 || !model.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c,b'-'|b'_'|b'.'))) { return Err(SearchError::new(400,"invalid_service_model")); }
        if value.api_key.as_ref().is_some_and(|key| key.len() > 16384 || key.chars().any(char::is_control)) { return Err(SearchError::new(400,"invalid_api_key")); }
        if value.clear_api_key && value.api_key.as_ref().is_some_and(|key| !key.trim().is_empty()) { return Err(SearchError::new(400,"conflicting_api_key_update")); }
        if value.auth_mode == AuthMode::SharedChatgpt && value.api_key.as_ref().is_some_and(|key| !key.trim().is_empty()) { return Err(SearchError::new(400,"shared_mode_does_not_accept_api_key")); }
        if self.kind != value.kind || self.auth_mode != value.auth_mode || value.auth_mode == AuthMode::SharedChatgpt { self.api_key = None; }
        self.name = value.name.trim().to_owned();
        self.kind = value.kind;
        self.enabled = value.enabled;
        self.auth_mode = value.auth_mode;
        self.model = value.model;
        if value.clear_api_key { self.api_key = None; }
        else if let Some(key) = value.api_key.filter(|key| !key.trim().is_empty()) { self.api_key = Some(key); }
        Ok(())
    }
}

pub(crate) struct Vault { pub services: BTreeMap<String,Service>, dir: PathBuf, _lock: File }
fn disk_error() -> SearchError { SearchError::new(502,"private_storage_unavailable") }
fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    options
}
impl Vault {
    pub fn open(dir: &Path) -> Result<Self,SearchError> {
        if let Ok(metadata) = fs::symlink_metadata(dir) { if metadata.file_type().is_symlink() || !metadata.is_dir() { return Err(disk_error()); } }
        fs::create_dir_all(dir).map_err(|_| disk_error())?;
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; fs::set_permissions(dir,fs::Permissions::from_mode(0o700)).map_err(|_| disk_error())?; }
        let lock = private_options().create(true).truncate(false).open(dir.join("engine.lock")).map_err(|_| disk_error())?;
        protect_file(&lock)?;
        lock.try_lock_exclusive().map_err(|_| SearchError::new(409,"engine_data_already_locked"))?;
        let path = dir.join("services.json");
        let services: BTreeMap<String,Service> = if path.exists() {
            let file = private_options().open(&path).map_err(|_| disk_error())?;
            protect_file(&file)?;
            let mut bytes = Vec::new();
            file.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|_| disk_error())?;
            if bytes.len() > 2 * 1024 * 1024 { return Err(disk_error()); }
            serde_json::from_slice(&bytes).map_err(|_| SearchError::new(502,"private_storage_invalid"))?
        } else { BTreeMap::new() };
        if services.len() > 128 || services.iter().any(|(id,service)| id != &service.id || crate::validation::uuid(id).is_err()) { return Err(SearchError::new(502,"private_storage_invalid")); }
        Ok(Self { services, dir: dir.to_owned(), _lock: lock })
    }
    pub fn replace(&mut self, service: Service) -> Result<(),SearchError> {
        let id = service.id.clone();
        let old = self.services.insert(id.clone(),service);
        if let Err(error) = self.save() {
            if let Some(old) = old { self.services.insert(id,old); } else { self.services.remove(&id); }
            return Err(error);
        }
        Ok(())
    }
    pub fn remove(&mut self, id: &str) -> Result<(),SearchError> {
        let old = self.services.remove(id).ok_or(SearchError::new(404,"service_not_found"))?;
        if let Err(error) = self.save() { self.services.insert(id.to_owned(),old); return Err(error); }
        Ok(())
    }
    fn save(&self) -> Result<(),SearchError> {
        let temporary = self.dir.join(format!(".services-{}.tmp",uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = private_options().create_new(true).open(&temporary).map_err(|_| disk_error())?;
            {
                let mut writer = LimitedWriter { inner:std::io::BufWriter::new(&mut file),remaining:2 * 1024 * 1024 };
                serde_json::to_writer(&mut writer,&self.services).map_err(|_| disk_error())?;
                writer.flush().map_err(|_| disk_error())?;
            }
            file.sync_all().map_err(|_| disk_error())?;
            fs::rename(&temporary,self.dir.join("services.json")).map_err(|_| disk_error())?;
            File::open(&self.dir).and_then(|dir| dir.sync_all()).map_err(|_| disk_error())?;
            Ok(())
        })();
        if result.is_err() { let _ = fs::remove_file(&temporary); }
        result
    }
}
fn protect_file(file: &File) -> Result<(),SearchError> {
    let metadata = file.metadata().map_err(|_| disk_error())?;
    if !metadata.is_file() { return Err(disk_error()); }
    #[cfg(unix)] {
        use std::os::unix::fs::{MetadataExt,PermissionsExt};
        if metadata.nlink() != 1 || metadata.uid() != unsafe { libc::geteuid() } { return Err(disk_error()); }
        file.set_permissions(fs::Permissions::from_mode(0o600)).map_err(|_| disk_error())?;
    }
    Ok(())
}

struct LimitedWriter<W> { inner: W, remaining: usize }
impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self,bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.remaining { return Err(std::io::Error::other("private snapshot size limit")); }
        let count = self.inner.write(bytes)?;
        self.remaining -= count;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> { self.inner.flush() }
}

#[cfg(test)]
#[path = "vault_tests.rs"]
mod tests;
