//! Metadata contract for DTS extensions.

use std::collections::HashSet;

use serde::Deserialize;

use crate::error::{AppError, Result};

pub const PLUGIN_API_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    pub contributions: Vec<Contribution>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contribution {
    pub id: String,
    pub label: String,
    pub kind: ContributionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionKind {
    DocumentAction,
    Importer,
    Exporter,
    Editor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ReadDocument,
    WriteDocument,
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
            ("entrypoint", json!("run.exe"), "unknown field"),
        ];
        for (key, value, expected) in cases {
            let mut input = example.clone();
            input[key] = value;
            let error = PluginManifest::parse(&input.to_string()).unwrap_err();
            assert!(error.to_string().contains(expected), "{key}: {error}");
        }
    }
}
