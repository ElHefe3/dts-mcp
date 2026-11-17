//! Host-owned DTS plugin package discovery, permission checks and contributions.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    error::{AppError, Result},
    model::Document,
    store::Store,
};

pub const PLUGIN_API_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    pub contributions: Vec<Contribution>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    pub entrypoint: PluginEntrypoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contribution {
    pub id: String,
    pub label: String,
    pub kind: ContributionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionKind {
    DocumentAction,
    Importer,
    Exporter,
    Editor,
    TemplatePack,
    ValidationRule,
    RenderHook,
    /// A stateful plugin operation exposed through the DTS browser and MCP APIs.
    HttpOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ReadDocument,
    WriteDocument,
}

/// A plugin is a local package. `command` is intentionally relative to that
/// package; it runs without DTS filesystem paths or credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "runtime", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginEntrypoint {
    Builtin {
        handler: String,
    },
    Command {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// A DTS-managed local HTTP process. DTS allocates its loopback address and
    /// short-lived token, and no client receives either value.
    Http {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default = "default_health_path")]
        health_path: String,
    },
}

fn default_health_path() -> String {
    "/health".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledPlugin {
    #[serde(flatten)]
    pub manifest: PluginManifest,
    pub enabled: bool,
    pub status: String,
    #[serde(skip_serializing)]
    package_dir: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PluginState {
    #[serde(default)]
    enabled: BTreeMap<String, bool>,
}

/// Runtime registry. A corrupt package is reported in the manager instead of
/// preventing the document service from starting.
pub struct PluginManager {
    root: PathBuf,
    runtimes: Mutex<BTreeMap<String, HttpRuntime>>,
}

struct HttpRuntime {
    child: Child,
    address: SocketAddr,
    token: String,
}

impl PluginManager {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(AppError::storage)?;
        let manager = Self {
            root,
            runtimes: Mutex::new(BTreeMap::new()),
        };
        manager.install_bundled()?;
        Ok(manager)
    }

    pub fn list(&self) -> Result<Vec<InstalledPlugin>> {
        let state = self.read_state()?;
        let mut plugins = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(AppError::storage)? {
            let path = entry.map_err(AppError::storage)?.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("plugin.json");
            if !manifest_path.is_file() {
                continue;
            }
            match fs::read_to_string(&manifest_path)
                .map_err(AppError::storage)
                .and_then(|text| PluginManifest::parse(&text))
            {
                Ok(manifest) => plugins.push(InstalledPlugin {
                    enabled: state.enabled.get(&manifest.id).copied().unwrap_or(matches!(
                        manifest.entrypoint,
                        PluginEntrypoint::Builtin { .. }
                    )),
                    manifest,
                    status: "ready".into(),
                    package_dir: path,
                }),
                Err(error) => plugins.push(InstalledPlugin {
                    manifest: invalid_manifest(&path),
                    enabled: false,
                    status: format!("invalid: {error}"),
                    package_dir: path,
                }),
            }
        }
        plugins.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        let duplicates: HashSet<String> = plugins
            .iter()
            .filter(|candidate| {
                plugins
                    .iter()
                    .filter(|other| other.manifest.id == candidate.manifest.id)
                    .count()
                    > 1
            })
            .map(|candidate| candidate.manifest.id.clone())
            .collect();
        for plugin in &mut plugins {
            if duplicates.contains(&plugin.manifest.id) {
                plugin.enabled = false;
                plugin.status = "invalid: duplicate plugin id".into();
            }
        }
        Ok(plugins)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let plugins = self.list()?;
        let plugin = plugins
            .iter()
            .find(|plugin| plugin.manifest.id == id)
            .ok_or_else(|| AppError::NotFound(format!("plugin '{id}' was not found")))?;
        if plugin.status != "ready" {
            return Err(AppError::Invalid(format!(
                "plugin '{id}' cannot be enabled: {}",
                plugin.status
            )));
        }
        if enabled {
            if matches!(plugin.manifest.entrypoint, PluginEntrypoint::Http { .. }) {
                self.ensure_http_runtime(plugin)?;
            }
        } else {
            self.stop_http_runtime(id);
        }
        let mut state = self.read_state()?;
        state.enabled.insert(id.into(), enabled);
        self.write_state(&state)
    }

    pub fn contributions(&self) -> Result<Vec<PluginContribution>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|p| p.enabled && p.status == "ready")
            .flat_map(|p| {
                let plugin_id = p.manifest.id;
                let plugin_name = p.manifest.name;
                p.manifest
                    .contributions
                    .into_iter()
                    .map(move |contribution| PluginContribution {
                        plugin_id: plugin_id.clone(),
                        plugin_name: plugin_name.clone(),
                        contribution,
                    })
            })
            .collect())
    }

    pub fn invoke_document_action(
        &self,
        store: &Store,
        plugin_id: &str,
        contribution_id: &str,
        document_id: &str,
    ) -> Result<Document> {
        let plugin = self.ready_plugin(plugin_id)?;
        let contribution = plugin
            .manifest
            .contributions
            .iter()
            .find(|c| c.id == contribution_id && c.kind == ContributionKind::DocumentAction)
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "document action '{contribution_id}' was not found in plugin '{plugin_id}'"
                ))
            })?;
        let _ = contribution;
        require_permission(&plugin.manifest, Permission::ReadDocument)?;
        require_permission(&plugin.manifest, Permission::WriteDocument)?;
        let document = store.get_document(document_id)?;
        let mut result = match &plugin.manifest.entrypoint {
            PluginEntrypoint::Builtin { handler } if handler == "duplicate_draft" => {
                duplicate_document(&document)
            }
            PluginEntrypoint::Command { command, args } => self
                .run_command(
                    &plugin,
                    "document_action",
                    json!({"document": document}),
                    command,
                    args,
                )?
                .document
                .ok_or_else(|| {
                    AppError::Invalid("plugin action response must contain a document".into())
                })?,
            PluginEntrypoint::Builtin { handler } => {
                return Err(AppError::Invalid(format!(
                    "unknown built-in plugin handler '{handler}'"
                )))
            }
            PluginEntrypoint::Http { .. } => {
                return Err(AppError::Invalid(
                    "HTTP plugins cannot provide document actions".into(),
                ))
            }
        };
        // The host chooses the new ID and keeps the source template. A plugin cannot overwrite a draft it read.
        result.id = available_copy_id(store, document_id)?;
        result.template_id = document.template_id;
        result.schema_version = 1;
        store.create_document(result.clone(), Some(&result.data))
    }

    pub fn import_text(
        &self,
        store: &Store,
        plugin_id: &str,
        contribution_id: &str,
        request: PluginImport,
    ) -> Result<Document> {
        let plugin = self.ready_plugin(plugin_id)?;
        require_contribution(&plugin, contribution_id, ContributionKind::Importer)?;
        require_permission(&plugin.manifest, Permission::WriteDocument)?;
        if request.text.len() > 256 * 1024 {
            return Err(AppError::Invalid("plugin import exceeds 256 KiB".into()));
        }
        let data = match &plugin.manifest.entrypoint {
            PluginEntrypoint::Builtin { handler } if handler == "text_io" => {
                json!({"content": request.text})
            }
            PluginEntrypoint::Command { command, args } => {
                self.run_command(
                    &plugin,
                    "import",
                    json!({"text": request.text}),
                    command,
                    args,
                )?
                .data
            }
            PluginEntrypoint::Http { .. } => {
                return Err(AppError::Invalid(
                    "HTTP plugins cannot import documents".into(),
                ))
            }
            _ => return Err(AppError::Invalid("plugin does not implement import".into())),
        };
        store.create_document(
            Document {
                schema_version: 1,
                id: request.id,
                name: request.name,
                template_id: request.template_id,
                data,
            },
            None,
        )
    }

    pub fn export_text(
        &self,
        store: &Store,
        plugin_id: &str,
        contribution_id: &str,
        document_id: &str,
    ) -> Result<PluginExport> {
        let plugin = self.ready_plugin(plugin_id)?;
        require_contribution(&plugin, contribution_id, ContributionKind::Exporter)?;
        require_permission(&plugin.manifest, Permission::ReadDocument)?;
        let document = store.get_document(document_id)?;
        match &plugin.manifest.entrypoint {
            PluginEntrypoint::Builtin { handler } if handler == "text_io" => Ok(PluginExport {
                mime_type: "text/plain".into(),
                filename: format!("{}.txt", document.id),
                content: serde_json::to_string_pretty(&document.data).map_err(AppError::storage)?,
            }),
            PluginEntrypoint::Command { command, args } => {
                let response = self.run_command(
                    &plugin,
                    "export",
                    json!({"document": document}),
                    command,
                    args,
                )?;
                Ok(PluginExport {
                    mime_type: "text/plain".into(),
                    filename: format!("{}.txt", document_id),
                    content: response.message.unwrap_or_default(),
                })
            }
            PluginEntrypoint::Http { .. } => Err(AppError::Invalid(
                "HTTP plugins cannot export documents".into(),
            )),
            _ => Err(AppError::Invalid("plugin does not implement export".into())),
        }
    }

    /// Invoke a declared HTTP operation. The host owns the loopback endpoint
    /// and bearer token; callers can only name a declared contribution.
    pub fn invoke_http_operation(
        &self,
        plugin_id: &str,
        contribution_id: &str,
        input: Value,
    ) -> Result<Value> {
        let plugin = self.ready_plugin(plugin_id)?;
        require_contribution(&plugin, contribution_id, ContributionKind::HttpOperation)?;
        let PluginEntrypoint::Http { .. } = &plugin.manifest.entrypoint else {
            return Err(AppError::Invalid(
                "plugin operation requires an HTTP plugin".into(),
            ));
        };
        if serde_json::to_vec(&input).map_err(AppError::storage)?.len() > 256 * 1024 {
            return Err(AppError::Invalid(
                "plugin operation input exceeds 256 KiB".into(),
            ));
        }
        self.ensure_http_runtime(&plugin)?;
        let runtimes = self
            .runtimes
            .lock()
            .map_err(|_| AppError::Storage("plugin runtime lock poisoned".into()))?;
        let runtime = runtimes
            .get(plugin_id)
            .ok_or_else(|| AppError::Storage("plugin runtime did not start".into()))?;
        let response = http_json(
            runtime.address,
            "POST",
            &format!("/operations/{contribution_id}"),
            &runtime.token,
            Some(&input),
        )?;
        if response.len() > 256 * 1024 {
            return Err(AppError::Invalid(
                "plugin operation response exceeds 256 KiB".into(),
            ));
        }
        serde_json::from_slice(&response)
            .map_err(|error| AppError::Invalid(format!("invalid HTTP plugin response: {error}")))
    }

    fn ready_plugin(&self, id: &str) -> Result<InstalledPlugin> {
        let plugin = self
            .list()?
            .into_iter()
            .find(|plugin| plugin.manifest.id == id)
            .ok_or_else(|| AppError::NotFound(format!("plugin '{id}' was not found")))?;
        if plugin.status != "ready" {
            return Err(AppError::Invalid(format!(
                "plugin '{id}' is unavailable: {}",
                plugin.status
            )));
        }
        if !plugin.enabled {
            return Err(AppError::Invalid(format!("plugin '{id}' is disabled")));
        }
        Ok(plugin)
    }

    fn ensure_http_runtime(&self, plugin: &InstalledPlugin) -> Result<()> {
        let PluginEntrypoint::Http {
            command,
            args,
            health_path,
        } = &plugin.manifest.entrypoint
        else {
            return Ok(());
        };
        let mut runtimes = self
            .runtimes
            .lock()
            .map_err(|_| AppError::Storage("plugin runtime lock poisoned".into()))?;
        if let Some(runtime) = runtimes.get_mut(&plugin.manifest.id) {
            if runtime
                .child
                .try_wait()
                .map_err(AppError::storage)?
                .is_none()
                && http_json(runtime.address, "GET", health_path, &runtime.token, None).is_ok()
            {
                return Ok(());
            }
            let _ = runtime.child.kill();
            let _ = runtime.child.wait();
            runtimes.remove(&plugin.manifest.id);
        }
        validate_command(command)?;
        let executable = plugin.package_dir.join(command);
        if !executable.is_file() {
            return Err(AppError::Invalid(
                "HTTP plugin command is not installed".into(),
            ));
        }
        let listener = TcpListener::bind("127.0.0.1:0").map_err(AppError::storage)?;
        let address = listener.local_addr().map_err(AppError::storage)?;
        drop(listener);
        let token = random_token(&plugin.manifest.id);
        let data_dir = plugin.package_dir.join("data");
        fs::create_dir_all(&data_dir).map_err(AppError::storage)?;
        let child = Command::new(executable)
            .args(args)
            .current_dir(&plugin.package_dir)
            .env("DTS_PLUGIN_BIND", address.to_string())
            .env("DTS_PLUGIN_TOKEN", &token)
            .env("DTS_PLUGIN_DATA_DIR", &data_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(AppError::storage)?;
        runtimes.insert(
            plugin.manifest.id.clone(),
            HttpRuntime {
                child,
                address,
                token,
            },
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let healthy = runtimes.get(&plugin.manifest.id).is_some_and(|runtime| {
                http_json(runtime.address, "GET", health_path, &runtime.token, None).is_ok()
            });
            if healthy {
                return Ok(());
            }
            let exited = runtimes
                .get_mut(&plugin.manifest.id)
                .ok_or_else(|| AppError::Storage("plugin runtime disappeared".into()))?
                .child
                .try_wait()
                .map_err(AppError::storage)?
                .is_some();
            if exited || Instant::now() >= deadline {
                if let Some(mut runtime) = runtimes.remove(&plugin.manifest.id) {
                    let _ = runtime.child.kill();
                    let _ = runtime.child.wait();
                }
                return Err(AppError::Invalid(format!(
                    "HTTP plugin '{}' did not become ready within 5 seconds",
                    plugin.manifest.id
                )));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn stop_http_runtime(&self, id: &str) {
        if let Ok(mut runtimes) = self.runtimes.lock() {
            if let Some(mut runtime) = runtimes.remove(id) {
                let _ = runtime.child.kill();
                let _ = runtime.child.wait();
            }
        }
    }

    fn run_command(
        &self,
        plugin: &InstalledPlugin,
        operation: &str,
        payload: Value,
        command: &str,
        args: &[String],
    ) -> Result<PluginCommandResponse> {
        validate_command(command)?;
        let executable = plugin.package_dir.join(command);
        if !executable.is_file() {
            return Err(AppError::Invalid("plugin command is not installed".into()));
        }
        let mut child = std::process::Command::new(executable)
            .args(args)
            .current_dir(&plugin.package_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(AppError::storage)?;
        use std::io::Write;
        child
            .stdin
            .take()
            .ok_or_else(|| AppError::Storage("plugin stdin unavailable".into()))?
            .write_all(
                json!({"operation": operation, "input": payload})
                    .to_string()
                    .as_bytes(),
            )
            .map_err(AppError::storage)?;
        let started = std::time::Instant::now();
        while child.try_wait().map_err(AppError::storage)?.is_none() {
            if started.elapsed() > std::time::Duration::from_secs(5) {
                child.kill().map_err(AppError::storage)?;
                return Err(AppError::Invalid("plugin timed out after 5 seconds".into()));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = child.wait_with_output().map_err(AppError::storage)?;
        if !output.status.success() {
            return Err(AppError::Invalid("plugin process failed".into()));
        }
        if output.stdout.len() > 256 * 1024 {
            return Err(AppError::Invalid("plugin response exceeds 256 KiB".into()));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| AppError::Invalid(format!("invalid plugin response: {error}")))
    }

    fn install_bundled(&self) -> Result<()> {
        for (id, manifest) in [
            (
                "duplicate-draft",
                include_str!("../examples/plugins/duplicate-draft/plugin.json"),
            ),
            (
                "text-io",
                include_str!("../examples/plugins/text-io/plugin.json"),
            ),
            (
                "inline-cv",
                include_str!("../examples/plugins/inline-cv/plugin.json"),
            ),
            (
                "cv-template-pack",
                include_str!("../examples/plugins/cv-template-pack/plugin.json"),
            ),
        ] {
            let dir = self.root.join(id);
            fs::create_dir_all(&dir).map_err(AppError::storage)?;
            let target = dir.join("plugin.json");
            if !target.exists() {
                fs::write(target, manifest).map_err(AppError::storage)?;
            }
        }
        Ok(())
    }

    fn read_state(&self) -> Result<PluginState> {
        let path = self.root.join("state.json");
        if !path.is_file() {
            return Ok(PluginState::default());
        }
        serde_json::from_str(&fs::read_to_string(path).map_err(AppError::storage)?)
            .map_err(|error| AppError::Storage(format!("invalid plugin state: {error}")))
    }
    fn write_state(&self, state: &PluginState) -> Result<()> {
        fs::write(
            self.root.join("state.json"),
            serde_json::to_vec_pretty(state).map_err(AppError::storage)?,
        )
        .map_err(AppError::storage)
    }
}

impl Drop for PluginManager {
    fn drop(&mut self) {
        if let Ok(runtimes) = self.runtimes.get_mut() {
            for runtime in runtimes.values_mut() {
                let _ = runtime.child.kill();
                let _ = runtime.child.wait();
            }
        }
    }
}

fn validate_command(command: &str) -> Result<()> {
    if Path::new(command)
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(AppError::Invalid(
            "plugin command must be a relative package path".into(),
        ));
    }
    Ok(())
}

fn random_token(plugin_id: &str) -> String {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).expect("operating system random source must be available");
    let encoded = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("dts-{plugin_id}-{encoded}")
}

fn http_json(
    address: SocketAddr,
    method: &str,
    path: &str,
    token: &str,
    body: Option<&Value>,
) -> Result<Vec<u8>> {
    if !path.starts_with('/') || path.contains("..") {
        return Err(AppError::Invalid("invalid plugin HTTP path".into()));
    }
    let mut stream =
        TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(AppError::storage)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(AppError::storage)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(AppError::storage)?;
    let payload = body
        .map(serde_json::to_vec)
        .transpose()
        .map_err(AppError::storage)?
        .unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(AppError::storage)?;
    stream.write_all(&payload).map_err(AppError::storage)?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(AppError::storage)?;
    let split = response
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .ok_or_else(|| AppError::Storage("invalid HTTP plugin response".into()))?;
    let head = std::str::from_utf8(&response[..split]).map_err(AppError::storage)?;
    if !head.starts_with("HTTP/1.1 2") && !head.starts_with("HTTP/1.0 2") {
        return Err(AppError::Invalid(format!(
            "HTTP plugin request failed: {}",
            head.lines().next().unwrap_or("invalid response")
        )));
    }
    Ok(response[split + 4..].to_vec())
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginContribution {
    pub plugin_id: String,
    pub plugin_name: String,
    #[serde(flatten)]
    pub contribution: Contribution,
}
#[derive(Debug, Serialize)]
pub struct PluginExport {
    pub mime_type: String,
    pub filename: String,
    pub content: String,
}
#[derive(Debug)]
pub struct PluginImport {
    pub id: String,
    pub name: String,
    pub template_id: String,
    pub text: String,
}
#[derive(Debug, Deserialize)]
struct PluginCommandResponse {
    #[serde(default)]
    document: Option<Document>,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    message: Option<String>,
}

fn invalid_manifest(path: &Path) -> PluginManifest {
    PluginManifest {
        id: format!(
            "invalid-{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("package")
        ),
        name: "Invalid plugin package".into(),
        version: "0.0.0".into(),
        api_version: PLUGIN_API_VERSION,
        contributions: vec![],
        permissions: vec![],
        entrypoint: PluginEntrypoint::Builtin {
            handler: "invalid".into(),
        },
    }
}
fn require_permission(manifest: &PluginManifest, permission: Permission) -> Result<()> {
    if manifest.permissions.contains(&permission) {
        Ok(())
    } else {
        Err(AppError::Invalid(format!(
            "plugin '{}' lacks required permission {permission:?}",
            manifest.id
        )))
    }
}
fn require_contribution(plugin: &InstalledPlugin, id: &str, kind: ContributionKind) -> Result<()> {
    if plugin
        .manifest
        .contributions
        .iter()
        .any(|contribution| contribution.id == id && contribution.kind == kind)
    {
        Ok(())
    } else {
        Err(AppError::NotFound(format!(
            "plugin contribution '{id}' was not found"
        )))
    }
}
fn duplicate_document(source: &Document) -> Document {
    let mut document = source.clone();
    document.name = format!("{} (copy)", source.name);
    document
}
fn available_copy_id(store: &Store, source_id: &str) -> Result<String> {
    for number in 1..1000 {
        let suffix = if number == 1 {
            "copy".into()
        } else {
            format!("copy-{number}")
        };
        let id = format!("{source_id}-{suffix}");
        if store.get_document(&id).is_err() {
            return Ok(id);
        }
    }
    Err(AppError::Invalid(
        "could not allocate a new document id".into(),
    ))
}

impl PluginManifest {
    pub fn parse(json: &str) -> Result<Self> {
        let manifest: Self = serde_json::from_str(json)
            .map_err(|error| AppError::Invalid(format!("invalid plugin manifest: {error}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        if self.api_version != PLUGIN_API_VERSION {
            return Err(AppError::Invalid(format!(
                "plugin api_version {} is unsupported; expected {PLUGIN_API_VERSION}",
                self.api_version
            )));
        }
        validate_id("plugin id", &self.id)?;
        if self.name.trim().is_empty() {
            return Err(AppError::Invalid("plugin name is required".into()));
        }
        let version_parts: Vec<_> = self.version.split('.').collect();
        if version_parts.len() != 3
            || !version_parts.iter().all(|part| {
                !part.is_empty()
                    && part.bytes().all(|byte| byte.is_ascii_digit())
                    && (part.len() == 1 || !part.starts_with('0'))
            })
        {
            return Err(AppError::Invalid(
                "plugin version must use major.minor.patch, for example 1.0.0".into(),
            ));
        }
        if self.contributions.is_empty() {
            return Err(AppError::Invalid(
                "plugin must declare at least one contribution".into(),
            ));
        }
        let mut contribution_ids = HashSet::new();
        for contribution in &self.contributions {
            validate_id("contribution id", &contribution.id)?;
            if contribution.label.trim().is_empty() {
                return Err(AppError::Invalid(format!(
                    "contribution '{}' needs a label",
                    contribution.id
                )));
            }
            if !contribution_ids.insert(&contribution.id) {
                return Err(AppError::Invalid(format!(
                    "duplicate contribution id '{}'",
                    contribution.id
                )));
            }
        }
        let mut permissions = HashSet::new();
        for permission in &self.permissions {
            if !permissions.insert(permission) {
                return Err(AppError::Invalid(format!(
                    "duplicate plugin permission: {permission:?}"
                )));
            }
        }
        Ok(())
    }
}

fn validate_id(label: &str, id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || id.starts_with('-')
        || id.ends_with('-')
    {
        return Err(AppError::Invalid(format!(
            "{label} must be 1–64 lowercase ASCII letters, digits or interior hyphens"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;

    const EXAMPLE: &str = include_str!("../examples/plugins/duplicate-draft/plugin.json");

    #[test]
    fn parses_the_documented_example() {
        let plugin = PluginManifest::parse(EXAMPLE).unwrap();
        assert_eq!(plugin.id, "duplicate-draft");
        assert_eq!(
            plugin.contributions[0].kind,
            ContributionKind::DocumentAction
        );
        assert_eq!(
            plugin.permissions,
            [Permission::ReadDocument, Permission::WriteDocument]
        );
    }

    #[test]
    fn rejects_invalid_or_ambiguous_manifests() {
        let example: Value = serde_json::from_str(EXAMPLE).unwrap();
        let cases = [
            ("api_version", json!(2), "unsupported"),
            ("id", json!("../draft"), "plugin id"),
            ("version", json!("latest"), "major.minor.patch"),
            ("name", json!(" "), "name is required"),
            ("contributions", json!([]), "at least one contribution"),
            (
                "contributions",
                json!([example["contributions"][0], example["contributions"][0]]),
                "duplicate contribution",
            ),
            (
                "permissions",
                json!(["read_document", "read_document"]),
                "duplicate plugin permission",
            ),
            ("permissions", json!(["network"]), "unknown variant"),
            ("entrypoint", json!("run.exe"), "invalid type"),
        ];
        for (key, value, expected) in cases {
            let mut input = example.clone();
            input[key] = value;
            let error = PluginManifest::parse(&input.to_string()).unwrap_err();
            assert!(error.to_string().contains(expected), "{key}: {error}");
        }
    }

    #[test]
    fn registry_persists_enablement_and_hides_disabled_contributions() {
        let directory = tempfile::tempdir().unwrap();
        let manager = PluginManager::open(directory.path()).unwrap();
        assert!(manager
            .contributions()
            .unwrap()
            .iter()
            .any(|item| item.plugin_id == "duplicate-draft"));

        manager.set_enabled("duplicate-draft", false).unwrap();
        let restarted = PluginManager::open(directory.path()).unwrap();
        let plugin = restarted
            .list()
            .unwrap()
            .into_iter()
            .find(|item| item.manifest.id == "duplicate-draft")
            .unwrap();
        assert!(!plugin.enabled);
        assert!(!restarted
            .contributions()
            .unwrap()
            .iter()
            .any(|item| item.plugin_id == "duplicate-draft"));
    }

    #[test]
    fn malformed_and_duplicate_packages_are_isolated() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for package in ["broken", "duplicate"] {
            std::fs::create_dir(root.join(package)).unwrap();
        }
        std::fs::write(root.join("broken/plugin.json"), "not json").unwrap();
        std::fs::write(root.join("duplicate/plugin.json"), EXAMPLE).unwrap();
        let manager = PluginManager::open(root).unwrap();
        let plugins = manager.list().unwrap();
        assert!(plugins
            .iter()
            .any(|item| item.status.starts_with("invalid:")));
        assert!(!plugins
            .iter()
            .filter(|item| item.manifest.id == "duplicate-draft")
            .any(|item| item.enabled));
    }
}
