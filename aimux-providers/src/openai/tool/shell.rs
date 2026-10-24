//! The shell provider tool and its arguments (tool/shell.ts).

use std::collections::BTreeMap;

use aimux_core::AiMuxError;
use aimux_core::tool::ProviderTool;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::openai::tools::provider_tool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ShellMemoryLimit {
    #[serde(rename = "1g")]
    OneG,
    #[serde(rename = "4g")]
    FourG,
    #[serde(rename = "16g")]
    SixteenG,
    #[serde(rename = "64g")]
    SixtyFourG,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellDomainSecret {
    pub domain: String,

    pub name: String,

    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShellNetworkPolicy {
    Disabled,
    Allowlist {
        #[serde(rename = "allowedDomains")]
        allowed_domains: Vec<String>,
        #[serde(
            rename = "domainSecrets",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        domain_secrets: Option<Vec<ShellDomainSecret>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShellSkill {
    SkillReference {
        #[serde(rename = "providerReference")]
        provider_reference: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    Inline {
        name: String,
        description: String,
        source: ShellSkillSource,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShellSkillSource {
    Base64 {
        #[serde(rename = "mediaType")]
        media_type: ShellSkillMediaType,
        data: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ShellSkillMediaType {
    #[serde(rename = "application/zip")]
    Zip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalShellSkill {
    pub name: String,

    pub description: String,

    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShellEnvironment {
    ContainerAuto {
        #[serde(rename = "fileIds", default, skip_serializing_if = "Option::is_none")]
        file_ids: Option<Vec<String>>,
        #[serde(
            rename = "memoryLimit",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        memory_limit: Option<ShellMemoryLimit>,
        #[serde(
            rename = "networkPolicy",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        network_policy: Option<ShellNetworkPolicy>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skills: Option<Vec<ShellSkill>>,
    },
    ContainerReference {
        #[serde(rename = "containerId")]
        container_id: String,
    },
    Local {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skills: Option<Vec<LocalShellSkill>>,
    },
}

impl<'de> Deserialize<'de> for ShellEnvironment {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(remote = "ShellEnvironment", tag = "type", rename_all = "camelCase")]
        enum TaggedEnvironment {
            ContainerAuto {
                #[serde(rename = "fileIds")]
                file_ids: Option<Vec<String>>,
                #[serde(rename = "memoryLimit")]
                memory_limit: Option<ShellMemoryLimit>,
                #[serde(rename = "networkPolicy")]
                network_policy: Option<ShellNetworkPolicy>,
                skills: Option<Vec<ShellSkill>>,
            },
            ContainerReference {
                #[serde(rename = "containerId")]
                container_id: String,
            },
            Local {
                skills: Option<Vec<LocalShellSkill>>,
            },
        }

        let mut value = Value::deserialize(deserializer)?;
        if let Some(object) = value.as_object_mut() {
            object
                .entry("type")
                .or_insert_with(|| Value::String("local".into()));
        }
        TaggedEnvironment::deserialize(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<ShellEnvironment>,
}

/// Configure the `shell` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn shell(args: ShellArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("shell", args)
}
