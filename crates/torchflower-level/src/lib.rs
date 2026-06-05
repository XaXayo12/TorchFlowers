#![allow(unknown_lints)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum LevelError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("world path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("chunk is not loaded by the lightweight reader yet")]
    ChunkUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Dimension {
    Overworld,
    Nether,
    End,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockState {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    pub x: i32,
    pub z: i32,
    pub dimension: Dimension,
}

impl Chunk {
    pub fn get_block(&self, _x: u8, _y: i16, _z: u8) -> BlockState {
        BlockState {
            name: "minecraft:air".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerData {
    pub xuid: String,
    pub raw_nbt: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelDat {
    pub raw_little_endian_nbt: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct World {
    path: PathBuf,
}

impl World {
    pub fn open(path: &Path) -> Result<Self, LevelError> {
        if !path.is_dir() {
            return Err(LevelError::NotDirectory(path.to_path_buf()));
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    pub fn get_chunk(
        &self,
        x: i32,
        z: i32,
        dimension: Dimension,
    ) -> Result<Option<Chunk>, LevelError> {
        let key = chunk_key(x, z, dimension, 47);
        let db_path = self.path.join("db");
        if !db_path.exists() {
            return Ok(None);
        }
        let _ = key;
        Ok(None)
    }

    pub fn get_player_data(&self, xuid: &str) -> Result<Option<PlayerData>, LevelError> {
        let path = self.path.join("playerdata").join(format!("{xuid}.dat"));
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(PlayerData {
            xuid: xuid.to_string(),
            raw_nbt: fs::read(path)?,
        }))
    }

    pub fn level_dat(&self) -> Result<LevelDat, LevelError> {
        Ok(LevelDat {
            raw_little_endian_nbt: fs::read(self.path.join("level.dat"))?,
        })
    }
}

pub fn chunk_key(x: i32, z: i32, dimension: Dimension, tag: u8) -> Vec<u8> {
    let mut key = Vec::with_capacity(13);
    key.extend_from_slice(&x.to_le_bytes());
    key.extend_from_slice(&z.to_le_bytes());
    if dimension != Dimension::Overworld {
        key.extend_from_slice(&dimension_id(dimension).to_le_bytes());
    }
    key.push(tag);
    key
}

pub fn parse_chunk_key(key: &[u8]) -> Option<(i32, i32, Dimension, u8)> {
    match key.len() {
        9 => Some((
            i32::from_le_bytes(key[0..4].try_into().ok()?),
            i32::from_le_bytes(key[4..8].try_into().ok()?),
            Dimension::Overworld,
            key[8],
        )),
        13 => Some((
            i32::from_le_bytes(key[0..4].try_into().ok()?),
            i32::from_le_bytes(key[4..8].try_into().ok()?),
            dimension_from_id(i32::from_le_bytes(key[8..12].try_into().ok()?))?,
            key[12],
        )),
        _ => None,
    }
}

fn dimension_id(dimension: Dimension) -> i32 {
    match dimension {
        Dimension::Overworld => 0,
        Dimension::Nether => 1,
        Dimension::End => 2,
    }
}

fn dimension_from_id(id: i32) -> Option<Dimension> {
    match id {
        0 => Some(Dimension::Overworld),
        1 => Some(Dimension::Nether),
        2 => Some(Dimension::End),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_key_round_trip_overworld() {
        let key = chunk_key(-12, 34, Dimension::Overworld, 47);
        assert_eq!(
            parse_chunk_key(&key),
            Some((-12, 34, Dimension::Overworld, 47))
        );
    }

    #[test]
    fn chunk_key_round_trip_dimension() {
        let key = chunk_key(1, 2, Dimension::Nether, 54);
        assert_eq!(parse_chunk_key(&key), Some((1, 2, Dimension::Nether, 54)));
    }
}
