#[derive(Debug, Clone)]
pub struct VersionMetadata {
    pub version_string: &'static str,
    pub protocol_version: u32,
}

pub fn get_supported_versions() -> Vec<VersionMetadata> {
    vec![
        VersionMetadata {
            version_string: "1.21.100",
            protocol_version: 766,
        },
        VersionMetadata {
            version_string: "1.21.130",
            protocol_version: 776,
        },
    ]
}

pub struct BlockRegistry;

impl BlockRegistry {
    pub fn get_block_runtime_id(name: &str) -> Option<u32> {
        match name {
            "minecraft:air" => Some(0),
            "minecraft:stone" => Some(1),
            "minecraft:grass_block" => Some(2),
            "minecraft:dirt" => Some(3),
            "minecraft:cobblestone" => Some(4),
            _ => None,
        }
    }
}
