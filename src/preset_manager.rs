use serde::{Serialize, Deserialize};
use std::fs;
use std::path::Path;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectionPreset {
    pub source_app: String,
    pub source_port: String,
    pub sink_app: String,
    pub sink_port: String,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Preset {
    pub connections: Vec<ConnectionPreset>,
}

impl Preset {
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn std::error::Error>> {
        let toml = toml::to_string_pretty(self)?;
        fs::write(path, toml)?;
        Ok(())
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let preset = toml::from_str(&content)?;
        Ok(preset)
    }
}
