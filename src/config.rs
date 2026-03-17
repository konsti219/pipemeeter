use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

fn default_volume() -> f32 {
    1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeMatchProperty {
    #[default]
    Name,
    Description,
    MediaName,
    ProcessBinary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMatchRequirement {
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub match_property: NodeMatchProperty,
}

impl NodeMatchRequirement {
    pub fn new(pattern: String, match_property: NodeMatchProperty) -> Self {
        Self {
            pattern,
            match_property,
        }
    }
}

impl NodeMatchProperty {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Description => "Description",
            Self::MediaName => "Media name",
            Self::ProcessBinary => "Process binary",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub requirements: Vec<NodeMatchRequirement>,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub placeholder_meter: f32,
    #[serde(default)]
    pub routes_to_outputs: Vec<bool>,
}

impl StripConfig {
    pub fn new(name: String) -> Self {
        Self {
            name,
            requirements: vec![NodeMatchRequirement {
                pattern: String::new(),
                match_property: NodeMatchProperty::Name,
            }],
            volume: 1.0,
            placeholder_meter: 0.0,
            routes_to_outputs: Vec::new(),
        }
    }
    pub fn with_routes(name: String, output_count: usize) -> Self {
        let mut config = Self::new(name);
        config.routes_to_outputs.resize(output_count, false);
        config
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub physical_inputs: Vec<StripConfig>,
    pub virtual_inputs: Vec<StripConfig>,
    pub physical_outputs: Vec<StripConfig>,
    pub virtual_outputs: Vec<StripConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            physical_inputs: Vec::new(),
            virtual_inputs: vec![StripConfig::with_routes("Defaut".to_owned(), 0)],
            physical_outputs: Vec::new(),
            virtual_outputs: vec![StripConfig::new("Defaut".to_owned())],
        }
    }
}

impl AppConfig {
    fn normalize_strip(strip: &mut StripConfig, default_name: &str) {
        if strip.name.trim().is_empty() {
            strip.name = default_name.to_owned();
        }

        strip.volume = strip.volume.clamp(0.0, 1.0);
        strip.placeholder_meter = strip.placeholder_meter.clamp(0.0, 1.0);
    }

    pub fn output_count(&self) -> usize {
        self.physical_outputs.len() + self.virtual_outputs.len()
    }

    pub fn output_labels(&self) -> Vec<String> {
        let physical = self
            .physical_outputs
            .iter()
            .enumerate()
            .map(|(idx, _)| format!("P{}", idx + 1));
        let virtuals = self
            .virtual_outputs
            .iter()
            .enumerate()
            .map(|(idx, _)| format!("V{}", idx + 1));

        physical.chain(virtuals).collect()
    }

    pub fn normalize(&mut self) {
        if self.virtual_inputs.is_empty() {
            self.virtual_inputs
                .push(StripConfig::new("Defaut".to_owned()));
        }
        self.virtual_inputs[0].requirements.clear();
        if self.virtual_outputs.is_empty() {
            self.virtual_outputs
                .push(StripConfig::new("Defaut".to_owned()));
        }
        self.virtual_outputs[0].requirements.clear();

        for input in self
            .physical_inputs
            .iter_mut()
            .chain(self.virtual_inputs.iter_mut())
        {
            Self::normalize_strip(input, "Input");
        }

        for output in self
            .physical_outputs
            .iter_mut()
            .chain(self.virtual_outputs.iter_mut())
        {
            Self::normalize_strip(output, "Output");
            output.routes_to_outputs.clear();
        }

        let output_count = self.output_count();
        for input in self
            .physical_inputs
            .iter_mut()
            .chain(self.virtual_inputs.iter_mut())
        {
            input.routes_to_outputs.resize(output_count, false);
        }
    }
}

pub fn config_path() -> Result<PathBuf> {
    let base = dirs::config_dir().context("unable to resolve config directory")?;
    Ok(base.join("pipemeeter").join("config.json"))
}

pub fn load_config(path: &Path) -> Result<AppConfig> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read config at {}", path.display()))?;
    let mut config = serde_json::from_str::<AppConfig>(&text)
        .with_context(|| format!("failed to parse config at {}", path.display()))?;
    config.normalize();
    Ok(config)
}

pub fn save_config(path: &Path, config: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config dir {}", parent.display()))?;
    }

    let mut normalized = config.clone();
    normalized.normalize();

    let text = serde_json::to_string_pretty(&normalized).context("failed to serialize config")?;
    fs::write(path, text)
        .with_context(|| format!("failed to write config at {}", path.display()))?;
    Ok(())
}
