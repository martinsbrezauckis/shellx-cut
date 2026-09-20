//! Read the Runner-owned provider runtime context without launching a provider.
//!
//! A caller that receives [`ProviderChildLaunch`] builds its own structured
//! `Command`: program = `executable`, then optional `entrypoint`, then the
//! surface's existing arguments.  It applies `environment` only after clearing
//! the provider child's inherited environment, then adds that surface's
//! deliberately preserved policy variables.  This module neither selects a
//! provider for the caller nor falls back to discovery/PATH resolution.

use serde::Deserialize;
use serde_json::{json, Value};
#[cfg(test)]
use std::cell::RefCell;
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const CONTEXT_ENV: &str = "RELEASE_RUNNER_PROVIDER_CONTEXT";
const V1_SCHEMA: &str = "release-runner.provider-runtime/v1";
const V2_SCHEMA: &str = "release-runner.provider-runtime/v2";
const MAX_CONTEXT_BYTES: u64 = 64 * 1024;
pub(crate) const PYTHON_CHILD_LAUNCHES_SCHEMA: &str = "shellx-cut/provider-child-launches/1";
const ALLOWED_ENVIRONMENT_KEYS: [&str; 9] = [
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "PATH",
    "SystemRoot",
];

/// The admitted command prefix and its curated, provider-child-only environment.
///
/// This deliberately has no method that applies the environment.  Each caller
/// remains responsible for its own `Command`, `env_clear` ordering and
/// provider-specific policy values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderChildLaunch {
    pub(crate) executable: PathBuf,
    pub(crate) entrypoint: Option<PathBuf>,
    pub(crate) environment: BTreeMap<String, String>,
}

/// Serialize an already-validated subset for a Cut-owned Python child.
///
/// This is deliberately a transport shape, not another Runner-context parser.
/// Callers choose their own logical-provider set and retain their own ladder
/// order; the Python process receives only exact prefixes and per-child
/// environments for those providers.
pub(crate) fn python_child_launches_value(
    launches: &BTreeMap<String, ProviderChildLaunch>,
    allowed: &[&str],
) -> Value {
    let mut entries = serde_json::Map::new();
    for (provider, launch) in launches {
        if !allowed.contains(&provider.as_str()) {
            continue;
        }
        entries.insert(
            provider.clone(),
            json!({
                "executable": &launch.executable,
                "entrypoint": &launch.entrypoint,
                "environment": &launch.environment,
            }),
        );
    }
    json!({
        "schema": PYTHON_CHILD_LAUNCHES_SCHEMA,
        "launches": entries,
    })
}

/// Returns the whole Runner-admitted provider set, keyed by logical provider.
///
/// `None` means Runner supplied no context at all. A set but empty, unreadable,
/// malformed, or incomplete context is an error. Multi-provider callers retain
/// their own ordering and select from this map; they must not rediscover a
/// provider from PATH.
pub(crate) fn provider_launches_from_process_environment(
) -> Result<Option<BTreeMap<String, ProviderChildLaunch>>, String> {
    #[cfg(test)]
    if let Some(context_path) = TEST_PROVIDER_CONTEXT.with(|context| context.borrow().clone()) {
        return provider_launches_from_environment_value(Some(context_path));
    }
    provider_launches_from_environment_value(env::var_os(CONTEXT_ENV))
}

/// Returns an exact selected launch. If a context exists but does not admit the
/// requested provider, callers must fail instead of using an ordinary resolver.
pub(crate) fn selected_from_process_environment(
    logical_provider: &str,
) -> Result<Option<ProviderChildLaunch>, String> {
    let Some(launches) = provider_launches_from_process_environment()? else {
        return Ok(None);
    };
    selected_from_launches(launches, logical_provider).map(Some)
}

fn provider_launches_from_environment_value(
    context_path: Option<OsString>,
) -> Result<Option<BTreeMap<String, ProviderChildLaunch>>, String> {
    let Some(context_path) = context_path else {
        return Ok(None);
    };
    if context_path.is_empty() {
        return Err("RELEASE_RUNNER_PROVIDER_CONTEXT is set but empty".into());
    }
    provider_launches_from_context_path(Path::new(&context_path)).map(Some)
}

fn provider_launches_from_context_path(
    context_path: &Path,
) -> Result<BTreeMap<String, ProviderChildLaunch>, String> {
    if !context_path.is_absolute() {
        return Err("provider runtime context path must be absolute".into());
    }

    let metadata = fs::symlink_metadata(context_path)
        .map_err(|error| format!("cannot inspect provider runtime context: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("provider runtime context must be a regular file".into());
    }
    if metadata.len() > MAX_CONTEXT_BYTES {
        return Err("provider runtime context exceeds 64 KiB".into());
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(context_path)
        .map_err(|error| format!("cannot open provider runtime context: {error}"))?
        .take(MAX_CONTEXT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read provider runtime context: {error}"))?;
    if bytes.len() as u64 > MAX_CONTEXT_BYTES {
        return Err("provider runtime context exceeds 64 KiB".into());
    }

    let document: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid provider runtime context JSON: {error}"))?;
    let schema = document
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or("provider runtime context schema is missing")?;

    match schema {
        V1_SCHEMA => {
            let context: SingleContext = serde_json::from_value(document)
                .map_err(|error| format!("invalid v1 provider runtime context: {error}"))?;
            if context.schema != V1_SCHEMA {
                return Err("provider runtime context schema is invalid".into());
            }
            let provider = context.admission.enrollment.provider.clone();
            let launch = launch_from(&context.admission, &context.effective_environment)?;
            Ok(BTreeMap::from([(provider, launch)]))
        }
        V2_SCHEMA => {
            let context: SetContext = serde_json::from_value(document)
                .map_err(|error| format!("invalid v2 provider runtime context: {error}"))?;
            if context.schema != V2_SCHEMA
                || context.providers.is_empty()
                || context.providers.len() > 16
            {
                return Err("provider runtime context requires 1..16 v2 provider entries".into());
            }

            let mut previous_id = None;
            let mut logical_providers = BTreeSet::new();
            let mut launches = BTreeMap::new();
            for entry in &context.providers {
                let launch = launch_from(&entry.admission, &entry.effective_environment)?;
                let enrollment = &entry.admission.enrollment;
                if previous_id.is_some_and(|previous| previous >= enrollment.id.as_str()) {
                    return Err(
                        "v2 provider runtime entries must be sorted by enrollment ID".into(),
                    );
                }
                if !logical_providers.insert(enrollment.provider.as_str()) {
                    return Err(
                        "v2 provider runtime entries must have unique logical providers".into(),
                    );
                }
                previous_id = Some(enrollment.id.as_str());
                launches.insert(enrollment.provider.clone(), launch);
            }
            Ok(launches)
        }
        _ => Err("unsupported provider runtime context schema".into()),
    }
}

#[cfg(test)]
fn selected_from_context_path(
    context_path: &Path,
    logical_provider: &str,
) -> Result<ProviderChildLaunch, String> {
    selected_from_launches(
        provider_launches_from_context_path(context_path)?,
        logical_provider,
    )
}

fn selected_from_launches(
    launches: BTreeMap<String, ProviderChildLaunch>,
    logical_provider: &str,
) -> Result<ProviderChildLaunch, String> {
    validate_logical_provider(logical_provider)?;
    launches
        .get(logical_provider)
        .cloned()
        .ok_or("selected provider is absent from provider runtime context".into())
}

fn launch_from(
    admission: &Admission,
    environment: &BTreeMap<String, String>,
) -> Result<ProviderChildLaunch, String> {
    validate_admission(admission)?;
    validate_effective_environment(environment, &admission.enrollment.canonical_environment)?;
    Ok(ProviderChildLaunch {
        executable: admission.enrollment.executable.path.clone(),
        entrypoint: admission
            .enrollment
            .entrypoint
            .as_ref()
            .map(|pin| pin.path.clone()),
        environment: environment.clone(),
    })
}

fn validate_admission(admission: &Admission) -> Result<(), String> {
    if admission.schema != V1_SCHEMA
        || admission.resource_lease.is_empty()
        || admission.resource_lease.len() > 256
    {
        return Err("provider admission is malformed".into());
    }
    let enrollment = &admission.enrollment;
    validate_runtime_identifier(&enrollment.id)?;
    validate_logical_provider(&enrollment.provider)?;
    if enrollment.user_identity.is_empty()
        || enrollment.user_identity.len() > 192
        || enrollment.runtime_code.len() > 8
    {
        return Err("provider enrollment is malformed".into());
    }
    validate_code_pin(&enrollment.executable, true)?;
    if let Some(entrypoint) = &enrollment.entrypoint {
        validate_code_pin(entrypoint, false)?;
        if entrypoint.path == enrollment.executable.path {
            return Err("provider entrypoint cannot duplicate the executable".into());
        }
    }
    let mut code_paths = BTreeSet::from([enrollment.executable.path.as_path()]);
    if let Some(entrypoint) = &enrollment.entrypoint {
        code_paths.insert(entrypoint.path.as_path());
    }
    for runtime_code in &enrollment.runtime_code {
        validate_code_pin(runtime_code, true)?;
        if !code_paths.insert(runtime_code.path.as_path()) {
            return Err("provider enrollment contains duplicate code paths".into());
        }
    }
    validate_canonical_environment(&enrollment.canonical_environment)
}

fn validate_runtime_identifier(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err("provider runtime ID must be a bounded lowercase identifier".into());
    }
    Ok(())
}

fn validate_logical_provider(value: &str) -> Result<(), String> {
    validate_runtime_identifier(value)
        .map_err(|_| "logical provider must be a bounded lowercase identifier".into())
}

fn validate_code_pin(pin: &CodePin, executable: bool) -> Result<(), String> {
    if !is_host_absolute_path(&pin.path) || !is_plain_text_path(&pin.path) {
        return Err("provider code path must be a native absolute path".into());
    }
    if pin.sha256.len() != 64
        || !pin
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("provider code pin requires a lowercase SHA-256".into());
    }
    let extension = pin
        .path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let file_name = pin
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if executable {
        if [
            "sh",
            "bash",
            "dash",
            "zsh",
            "fish",
            "cmd.exe",
            "powershell.exe",
            "pwsh.exe",
            "pwsh",
        ]
        .contains(&file_name.as_str())
            || ["cmd", "bat", "ps1", "sh", "js", "mjs", "py"].contains(&extension.as_str())
        {
            return Err(
                "provider executable must be native, never a shell or script wrapper".into(),
            );
        }
    } else if ["cmd", "bat", "ps1", "sh"].contains(&extension.as_str()) {
        return Err("provider entrypoint cannot be a shell or batch wrapper".into());
    }
    Ok(())
}

fn validate_canonical_environment(environment: &CanonicalEnvironment) -> Result<(), String> {
    for path in environment.paths() {
        if !is_host_absolute_path(path) || !is_plain_text_path(path) {
            return Err("provider canonical environment contains a non-native path".into());
        }
    }
    Ok(())
}

fn validate_effective_environment(
    environment: &BTreeMap<String, String>,
    canonical_environment: &CanonicalEnvironment,
) -> Result<(), String> {
    validate_effective_environment_for_platform(environment, canonical_environment, cfg!(windows))
}

fn validate_effective_environment_for_platform(
    environment: &BTreeMap<String, String>,
    canonical_environment: &CanonicalEnvironment,
    windows: bool,
) -> Result<(), String> {
    if !environment.contains_key("HOME") || !environment.contains_key("PATH") {
        return Err("provider effective environment requires HOME and PATH".into());
    }
    for (key, value) in environment {
        if !ALLOWED_ENVIRONMENT_KEYS.contains(&key.as_str()) {
            return Err("provider effective environment contains an uncurated key".into());
        }
        if value.is_empty() || !is_plain_text(value) {
            return Err("provider effective environment contains an invalid value".into());
        }
    }
    for (key, path) in canonical_environment.named_paths() {
        if environment.get(key).map(String::as_str) != Some(path) {
            return Err("provider effective environment differs from its canonical locator".into());
        }
    }
    for key in [
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
    ] {
        if canonical_environment.named_path(key).is_none() && environment.contains_key(key) {
            return Err("provider effective environment contains an undeclared locator".into());
        }
    }

    if windows {
        for key in ["USERPROFILE", "APPDATA", "LOCALAPPDATA", "SystemRoot"] {
            let value = environment
                .get(key)
                .ok_or("provider effective environment lacks a Windows locator")?;
            if !is_host_absolute_path(Path::new(value)) {
                return Err("provider effective environment contains a non-native path".into());
            }
        }
        if environment.contains_key("XDG_CONFIG_HOME")
            || environment.contains_key("XDG_DATA_HOME")
            || environment.contains_key("XDG_CACHE_HOME")
        {
            return Err("provider effective environment contains Unix locators on Windows".into());
        }
    } else {
        if environment.contains_key("USERPROFILE")
            || environment.contains_key("APPDATA")
            || environment.contains_key("LOCALAPPDATA")
            || environment.contains_key("SystemRoot")
        {
            return Err("provider effective environment contains Windows locators on Unix".into());
        }
    }

    for key in [
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "SystemRoot",
    ] {
        if let Some(value) = environment.get(key) {
            if !is_host_absolute_path(Path::new(value)) || !is_plain_text(value) {
                return Err("provider effective environment contains a non-native path".into());
            }
        }
    }
    let path = environment.get("PATH").expect("checked above");
    let components = env::split_paths(path).collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !is_host_absolute_path(component))
    {
        return Err("provider PATH must contain only native absolute directories".into());
    }
    Ok(())
}

fn is_host_absolute_path(path: &Path) -> bool {
    path.is_absolute()
}

fn is_plain_text_path(path: &Path) -> bool {
    path.to_str().is_some_and(is_plain_text)
}

fn is_plain_text(value: &str) -> bool {
    !value.contains(['\0', '\n', '\r'])
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SingleContext {
    schema: String,
    admission: Admission,
    effective_environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderEntry {
    admission: Admission,
    effective_environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetContext {
    schema: String,
    providers: Vec<ProviderEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Admission {
    schema: String,
    enrollment: Enrollment,
    resource_lease: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Enrollment {
    id: String,
    provider: String,
    user_identity: String,
    executable: CodePin,
    #[serde(default)]
    entrypoint: Option<CodePin>,
    #[serde(default)]
    runtime_code: Vec<CodePin>,
    canonical_environment: CanonicalEnvironment,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodePin {
    path: PathBuf,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CanonicalEnvironment {
    home: PathBuf,
    #[serde(default)]
    user_profile: Option<PathBuf>,
    #[serde(default)]
    app_data: Option<PathBuf>,
    #[serde(default)]
    local_app_data: Option<PathBuf>,
    #[serde(default)]
    xdg_config_home: Option<PathBuf>,
    #[serde(default)]
    xdg_data_home: Option<PathBuf>,
    #[serde(default)]
    xdg_cache_home: Option<PathBuf>,
}

impl CanonicalEnvironment {
    fn named_paths(&self) -> Vec<(&'static str, &str)> {
        let mut paths = vec![("HOME", self.home.to_str().unwrap_or_default())];
        for (key, path) in [
            ("USERPROFILE", &self.user_profile),
            ("APPDATA", &self.app_data),
            ("LOCALAPPDATA", &self.local_app_data),
            ("XDG_CONFIG_HOME", &self.xdg_config_home),
            ("XDG_DATA_HOME", &self.xdg_data_home),
            ("XDG_CACHE_HOME", &self.xdg_cache_home),
        ] {
            if let Some(path) = path {
                paths.push((key, path.to_str().unwrap_or_default()));
            }
        }
        paths
    }

    fn named_path(&self, key: &str) -> Option<&Path> {
        match key {
            "HOME" => Some(&self.home),
            "USERPROFILE" => self.user_profile.as_deref(),
            "APPDATA" => self.app_data.as_deref(),
            "LOCALAPPDATA" => self.local_app_data.as_deref(),
            "XDG_CONFIG_HOME" => self.xdg_config_home.as_deref(),
            "XDG_DATA_HOME" => self.xdg_data_home.as_deref(),
            "XDG_CACHE_HOME" => self.xdg_cache_home.as_deref(),
            _ => None,
        }
    }

    fn paths(&self) -> Vec<&Path> {
        let mut paths = vec![self.home.as_path()];
        for path in [
            self.user_profile.as_deref(),
            self.app_data.as_deref(),
            self.local_app_data.as_deref(),
            self.xdg_config_home.as_deref(),
            self.xdg_data_home.as_deref(),
            self.xdg_cache_home.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            paths.push(path);
        }
        paths
    }
}

#[cfg(test)]
pub(crate) fn test_fixture_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("server crate is beneath the Cut worktree")
        .join(".scratch/test-fixtures");
    fs::create_dir_all(&root).unwrap();
    root
}

#[cfg(test)]
thread_local! {
    static TEST_PROVIDER_CONTEXT: RefCell<Option<OsString>> = const { RefCell::new(None) };
}

/// Override Runner context only for the current source-test thread.
///
/// This prevents tests from mutating process-global environment state that
/// unrelated provider readers can observe in parallel.
#[cfg(test)]
pub(crate) struct TestProviderContext {
    previous: Option<OsString>,
    thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

#[cfg(test)]
pub(crate) fn install_test_provider_context(
    context_path: impl Into<OsString>,
) -> TestProviderContext {
    let previous = TEST_PROVIDER_CONTEXT.with(|context| context.replace(Some(context_path.into())));
    TestProviderContext {
        previous,
        thread_bound: std::marker::PhantomData,
    }
}

#[cfg(test)]
impl Drop for TestProviderContext {
    fn drop(&mut self) {
        TEST_PROVIDER_CONTEXT.with(|context| {
            context.replace(self.previous.take());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map, Value};
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn native_path(name: &str) -> String {
        test_fixture_root()
            .join(name)
            .to_string_lossy()
            .into_owned()
    }

    fn program_path() -> String {
        #[cfg(windows)]
        return native_path("provider.exe");
        #[cfg(not(windows))]
        native_path("provider")
    }

    fn entrypoint_path() -> String {
        native_path("provider-entrypoint.mjs")
    }

    fn canonical_and_effective_environment(home: &str) -> (Value, Value) {
        let mut canonical_environment = Map::new();
        canonical_environment.insert("home".into(), json!(home));
        let mut environment = Map::new();
        environment.insert("HOME".into(), json!(home));
        environment.insert("PATH".into(), json!(native_path("provider-bin")));
        #[cfg(windows)]
        {
            let app_data = native_path("app-data");
            let local_app_data = native_path("local-app-data");
            canonical_environment.insert("userProfile".into(), json!(home));
            canonical_environment.insert("appData".into(), json!(app_data));
            canonical_environment.insert("localAppData".into(), json!(local_app_data));
            environment.insert("USERPROFILE".into(), json!(home));
            environment.insert("APPDATA".into(), json!(app_data));
            environment.insert("LOCALAPPDATA".into(), json!(local_app_data));
            environment.insert("SystemRoot".into(), json!(native_path("windows")));
        }
        (
            Value::Object(canonical_environment),
            Value::Object(environment),
        )
    }

    fn fixture_user_identity() -> &'static str {
        #[cfg(windows)]
        return "S-1-5-21-1000-1000-1000-1000";
        #[cfg(not(windows))]
        "uid:1000"
    }

    fn entry(id: &str, provider: &str) -> Value {
        let home = native_path(&format!("home-{id}"));
        let (canonical_environment, effective_environment) =
            canonical_and_effective_environment(&home);
        json!({
            "admission": {
                "schema": V1_SCHEMA,
                "resourceLease": format!("provider-login:test:{provider}"),
                "enrollment": {
                    "id": id,
                    "provider": provider,
                    "userIdentity": fixture_user_identity(),
                    "executable": { "path": program_path(), "sha256": "a".repeat(64) },
                    "entrypoint": { "path": entrypoint_path(), "sha256": "b".repeat(64) },
                    "runtimeCode": [],
                    "canonicalEnvironment": canonical_environment
                }
            },
            "effectiveEnvironment": effective_environment
        })
    }

    fn v1(provider: &str) -> Value {
        let entry = entry(provider, provider);
        json!({
            "schema": V1_SCHEMA,
            "admission": entry["admission"].clone(),
            "effectiveEnvironment": entry["effectiveEnvironment"].clone()
        })
    }

    fn context_file(document: &Value) -> NamedTempFile {
        let mut file = NamedTempFile::new_in(test_fixture_root()).unwrap();
        file.write_all(&serde_json::to_vec(document).unwrap())
            .unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn unset_context_allows_the_ordinary_product_route_but_empty_is_refused() {
        assert_eq!(
            provider_launches_from_environment_value(None).unwrap(),
            None
        );
        assert!(provider_launches_from_environment_value(Some(OsString::new())).is_err());
    }

    #[test]
    fn v1_returns_the_explicit_prefix_and_only_curated_environment() {
        let document = v1("claude");
        let file = context_file(&document);
        let launches = provider_launches_from_context_path(file.path()).unwrap();
        assert_eq!(launches.keys().collect::<Vec<_>>(), vec!["claude"]);
        let launch = launches.get("claude").unwrap();

        assert_eq!(launch.executable, PathBuf::from(program_path()));
        assert_eq!(launch.entrypoint, Some(PathBuf::from(entrypoint_path())));
        assert_eq!(
            launch.environment.get("HOME").map(String::as_str),
            document["effectiveEnvironment"]["HOME"].as_str()
        );
        assert!(launch
            .environment
            .keys()
            .all(|key| ALLOWED_ENVIRONMENT_KEYS.contains(&key.as_str())));

        let mut without_entrypoint = document;
        without_entrypoint["admission"]["enrollment"]["entrypoint"] = Value::Null;
        assert_eq!(
            selected_from_context_path(context_file(&without_entrypoint).path(), "claude")
                .unwrap()
                .entrypoint,
            None
        );
    }

    #[test]
    fn v2_requires_an_exact_unique_logical_provider() {
        let document = json!({
            "schema": V2_SCHEMA,
            "providers": [entry("claude", "claude"), entry("grok", "grok")]
        });
        let file = context_file(&document);
        let launches = provider_launches_from_context_path(file.path()).unwrap();
        assert_eq!(launches.keys().collect::<Vec<_>>(), vec!["claude", "grok"]);
        let expected_claude: BTreeMap<String, String> =
            serde_json::from_value(document["providers"][0]["effectiveEnvironment"].clone())
                .unwrap();
        let expected_grok: BTreeMap<String, String> =
            serde_json::from_value(document["providers"][1]["effectiveEnvironment"].clone())
                .unwrap();
        assert_eq!(launches["claude"].environment, expected_claude);
        assert_eq!(launches["grok"].environment, expected_grok);
        assert_ne!(launches["claude"].environment, launches["grok"].environment);
        assert_eq!(
            selected_from_context_path(file.path(), "grok")
                .unwrap()
                .executable,
            PathBuf::from(program_path())
        );
        assert!(selected_from_context_path(file.path(), "codex").is_err());

        let duplicate = json!({
            "schema": V2_SCHEMA,
            "providers": [entry("claude", "claude"), entry("grok", "claude")]
        });
        assert!(selected_from_context_path(context_file(&duplicate).path(), "claude").is_err());
    }

    #[test]
    fn rejects_untrusted_environment_keys_and_shell_programs() {
        let mut injected_environment = v1("claude");
        injected_environment["effectiveEnvironment"]
            .as_object_mut()
            .unwrap()
            .insert("UNTRUSTED".into(), json!("value"));
        assert!(
            selected_from_context_path(context_file(&injected_environment).path(), "claude")
                .is_err()
        );

        let mut shell_program = v1("claude");
        #[cfg(windows)]
        let shell_path = r"C:\\Windows\\System32\\cmd.exe";
        #[cfg(unix)]
        let shell_path = "/bin/sh";
        shell_program["admission"]["enrollment"]["executable"]["path"] = json!(shell_path);
        assert!(selected_from_context_path(context_file(&shell_program).path(), "claude").is_err());
    }

    #[test]
    fn omitted_runtime_code_defaults_to_an_empty_list() {
        let mut omitted_runtime_code = v1("claude");
        omitted_runtime_code["admission"]["enrollment"]
            .as_object_mut()
            .unwrap()
            .remove("runtimeCode");
        assert!(
            selected_from_context_path(context_file(&omitted_runtime_code).path(), "claude")
                .is_ok()
        );
    }

    #[test]
    fn rejects_a_context_larger_than_the_contract_limit() {
        let mut file = NamedTempFile::new_in(test_fixture_root()).unwrap();
        file.write_all(&vec![b' '; MAX_CONTEXT_BYTES as usize + 1])
            .unwrap();
        file.flush().unwrap();
        assert!(selected_from_context_path(file.path(), "claude").is_err());
    }

    #[test]
    fn synthetic_windows_environment_requires_all_windows_locators() {
        let home = PathBuf::from(native_path("windows-home"));
        let canonical = CanonicalEnvironment {
            home: home.clone(),
            user_profile: Some(home.clone()),
            app_data: Some(PathBuf::from(native_path("windows-app-data"))),
            local_app_data: Some(PathBuf::from(native_path("windows-local-app-data"))),
            xdg_config_home: None,
            xdg_data_home: None,
            xdg_cache_home: None,
        };
        let mut environment = BTreeMap::from([
            ("HOME".into(), home.to_string_lossy().into_owned()),
            (
                "USERPROFILE".into(),
                canonical
                    .user_profile
                    .as_ref()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            ),
            (
                "APPDATA".into(),
                canonical
                    .app_data
                    .as_ref()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            ),
            (
                "LOCALAPPDATA".into(),
                canonical
                    .local_app_data
                    .as_ref()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            ),
            ("SystemRoot".into(), native_path("windows-system-root")),
            ("PATH".into(), native_path("windows-system32")),
        ]);
        assert!(
            validate_effective_environment_for_platform(&environment, &canonical, true).is_ok()
        );
        environment.remove("SystemRoot");
        assert!(
            validate_effective_environment_for_platform(&environment, &canonical, true).is_err()
        );
    }
}
