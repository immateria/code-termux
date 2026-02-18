mod config_requirements;
mod macos;

use crate::config::CONFIG_TOML_FILE;
use config_requirements::ConfigRequirements;
use config_requirements::ConfigRequirementsToml;
use config_requirements::LegacyManagedConfigToml;
use code_app_server_protocol::ConfigLayerMetadata;
use code_app_server_protocol::ConfigLayerSource;
use code_utils_absolute_path::AbsolutePathBuf;
use macos::load_managed_admin_config_layer;
use sha1::Digest;
use sha1::Sha1;
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use tokio::fs;
use tokio::runtime::{Builder as RuntimeBuilder, Handle};
use toml::Value as TomlValue;

#[cfg(unix)]
const CODE_MANAGED_CONFIG_SYSTEM_PATH: &str = "/etc/code/managed_config.toml";

#[cfg(unix)]
const CODE_REQUIREMENTS_SYSTEM_PATH: &str = "/etc/code/requirements.toml";

#[cfg(unix)]
const CODE_SYSTEM_CONFIG_SYSTEM_PATH: &str = "/etc/code/config.toml";

#[derive(Debug, Default, Clone)]
pub struct LoaderOverrides {
    /// Optional override for the system config file path. When unset, defaults
    /// to `/etc/code/config.toml` on Unix.
    pub system_config_path: Option<PathBuf>,
    pub managed_config_path: Option<PathBuf>,
    pub requirements_path: Option<PathBuf>,
    #[cfg(target_os = "macos")]
    pub managed_preferences_base64: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConfigLayerEntry {
    pub name: ConfigLayerSource,
    pub version: String,
    pub config: TomlValue,
    pub disabled_reason: Option<String>,
}

impl ConfigLayerEntry {
    pub fn new(name: ConfigLayerSource, config: TomlValue) -> Self {
        let version = version_for_toml(&config);
        Self {
            name,
            version,
            config,
            disabled_reason: None,
        }
    }

    pub fn new_disabled(
        name: ConfigLayerSource,
        config: TomlValue,
        disabled_reason: impl Into<String>,
    ) -> Self {
        let version = version_for_toml(&config);
        Self {
            name,
            version,
            config,
            disabled_reason: Some(disabled_reason.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigLayerStack {
    layers_low_to_high: Vec<ConfigLayerEntry>,
}

impl ConfigLayerStack {
    pub fn new(layers_low_to_high: Vec<ConfigLayerEntry>) -> Self {
        Self { layers_low_to_high }
    }

    pub fn layers_high_to_low(&self) -> impl Iterator<Item = &ConfigLayerEntry> {
        self.layers_low_to_high.iter().rev()
    }

    pub fn effective_config(&self) -> TomlValue {
        let mut merged = default_empty_table();
        for layer in &self.layers_low_to_high {
            if layer.disabled_reason.is_some() {
                continue;
            }
            merge_toml_values(&mut merged, &layer.config);
        }
        merged
    }

    pub fn origins(&self) -> HashMap<String, ConfigLayerMetadata> {
        let mut origins = HashMap::<String, ConfigLayerMetadata>::new();
        for layer in &self.layers_low_to_high {
            if layer.disabled_reason.is_some() {
                continue;
            }
            update_origins_for_value(
                &mut origins,
                &layer.name,
                &layer.version,
                "",
                &layer.config,
            );
        }
        origins
    }
}

fn update_origins_for_value(
    origins: &mut HashMap<String, ConfigLayerMetadata>,
    name: &ConfigLayerSource,
    version: &str,
    prefix: &str,
    value: &TomlValue,
) {
    match value {
        TomlValue::Table(table) => {
            if !prefix.is_empty() {
                // A table at `prefix` replaces any scalar/array previously set at that path.
                origins.remove(prefix);
            }
            for (key, child) in table {
                let next = if prefix.is_empty() {
                    key.to_string()
                } else {
                    format!("{prefix}.{key}")
                };
                update_origins_for_value(origins, name, version, &next, child);
            }
        }
        _ => {
            if prefix.is_empty() {
                // A non-table at the root replaces the entire config.
                origins.clear();
            } else {
                // A scalar/array at `prefix` replaces the entire subtree.
                remove_origin_descendants(origins, prefix);
            }
            if !prefix.is_empty() {
                origins.insert(
                    prefix.to_string(),
                    ConfigLayerMetadata {
                        name: name.clone(),
                        version: version.to_string(),
                    },
                );
            }
        }
    }
}

fn remove_origin_descendants(origins: &mut HashMap<String, ConfigLayerMetadata>, prefix: &str) {
    let subtree_prefix = format!("{prefix}.");
    origins.retain(|key, _| !key.starts_with(&subtree_prefix));
}

// Configuration layering pipeline (top overrides bottom):
//
//        +-----------------------------+
//        | Managed preferences (*)     |
//        +-----------------------------+
//                       ^
//                       |
//        +-----------------------------+
//        | legacy managed_config.toml  |
//        +-----------------------------+
//                       ^
//                       |
//        +-----------------------------+
//        |   Session flags (-c)        |
//        +-----------------------------+
//                       ^
//                       |
//        +-----------------------------+
//        | project .code/config.toml   |
//        +-----------------------------+
//                       ^
//                       |
//        +-----------------------------+
//        | user CODE_HOME/config.toml  |
//        +-----------------------------+
//                       ^
//                       |
//        +-----------------------------+
//        | system /etc/code/config.toml|
//        +-----------------------------+
//
// (*) Only available on macOS via managed device profiles.

pub async fn load_config_as_toml(code_home: &Path) -> io::Result<TomlValue> {
    load_config_as_toml_with_overrides(code_home, LoaderOverrides::default()).await
}

fn default_empty_table() -> TomlValue {
    TomlValue::Table(Default::default())
}

pub async fn load_config_layers_state(
    code_home: &Path,
    cli_overrides: &[(String, TomlValue)],
    overrides: LoaderOverrides,
) -> io::Result<ConfigLayerStack> {
    load_config_layers_state_with_cwd(code_home, None, cli_overrides, overrides).await
}

pub async fn load_config_layers_state_with_cwd(
    code_home: &Path,
    cwd: Option<&Path>,
    cli_overrides: &[(String, TomlValue)],
    overrides: LoaderOverrides,
) -> io::Result<ConfigLayerStack> {
    #[cfg(target_os = "macos")]
    let LoaderOverrides {
        system_config_path,
        managed_config_path,
        requirements_path: _,
        managed_preferences_base64,
    } = overrides;

    #[cfg(not(target_os = "macos"))]
    let LoaderOverrides {
        system_config_path,
        managed_config_path,
        requirements_path: _,
    } = overrides;

    let system_config_path = system_config_path.unwrap_or_else(|| system_config_default_path(code_home));
    let managed_config_path =
        managed_config_path.unwrap_or_else(|| managed_config_default_path(code_home));

    let system_config = read_config_from_path(&system_config_path, false)
        .await?
        .unwrap_or_else(default_empty_table);

    let user_config_path = code_home.join(CONFIG_TOML_FILE);
    let user_config = read_config_from_path(&user_config_path, true)
        .await?
        .unwrap_or_else(default_empty_table);

    let session_flags_layer = if cli_overrides.is_empty() {
        None
    } else {
        Some(build_cli_overrides_layer(cli_overrides))
    };

    let managed_config = read_config_from_path(&managed_config_path, false).await?;

    #[cfg(target_os = "macos")]
    let managed_preferences =
        load_managed_admin_config_layer(managed_preferences_base64.as_deref()).await?;

    #[cfg(not(target_os = "macos"))]
    let managed_preferences = load_managed_admin_config_layer(None).await?;

    let mut layers = Vec::<ConfigLayerEntry>::new();

    layers.push(ConfigLayerEntry::new(
        ConfigLayerSource::System {
            file: AbsolutePathBuf::from_absolute_path(&system_config_path)?,
        },
        system_config,
    ));

    layers.push(ConfigLayerEntry::new(
        ConfigLayerSource::User {
            file: AbsolutePathBuf::from_absolute_path(&user_config_path)?,
        },
        user_config,
    ));

    if let Some(cwd) = cwd {
        let resolved_cwd = resolve_cwd_for_config_layers(cwd)?;

        let trusted = {
            let mut merged_so_far = default_empty_table();
            // Only use system + user layers to determine trust.
            merge_toml_values(&mut merged_so_far, &layers[0].config);
            merge_toml_values(&mut merged_so_far, &layers[1].config);
            let cfg: crate::config::ConfigToml =
                merged_so_far.try_into().map_err(|err: toml::de::Error| {
                    io::Error::new(io::ErrorKind::InvalidData, err)
                })?;
            cfg.is_cwd_trusted(&resolved_cwd)
        };

        let mut project_layers =
            load_project_layers(&resolved_cwd, code_home, trusted).await?;
        layers.append(&mut project_layers);
    }

    if let Some(session_flags_layer) = session_flags_layer {
        layers.push(ConfigLayerEntry::new(
            ConfigLayerSource::SessionFlags,
            session_flags_layer,
        ));
    }

    if let Some(managed_config) = managed_config {
        layers.push(ConfigLayerEntry::new(
            ConfigLayerSource::LegacyManagedConfigTomlFromFile {
                file: AbsolutePathBuf::from_absolute_path(&managed_config_path)?,
            },
            managed_config,
        ));
    }

    if let Some(managed_preferences) = managed_preferences {
        layers.push(ConfigLayerEntry::new(
            ConfigLayerSource::LegacyManagedConfigTomlFromMdm,
            managed_preferences,
        ));
    }

    Ok(ConfigLayerStack::new(layers))
}

pub fn load_config_layers_state_blocking(
    code_home: &Path,
    cli_overrides: &[(String, TomlValue)],
    overrides: LoaderOverrides,
) -> io::Result<ConfigLayerStack> {
    load_config_layers_state_blocking_with_cwd(code_home, None, cli_overrides, overrides)
}

pub fn load_config_layers_state_blocking_with_cwd(
    code_home: &Path,
    cwd: Option<&Path>,
    cli_overrides: &[(String, TomlValue)],
    overrides: LoaderOverrides,
) -> io::Result<ConfigLayerStack> {
    let code_home = code_home.to_path_buf();
    let cwd: Option<PathBuf> = cwd.map(resolve_cwd_for_config_layers).transpose()?;
    let cli_overrides: Vec<(String, TomlValue)> = cli_overrides
        .iter()
        .map(|(path, value)| (path.clone(), value.clone()))
        .collect();

    block_on_loader(async move {
        load_config_layers_state_with_cwd(
            &code_home,
            cwd.as_deref(),
            &cli_overrides,
            overrides,
        )
        .await
    })
}

pub(crate) fn load_config_as_toml_blocking(
    code_home: &Path,
    overrides: LoaderOverrides,
) -> io::Result<TomlValue> {
    let code_home = code_home.to_path_buf();
    block_on_loader(async move { load_config_as_toml_with_overrides(&code_home, overrides).await })
}

pub(crate) fn load_config_requirements_blocking(
    code_home: &Path,
    overrides: LoaderOverrides,
) -> io::Result<ConfigRequirements> {
    let code_home = code_home.to_path_buf();
    block_on_loader(async move { load_config_requirements_internal(&code_home, overrides).await })
}

async fn load_config_as_toml_with_overrides(
    code_home: &Path,
    overrides: LoaderOverrides,
) -> io::Result<TomlValue> {
    let stack = load_config_layers_state_with_cwd(code_home, None, &[], overrides).await?;
    Ok(stack.effective_config())
}

async fn load_config_requirements_internal(
    code_home: &Path,
    overrides: LoaderOverrides,
) -> io::Result<ConfigRequirements> {
    #[cfg(target_os = "macos")]
    let LoaderOverrides {
        system_config_path: _,
        managed_config_path,
        requirements_path,
        managed_preferences_base64,
    } = overrides;

    #[cfg(not(target_os = "macos"))]
    let LoaderOverrides {
        system_config_path: _,
        managed_config_path,
        requirements_path,
    } = overrides;

    let managed_config_path =
        managed_config_path.unwrap_or_else(|| managed_config_default_path(code_home));
    let requirements_path = requirements_path.unwrap_or_else(|| requirements_default_path(code_home));

    let mut requirements = if let Some(value) = read_config_from_path(&requirements_path, false).await? {
        let parsed: ConfigRequirementsToml =
            value.try_into().map_err(|err: toml::de::Error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Failed to parse config requirements TOML: {err}"),
                )
            })?;
        ConfigRequirements::try_from(parsed)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?
    } else {
        ConfigRequirements::default()
    };

    let managed_config = read_config_from_path(&managed_config_path, false).await?;

    #[cfg(target_os = "macos")]
    let managed_preferences =
        load_managed_admin_config_layer(managed_preferences_base64.as_deref()).await?;

    #[cfg(not(target_os = "macos"))]
    let managed_preferences = None;

    // If multiple legacy layers specify approval_policy (e.g. both a managed_config
    // file and macOS managed preferences), allow the later/higher-precedence layer
    // to override earlier ones.
    let mut legacy_approval_policy = None;

    for legacy in [managed_config, managed_preferences].into_iter().flatten() {
        let legacy: LegacyManagedConfigToml = legacy.try_into().map_err(|err: toml::de::Error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to parse legacy managed_config TOML: {err}"),
            )
        })?;

        if let Some(approval_policy) = legacy.approval_policy {
            legacy_approval_policy = Some(approval_policy);
        }
    }

    if let Some(approval_policy) = legacy_approval_policy {
        requirements.approval_policy.can_set(&approval_policy)?;
        requirements.approval_policy = crate::config::Constrained::allow_only(approval_policy);
    }

    Ok(requirements)
}

async fn read_config_from_path(
    path: &Path,
    log_missing_as_info: bool,
) -> io::Result<Option<TomlValue>> {
    match fs::read_to_string(path).await {
        Ok(contents) => match toml::from_str::<TomlValue>(&contents) {
            Ok(value) => Ok(Some(value)),
            Err(err) => {
                tracing::error!("Failed to parse {}: {err}", path.display());
                Err(io::Error::new(io::ErrorKind::InvalidData, err))
            }
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            if log_missing_as_info {
                tracing::info!("{} not found, using defaults", path.display());
            } else {
                tracing::debug!("{} not found", path.display());
            }
            Ok(None)
        }
        Err(err) => {
            tracing::error!("Failed to read {}: {err}", path.display());
            Err(err)
        }
    }
}

/// Merge config `overlay` into `base`, giving `overlay` precedence.
pub fn merge_toml_values(base: &mut TomlValue, overlay: &TomlValue) {
    if let TomlValue::Table(overlay_table) = overlay
        && let TomlValue::Table(base_table) = base
    {
        for (key, value) in overlay_table {
            if let Some(existing) = base_table.get_mut(key) {
                merge_toml_values(existing, value);
            } else {
                base_table.insert(key.clone(), value.clone());
            }
        }
    } else {
        *base = overlay.clone();
    }
}

fn build_cli_overrides_layer(cli_overrides: &[(String, TomlValue)]) -> TomlValue {
    let mut layer = default_empty_table();
    for (path, value) in cli_overrides {
        if path.is_empty() || path.split('.').any(str::is_empty) {
            tracing::warn!("Ignoring invalid CLI override path `{path}`");
            continue;
        }
        apply_toml_override(&mut layer, path, value.clone());
    }
    layer
}

fn apply_toml_override(root: &mut TomlValue, path: &str, value: TomlValue) {
    use toml::value::Table;

    let segments: Vec<&str> = path.split('.').collect();
    let mut current = root;

    for (idx, segment) in segments.iter().enumerate() {
        let is_last = idx == segments.len() - 1;

        if is_last {
            match current {
                TomlValue::Table(table) => {
                    table.insert((*segment).to_string(), value);
                }
                _ => {
                    let mut table = Table::new();
                    table.insert((*segment).to_string(), value);
                    *current = TomlValue::Table(table);
                }
            }
            return;
        }

        match current {
            TomlValue::Table(table) => {
                current = table
                    .entry((*segment).to_string())
                    .or_insert_with(|| TomlValue::Table(Table::new()));
            }
            _ => {
                *current = TomlValue::Table(Table::new());
                if let TomlValue::Table(table) = current {
                    current = table
                        .entry((*segment).to_string())
                        .or_insert_with(|| TomlValue::Table(Table::new()));
                }
            }
        }
    }
}

pub fn version_for_toml(value: &TomlValue) -> String {
    let mut hasher = Sha1::new();
    hash_toml_value(&mut hasher, value);
    format!("{:x}", hasher.finalize())
}

fn hash_toml_value(hasher: &mut Sha1, value: &TomlValue) {
    match value {
        TomlValue::String(s) => {
            hasher.update(b"s");
            hasher.update(u64::try_from(s.len()).unwrap_or(0).to_le_bytes());
            hasher.update(s.as_bytes());
        }
        TomlValue::Integer(i) => {
            hasher.update(b"i");
            hasher.update(i.to_le_bytes());
        }
        TomlValue::Float(f) => {
            hasher.update(b"f");
            hasher.update(f.to_bits().to_le_bytes());
        }
        TomlValue::Boolean(b) => {
            hasher.update(b"b");
            hasher.update([u8::from(*b)]);
        }
        TomlValue::Datetime(dt) => {
            hasher.update(b"d");
            let rendered = dt.to_string();
            hasher.update(u64::try_from(rendered.len()).unwrap_or(0).to_le_bytes());
            hasher.update(rendered.as_bytes());
        }
        TomlValue::Array(arr) => {
            hasher.update(b"a");
            hasher.update(u64::try_from(arr.len()).unwrap_or(0).to_le_bytes());
            for item in arr {
                hash_toml_value(hasher, item);
            }
        }
        TomlValue::Table(table) => {
            hasher.update(b"t");
            hasher.update(u64::try_from(table.len()).unwrap_or(0).to_le_bytes());
            let mut keys: Vec<&String> = table.keys().collect();
            keys.sort_unstable();
            for key in keys {
                hasher.update(u64::try_from(key.len()).unwrap_or(0).to_le_bytes());
                hasher.update(key.as_bytes());
                if let Some(v) = table.get(key) {
                    hash_toml_value(hasher, v);
                }
            }
        }
    }
}

fn managed_config_default_path(code_home: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        let _ = code_home;
        PathBuf::from(CODE_MANAGED_CONFIG_SYSTEM_PATH)
    }

    #[cfg(not(unix))]
    {
        code_home.join("managed_config.toml")
    }
}

fn requirements_default_path(code_home: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        let _ = code_home;
        PathBuf::from(CODE_REQUIREMENTS_SYSTEM_PATH)
    }

    #[cfg(not(unix))]
    {
        code_home.join("requirements.toml")
    }
}

fn system_config_default_path(code_home: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        let _ = code_home;
        PathBuf::from(CODE_SYSTEM_CONFIG_SYSTEM_PATH)
    }

    #[cfg(not(unix))]
    {
        code_home.join("system_config.toml")
    }
}

fn resolve_cwd_for_config_layers(cwd: &Path) -> io::Result<PathBuf> {
    let absolute = if cwd.is_absolute() {
        cwd.to_path_buf()
    } else {
        std::env::current_dir()?.join(cwd)
    };

    let base = match std::fs::metadata(&absolute) {
        Ok(meta) if meta.is_dir() => absolute,
        Ok(_) => absolute.parent().map(Path::to_path_buf).unwrap_or(absolute),
        Err(_) => absolute,
    };

    Ok(std::fs::canonicalize(&base).unwrap_or(base))
}

async fn load_project_layers(
    cwd: &Path,
    code_home: &Path,
    trusted: bool,
) -> io::Result<Vec<ConfigLayerEntry>> {
    let project_root = crate::git_info::resolve_root_git_project_for_trust(cwd)
        .unwrap_or_else(|| cwd.to_path_buf());

    let mut dirs = Vec::<PathBuf>::new();
    for ancestor in cwd.ancestors() {
        dirs.push(ancestor.to_path_buf());
        if ancestor == project_root {
            break;
        }
    }
    dirs.reverse();

    let code_home_normalized = std::fs::canonicalize(code_home).unwrap_or_else(|_| code_home.to_path_buf());

    let mut layers = Vec::<ConfigLayerEntry>::new();
    for dir in dirs {
        let dot_code = dir.join(".code");
        if !tokio::fs::metadata(&dot_code)
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
        {
            continue;
        }

        let dot_code_normalized =
            std::fs::canonicalize(&dot_code).unwrap_or_else(|_| dot_code.clone());
        if dot_code_normalized == code_home_normalized {
            continue;
        }

        let config_file = dot_code.join(CONFIG_TOML_FILE);
        let layer_source = ConfigLayerSource::Project {
            dot_codex_folder: AbsolutePathBuf::from_absolute_path(&dot_code)?,
        };

        match tokio::fs::read_to_string(&config_file).await {
            Ok(contents) => match toml::from_str::<TomlValue>(&contents) {
                Ok(config) => {
                    if trusted {
                        layers.push(ConfigLayerEntry::new(layer_source, config));
                    } else {
                        layers.push(ConfigLayerEntry::new_disabled(
                            layer_source,
                            config,
                            "Project directory is not trusted; ignoring project config layer.",
                        ));
                    }
                }
                Err(err) => {
                    if trusted {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "Failed to parse project config file {}: {err}",
                                config_file.display()
                            ),
                        ));
                    }
                    layers.push(ConfigLayerEntry::new_disabled(
                        layer_source,
                        default_empty_table(),
                        format!(
                            "Project directory is not trusted and project config could not be parsed (ignored): {err}"
                        ),
                    ));
                }
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                // Record an empty layer entry when the folder exists, even if config.toml is missing.
                if trusted {
                    layers.push(ConfigLayerEntry::new(layer_source, default_empty_table()));
                } else {
                    layers.push(ConfigLayerEntry::new_disabled(
                        layer_source,
                        default_empty_table(),
                        "Project directory is not trusted; ignoring project config layer.",
                    ));
                }
            }
            Err(err) => {
                if trusted {
                    return Err(io::Error::new(
                        err.kind(),
                        format!(
                            "Failed to read project config file {}: {err}",
                            config_file.display()
                        ),
                    ));
                }
                layers.push(ConfigLayerEntry::new_disabled(
                    layer_source,
                    default_empty_table(),
                    format!(
                        "Project directory is not trusted and project config could not be read (ignored): {err}"
                    ),
                ));
            }
        }
    }

    Ok(layers)
}

fn block_on_loader<F, T>(future: F) -> io::Result<T>
where
    F: std::future::Future<Output = io::Result<T>> + Send + 'static,
    T: Send + 'static,
{
    if Handle::try_current().is_ok() {
        std::thread::Builder::new()
            .name("config-loader".to_string())
            .spawn(move || run_future(future))
            .map_err(|err| io::Error::other(format!("config loader thread spawn failed: {err}")))?
            .join()
            .map_err(|_| io::Error::other("config loader thread panicked"))?
    } else {
        run_future(future)
    }
}

fn run_future<F, T>(future: F) -> io::Result<T>
where
    F: std::future::Future<Output = io::Result<T>>,
    T: Send + 'static,
{
    let runtime = RuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .map_err(io::Error::other)?;
    runtime.block_on(future)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn merges_managed_config_layer_on_top() {
        let tmp = tempdir().expect("tempdir");
        let system_path = tmp.path().join("system_config.toml");
        let managed_path = tmp.path().join("managed_config.toml");

        std::fs::write(
            tmp.path().join(CONFIG_TOML_FILE),
            r#"foo = 1

[nested]
value = "base"
"#,
        )
        .expect("write base");
        std::fs::write(
            &managed_path,
            r#"foo = 2

[nested]
value = "managed_config"
extra = true
"#,
        )
        .expect("write managed config");

        let overrides = LoaderOverrides {
            system_config_path: Some(system_path),
            managed_config_path: Some(managed_path),
            requirements_path: None,
            #[cfg(target_os = "macos")]
            managed_preferences_base64: None,
        };

        let loaded = load_config_as_toml_with_overrides(tmp.path(), overrides)
            .await
            .expect("load config");
        let table = loaded.as_table().expect("top-level table expected");

        assert_eq!(table.get("foo"), Some(&TomlValue::Integer(2)));
        let nested = table
            .get("nested")
            .and_then(|v| v.as_table())
            .expect("nested");
        assert_eq!(
            nested.get("value"),
            Some(&TomlValue::String("managed_config".to_string()))
        );
        assert_eq!(nested.get("extra"), Some(&TomlValue::Boolean(true)));
    }

    #[tokio::test]
    async fn returns_empty_when_all_layers_missing() {
        let tmp = tempdir().expect("tempdir");
        let system_path = tmp.path().join("system_config.toml");
        let managed_path = tmp.path().join("managed_config.toml");
        let overrides = LoaderOverrides {
            system_config_path: Some(system_path),
            managed_config_path: Some(managed_path),
            requirements_path: None,
            #[cfg(target_os = "macos")]
            managed_preferences_base64: None,
        };

        let layers = load_config_layers_state_with_cwd(tmp.path(), None, &[], overrides)
            .await
            .expect("load layers");
        let loaded = layers.effective_config();
        let table = loaded.as_table().expect("top-level table expected");
        assert!(table.is_empty(), "expected empty table when configs missing");
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn managed_preferences_take_highest_precedence() {
        use base64::Engine;

        let managed_payload = r#"
[nested]
value = "managed"
flag = false
"#;
        let encoded = base64::prelude::BASE64_STANDARD.encode(managed_payload.as_bytes());
        let tmp = tempdir().expect("tempdir");
        let system_path = tmp.path().join("system_config.toml");
        let managed_path = tmp.path().join("managed_config.toml");

        std::fs::write(
            tmp.path().join(CONFIG_TOML_FILE),
            r#"[nested]
value = "base"
"#,
        )
        .expect("write base");
        std::fs::write(
            &managed_path,
            r#"[nested]
value = "managed_config"
flag = true
"#,
        )
        .expect("write managed config");

        let overrides = LoaderOverrides {
            system_config_path: Some(system_path),
            managed_config_path: Some(managed_path),
            requirements_path: None,
            managed_preferences_base64: Some(encoded),
        };

        let loaded = load_config_as_toml_with_overrides(tmp.path(), overrides)
            .await
            .expect("load config");
        let nested = loaded
            .get("nested")
            .and_then(|v| v.as_table())
            .expect("nested table");
        assert_eq!(
            nested.get("value"),
            Some(&TomlValue::String("managed".to_string()))
        );
        assert_eq!(nested.get("flag"), Some(&TomlValue::Boolean(false)));
    }
}
