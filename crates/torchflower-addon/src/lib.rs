#![allow(unknown_lints)]

use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum AddonError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("archive does not contain manifest.json")]
    MissingManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonManifest {
    pub header: ManifestHeader,
    pub modules: Vec<Module>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestHeader {
    pub name: String,
    pub description: String,
    pub uuid: Uuid,
    pub version: [u32; 3],
    pub min_engine_version: [u32; 3],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Module {
    Resources { uuid: Uuid, version: [u32; 3] },
    Data { uuid: Uuid, version: [u32; 3] },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub uuid: Option<Uuid>,
    pub module_name: Option<String>,
    pub version: Option<[u32; 3]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationWarning {
    pub message: String,
}

impl AddonManifest {
    pub fn from_file(path: &Path) -> Result<Self, AddonError> {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }

    pub fn to_file(&self, path: &Path) -> Result<(), AddonError> {
        fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn validate(&self) -> Vec<ValidationWarning> {
        let mut warnings = Vec::new();
        if self.header.name.trim().is_empty() {
            warnings.push(ValidationWarning {
                message: "manifest header name is empty".to_string(),
            });
        }
        if self.modules.is_empty() {
            warnings.push(ValidationWarning {
                message: "manifest has no modules".to_string(),
            });
        }
        warnings
    }
}

pub struct McPack {
    pub manifest: AddonManifest,
    pub files: HashMap<PathBuf, Vec<u8>>,
}

impl McPack {
    pub fn from_archive(path: &Path) -> Result<Self, AddonError> {
        let file = fs::File::open(path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        let mut files = HashMap::new();
        let mut manifest = None;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            let path = PathBuf::from(entry.name());
            if path.file_name().and_then(|name| name.to_str()) == Some("manifest.json") {
                manifest = Some(serde_json::from_slice(&bytes)?);
            }
            files.insert(path, bytes);
        }
        Ok(Self {
            manifest: manifest.ok_or(AddonError::MissingManifest)?,
            files,
        })
    }

    pub fn extract_to(self, dest: &Path) -> Result<(), AddonError> {
        fs::create_dir_all(dest)?;
        for (relative, bytes) in self.files {
            let path = dest.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::File::create(path)?.write_all(&bytes)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trip() {
        let manifest = AddonManifest {
            header: ManifestHeader {
                name: "Pack".to_string(),
                description: "Test".to_string(),
                uuid: Uuid::nil(),
                version: [1, 0, 0],
                min_engine_version: [1, 21, 0],
            },
            modules: vec![Module::Resources {
                uuid: Uuid::nil(),
                version: [1, 0, 0],
            }],
            dependencies: Vec::new(),
        };
        let encoded = serde_json::to_string(&manifest).unwrap();
        let decoded: AddonManifest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, manifest);
        assert!(decoded.validate().is_empty());
    }
}
