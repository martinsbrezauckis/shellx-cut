//! Typed consumer for a Runner native-runtime context set.
//!
//! The set preserves separately sealed roots behind the existing one-locator
//! contract. Consumers select only their declared member IDs after every member
//! has passed its own typed validation.

use crate::{
    canonical_root, context_io::read_context, PinnedExecutableBundleContext, RuntimeContext,
    CONTEXT_ENV, MAX_FILES, PINNED_EXECUTABLE_BUNDLE_CONTEXT_CONTRACT, SET_CONTEXT_CONTRACT,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const MAX_SET_MEMBERS: usize = 16;
const MAX_SET_TOTAL_BYTES: u64 = 32 * 1024 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContextSetWire {
    schema: String,
    runtimes: Vec<ContextSetMemberWire>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContextSetMemberWire {
    id: String,
    context: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ContextSetMember {
    Python(RuntimeContext),
    PinnedExecutableBundle(PinnedExecutableBundleContext),
}

impl ContextSetMember {
    fn root(&self) -> &Path {
        match self {
            Self::Python(context) => &context.root,
            Self::PinnedExecutableBundle(context) => &context.root,
        }
    }

    fn files(&self) -> usize {
        match self {
            Self::Python(context) => context.files,
            Self::PinnedExecutableBundle(context) => context.files,
        }
    }

    fn total_bytes(&self) -> u64 {
        match self {
            Self::Python(context) => context.total_bytes,
            Self::PinnedExecutableBundle(context) => context.total_bytes,
        }
    }
}

/// A generic member set delivered through Runner's existing sealed locator.
/// Cut's callers choose their product-owned member IDs; this type never assigns
/// meaning to an ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeContextSet {
    locator: Option<PathBuf>,
    members: Vec<(String, ContextSetMember)>,
}

impl RuntimeContextSet {
    /// Read the fixed locator only when it contains the selected set schema.
    /// A present malformed set is an error and never authorizes a legacy
    /// Python/tool fallback.
    pub fn from_env() -> Result<Option<Self>, String> {
        match std::env::var_os(CONTEXT_ENV) {
            None => Ok(None),
            Some(path) if path.is_empty() => Err("native runtime context locator is empty".into()),
            Some(path) => {
                let locator = PathBuf::from(path);
                let mut set = Self::from_path(&locator)?;
                set.locator = Some(locator);
                Ok(Some(set))
            }
        }
    }

    pub fn from_path(path: &Path) -> Result<Self, String> {
        let bytes = read_context(path)?;
        let wire: ContextSetWire = serde_json::from_slice(&bytes)
            .map_err(|error| format!("parse native runtime context set: {error}"))?;
        if wire.schema != SET_CONTEXT_CONTRACT
            || wire.runtimes.is_empty()
            || wire.runtimes.len() > MAX_SET_MEMBERS
        {
            return Err("native runtime context set has an invalid schema or member count".into());
        }

        let mut members = Vec::with_capacity(wire.runtimes.len());
        let mut roots = BTreeSet::<PathBuf>::new();
        let mut previous = None::<String>;
        let mut files = 0_usize;
        let mut total_bytes = 0_u64;
        for member in wire.runtimes {
            if !set_id(&member.id)
                || previous
                    .as_deref()
                    .is_some_and(|prior| prior >= member.id.as_str())
            {
                return Err("native runtime context set requires sorted unique member IDs".into());
            }
            let context = parse_member(member.context)?;
            let root = canonical_root(context.root())?;
            if !roots.insert(root) {
                return Err("native runtime context set requires unique member roots".into());
            }
            files = files
                .checked_add(context.files())
                .ok_or("native runtime context set file count overflow")?;
            total_bytes = total_bytes
                .checked_add(context.total_bytes())
                .ok_or("native runtime context set byte count overflow")?;
            previous = Some(member.id.clone());
            members.push((member.id, context));
        }
        if files > MAX_FILES || total_bytes > MAX_SET_TOTAL_BYTES {
            return Err("native runtime context set exceeds aggregate inventory bounds".into());
        }
        Ok(Self {
            locator: None,
            members,
        })
    }

    /// The exact sealed locator that produced this set when it was loaded from
    /// the environment. A direct `from_path` parse has no inherited locator.
    pub fn locator(&self) -> Option<&Path> {
        self.locator.as_deref()
    }

    /// Select a product-owned Python member by its declared generic ID.
    pub fn python_context(&self, id: &str) -> Result<&RuntimeContext, String> {
        match self.member(id)? {
            ContextSetMember::Python(context) => Ok(context),
            ContextSetMember::PinnedExecutableBundle(_) => Err(format!(
                "native runtime context set member {id} is not a Python context"
            )),
        }
    }

    /// Select an optional Python member. Absence is distinct from a present
    /// member of the wrong type; every member was already validated at load.
    pub fn optional_python_context(&self, id: &str) -> Result<Option<&RuntimeContext>, String> {
        if self.members.iter().any(|(member_id, _)| member_id == id) {
            self.python_context(id).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Select a product-owned executable-bundle member by its declared generic
    /// ID.
    pub fn pinned_executable_bundle(
        &self,
        id: &str,
    ) -> Result<&PinnedExecutableBundleContext, String> {
        match self.member(id)? {
            ContextSetMember::Python(_) => Err(format!(
                "native runtime context set member {id} is not an executable bundle"
            )),
            ContextSetMember::PinnedExecutableBundle(context) => Ok(context),
        }
    }

    fn member(&self, id: &str) -> Result<&ContextSetMember, String> {
        self.members
            .iter()
            .find_map(|(member_id, context)| (member_id == id).then_some(context))
            .ok_or_else(|| format!("native runtime context set is missing member {id}"))
    }
}

fn parse_member(value: serde_json::Value) -> Result<ContextSetMember, String> {
    let schema = value
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or("native runtime context set member schema is missing")?;
    match schema {
        crate::CONTEXT_CONTRACT => RuntimeContext::from_value(value).map(ContextSetMember::Python),
        PINNED_EXECUTABLE_BUNDLE_CONTEXT_CONTRACT => {
            PinnedExecutableBundleContext::from_value(value)
                .map(ContextSetMember::PinnedExecutableBundle)
        }
        _ => Err("native runtime context set member schema is not supported by ShellX Cut".into()),
    }
}

fn set_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
