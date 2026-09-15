//! Bounded child-environment policies for Agent Chat providers.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

/// Environment policy for one local-agent launch.
#[derive(Clone, Debug)]
pub struct LaunchEnvironment {
    pub(super) clear_inherited: bool,
    pub(super) vars: Vec<(OsString, OsString)>,
}

impl LaunchEnvironment {
    #[cfg(test)]
    pub fn names(&self) -> Vec<String> {
        self.vars
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect()
    }

    /// Apply the selected provider environment first, then this surface's
    /// intentionally narrow policy.  The policy may isolate provider config or
    /// add the Cut MCP bridge, but no unrelated application environment reaches
    /// an admitted provider child.
    pub fn apply_with_admitted_environment(
        &self,
        command: &mut tokio::process::Command,
        admitted: Option<&BTreeMap<String, String>>,
    ) {
        if let Some(environment) = admitted {
            command.env_clear();
            command.envs(environment);
        } else if self.clear_inherited {
            command.env_clear();
        }
        command.envs(self.vars.iter().cloned());
    }

    /// An admitted child must keep its provider environment bounded while using
    /// a Cut-owned operation directory for temporary files.  Ordinary launches
    /// retain their existing environment behavior.
    pub fn with_owned_temporary_directory(mut self, directory: &Path) -> Result<Self, String> {
        if !directory.is_absolute() {
            return Err(format!(
                "provider temporary directory must be absolute: {}",
                directory.display()
            ));
        }
        std::fs::create_dir_all(directory).map_err(|error| {
            format!(
                "could not create provider temporary directory {}: {error}",
                directory.display()
            )
        })?;
        self.vars.retain(|(name, _)| {
            !["TMPDIR", "TEMP", "TMP"]
                .iter()
                .any(|temporary| name.to_string_lossy().eq_ignore_ascii_case(temporary))
        });
        for name in ["TMPDIR", "TEMP", "TMP"] {
            self.vars
                .push((OsString::from(name), directory.as_os_str().to_owned()));
        }
        Ok(self)
    }
}

fn named<I>(environment: I, wanted: &str) -> Option<OsString>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    environment.into_iter().find_map(|(name, value)| {
        name.to_string_lossy()
            .eq_ignore_ascii_case(wanted)
            .then_some(value)
    })
}

/// Retain only runtime/auth routing essentials; remove credentials, proxies,
/// plugin settings, MCP variables, and all caller-controlled sentinel values.
pub fn sanitized_environment_from<I>(
    inherited: I,
    proxy_addr: &str,
    proxy_actor: &str,
) -> Result<LaunchEnvironment, String>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let entries: Vec<(OsString, OsString)> = inherited.into_iter().collect();
    let path = named(entries.clone(), "PATH").ok_or_else(|| {
        "cannot launch contained Claude: inherited PATH is unavailable".to_string()
    })?;
    let home = named(entries.clone(), "HOME");
    let user_profile = named(entries.clone(), "USERPROFILE");
    if home.is_none() && user_profile.is_none() {
        return Err("cannot launch contained Claude: HOME or USERPROFILE is unavailable for its existing login".into());
    }

    let mut vars = BTreeMap::new();
    vars.insert(OsString::from("PATH"), path);
    if let Some(home) = home {
        vars.insert(OsString::from("HOME"), home);
    }
    if let Some(profile) = user_profile {
        vars.insert(OsString::from("USERPROFILE"), profile);
    }
    // Windows CSPRNG initialization reaches system libraries through SystemRoot.
    // A fully cleared environment without this OS-owned path makes Node-based CLIs
    // abort before their own capability probe runs. Preserve only that runtime
    // locator; credentials and caller-controlled integration variables stay absent.
    #[cfg(windows)]
    if let Some(system_root) = named(entries.clone(), "SystemRoot") {
        vars.insert(OsString::from("SystemRoot"), system_root);
    }
    for locale in ["LANG", "LC_ALL"] {
        if let Some(value) = named(entries.clone(), locale) {
            vars.insert(OsString::from(locale), value);
        }
    }
    vars.insert(
        OsString::from("CUTD_PROXY_ADDR"),
        OsString::from(proxy_addr),
    );
    vars.insert(
        OsString::from("CUTD_PROXY_ACTOR"),
        OsString::from(proxy_actor),
    );
    vars.insert(
        OsString::from(crate::chat::capabilities::RESTRICTED_MCP_MARKER),
        OsString::from(crate::chat::capabilities::RESTRICTED_MCP_MARKER_VALUE),
    );
    Ok(LaunchEnvironment {
        clear_inherited: true,
        vars: vars.into_iter().collect(),
    })
}

pub fn sanitized_environment(
    proxy_addr: &str,
    proxy_actor: &str,
) -> Result<LaunchEnvironment, String> {
    sanitized_environment_from(std::env::vars_os(), proxy_addr, proxy_actor)
}

/// Claude's contained policy needs login/runtime locators from the admitted
/// provider environment, never from Cut's process environment.  This helper
/// keeps its ordinary-env counterpart unchanged while preserving only the
/// curated locator set that the Runner admitted for this child.
pub fn sanitized_environment_from_admitted(
    admitted: &BTreeMap<String, String>,
    proxy_addr: &str,
    proxy_actor: &str,
) -> Result<LaunchEnvironment, String> {
    let entries: Vec<(OsString, OsString)> = admitted
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect();
    let path = named(entries.clone(), "PATH").ok_or_else(|| {
        "cannot launch contained Claude: admitted PATH is unavailable".to_string()
    })?;
    let home = named(entries.clone(), "HOME");
    let user_profile = named(entries.clone(), "USERPROFILE");
    if home.is_none() && user_profile.is_none() {
        return Err("cannot launch contained Claude: admitted HOME or USERPROFILE is unavailable for its existing login".into());
    }

    let mut vars = BTreeMap::new();
    vars.insert(OsString::from("PATH"), path);
    for name in [
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "SystemRoot",
    ] {
        if let Some(value) = named(entries.clone(), name) {
            vars.insert(OsString::from(name), value);
        }
    }
    vars.insert(
        OsString::from("CUTD_PROXY_ADDR"),
        OsString::from(proxy_addr),
    );
    vars.insert(
        OsString::from("CUTD_PROXY_ACTOR"),
        OsString::from(proxy_actor),
    );
    vars.insert(
        OsString::from(crate::chat::capabilities::RESTRICTED_MCP_MARKER),
        OsString::from(crate::chat::capabilities::RESTRICTED_MCP_MARKER_VALUE),
    );
    Ok(LaunchEnvironment {
        clear_inherited: true,
        vars: vars.into_iter().collect(),
    })
}

pub fn sanitized_environment_for_provider(
    admitted: Option<&BTreeMap<String, String>>,
    proxy_addr: &str,
    proxy_actor: &str,
) -> Result<LaunchEnvironment, String> {
    match admitted {
        Some(environment) => {
            sanitized_environment_from_admitted(environment, proxy_addr, proxy_actor)
        }
        None => sanitized_environment(proxy_addr, proxy_actor),
    }
}

/// Preserve the user's normal CLI environment and auth/config routing for
/// native-policy providers. Cut only adds the exact live-engine proxy values
/// and restricted-MCP marker consumed by its MCP child; it does not copy, move,
/// or rewrite the CLI's credential files.
pub fn native_environment(proxy_addr: &str, proxy_actor: &str) -> LaunchEnvironment {
    LaunchEnvironment {
        clear_inherited: false,
        vars: vec![
            (
                OsString::from("CUTD_PROXY_ADDR"),
                OsString::from(proxy_addr),
            ),
            (
                OsString::from("CUTD_PROXY_ACTOR"),
                OsString::from(proxy_actor),
            ),
            (
                OsString::from(crate::chat::capabilities::RESTRICTED_MCP_MARKER),
                OsString::from(crate::chat::capabilities::RESTRICTED_MCP_MARKER_VALUE),
            ),
        ],
    }
}
