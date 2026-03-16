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
pub struct InputStripConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub represented_node_requirements: Vec<NodeMatchRequirement>,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub placeholder_meter: f32,
    #[serde(default)]
    pub routes_to_outputs: Vec<bool>,
}

impl InputStripConfig {
    pub fn new(name: String, output_count: usize) -> Self {
        Self {
            name,
            represented_node_requirements: Vec::new(),
            volume: 1.0,
            placeholder_meter: 0.0,
            routes_to_outputs: vec![false; output_count],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputStripConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub represented_node_requirements: Vec<NodeMatchRequirement>,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub placeholder_meter: f32,
}

impl OutputStripConfig {
    pub fn new(name: String) -> Self {
        Self {
            name,
            represented_node_requirements: Vec::new(),
            volume: 1.0,
            placeholder_meter: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub physical_inputs: Vec<InputStripConfig>,
    pub virtual_inputs: Vec<InputStripConfig>,
    pub physical_outputs: Vec<OutputStripConfig>,
    pub virtual_outputs: Vec<OutputStripConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            physical_inputs: Vec::new(),
            virtual_inputs: vec![InputStripConfig::new("Virtual In 1".to_owned(), 0)],
            physical_outputs: Vec::new(),
            virtual_outputs: vec![OutputStripConfig::new("Virtual Out 1".to_owned())],
        }
    }
}

impl AppConfig {
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
                .push(InputStripConfig::new("Virtual In 1".to_owned(), 0));
        }
        if self.virtual_outputs.is_empty() {
            self.virtual_outputs
                .push(OutputStripConfig::new("Virtual Out 1".to_owned()));
        }

        for input in self
            .physical_inputs
            .iter_mut()
            .chain(self.virtual_inputs.iter_mut())
        {
            if input.name.trim().is_empty() {
                input.name = "Input".to_owned();
            }
            input
                .represented_node_requirements
                .retain(|requirement| !requirement.pattern.trim().is_empty());

            input.volume = input.volume.clamp(0.0, 1.0);
            input.placeholder_meter = input.placeholder_meter.clamp(0.0, 1.0);
        }

        for output in self
            .physical_outputs
            .iter_mut()
            .chain(self.virtual_outputs.iter_mut())
        {
            if output.name.trim().is_empty() {
                output.name = "Output".to_owned();
            }
            output
                .represented_node_requirements
                .retain(|requirement| !requirement.pattern.trim().is_empty());

            output.volume = output.volume.clamp(0.0, 1.0);
            output.placeholder_meter = output.placeholder_meter.clamp(0.0, 1.0);
        }

        if !self
            .virtual_inputs
            .iter()
            .any(|strip| strip.represented_node_requirements.is_empty())
        {
            let output_count = self.output_count();
            self.virtual_inputs.push(InputStripConfig::new(
                format!("Virtual In {}", self.virtual_inputs.len() + 1),
                output_count,
            ));
        }

        if !self
            .virtual_outputs
            .iter()
            .any(|strip| strip.represented_node_requirements.is_empty())
        {
            self.virtual_outputs.push(OutputStripConfig::new(format!(
                "Virtual Out {}",
                self.virtual_outputs.len() + 1
            )));
        }

        self.virtual_inputs
            .sort_by_key(|strip| !strip.represented_node_requirements.is_empty());
        self.virtual_outputs
            .sort_by_key(|strip| !strip.represented_node_requirements.is_empty());

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
