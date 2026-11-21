//! Stable identifiers, socket types, parameter specs, and analysis range.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! string_id {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

string_id!(NodeId, "Unique node instance identifier within a graph document.");
string_id!(NodeTypeId, "Stable node type identifier (for example `onset.flux`).");
string_id!(SocketId, "Stable socket identifier on a node type (for example `curve`).");
string_id!(ParamId, "Stable parameter identifier on a node type (for example `loHz`).");

/// Strict socket data types. Links connect matching types only; there are no implicit conversions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SocketType {
    Audio,
    Spectrogram,
    Curve,
    Events,
    Timing,
    Range,
    Number,
    Integer,
    Bool,
    Enum,
    /// Reserved for v2 keysound sample slicing so file formats remain forward-compatible.
    #[serde(rename = "audio-clip")]
    AudioClip,
}

impl fmt::Display for SocketType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            SocketType::Audio => "audio",
            SocketType::Spectrogram => "spectrogram",
            SocketType::Curve => "curve",
            SocketType::Events => "events",
            SocketType::Timing => "timing",
            SocketType::Range => "range",
            SocketType::Number => "number",
            SocketType::Integer => "integer",
            SocketType::Bool => "bool",
            SocketType::Enum => "enum",
            SocketType::AudioClip => "audio-clip",
        };
        f.write_str(name)
    }
}

/// Type constraint declared by a socket specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SocketConstraint {
    /// Accepts or produces strictly one `SocketType`.
    Exact(SocketType),
    /// Polymorphic inspection sink (such as `output.viewer`); never performs implicit conversion.
    Any,
}

impl SocketConstraint {
    pub fn accepts(self, other: SocketType) -> bool {
        match self {
            SocketConstraint::Exact(expected) => expected == other,
            SocketConstraint::Any => other != SocketType::AudioClip,
        }
    }
}

/// Description of one input or output socket on a node type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocketSpec {
    pub id: SocketId,
    pub kind: SocketConstraint,
    pub required: bool,
    pub multi: bool,
}

impl SocketSpec {
    pub fn exact(id: impl Into<SocketId>, socket_type: SocketType, required: bool) -> Self {
        Self {
            id: id.into(),
            kind: SocketConstraint::Exact(socket_type),
            required,
            multi: false,
        }
    }

    pub fn multi(id: impl Into<SocketId>, socket_type: SocketType, required: bool) -> Self {
        Self {
            id: id.into(),
            kind: SocketConstraint::Exact(socket_type),
            required,
            multi: true,
        }
    }

    pub fn any(id: impl Into<SocketId>, required: bool) -> Self {
        Self {
            id: id.into(),
            kind: SocketConstraint::Any,
            required,
            multi: false,
        }
    }
}

/// Description of a tunable node parameter with its default and optional bounds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamSpec {
    pub id: ParamId,
    pub socket_type: SocketType,
    pub default: serde_json::Value,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub enum_values: Vec<String>,
}

impl ParamSpec {
    pub fn number(
        id: impl Into<ParamId>,
        default: f64,
        min: Option<f64>,
        max: Option<f64>,
    ) -> Self {
        Self {
            id: id.into(),
            socket_type: SocketType::Number,
            default: serde_json::json!(default),
            min,
            max,
            enum_values: Vec::new(),
        }
    }

    pub fn integer(
        id: impl Into<ParamId>,
        default: i64,
        min: Option<i64>,
        max: Option<i64>,
    ) -> Self {
        Self {
            id: id.into(),
            socket_type: SocketType::Integer,
            default: serde_json::json!(default),
            min: min.map(|v| v as f64),
            max: max.map(|v| v as f64),
            enum_values: Vec::new(),
        }
    }

    pub fn boolean(id: impl Into<ParamId>, default: bool) -> Self {
        Self {
            id: id.into(),
            socket_type: SocketType::Bool,
            default: serde_json::json!(default),
            min: None,
            max: None,
            enum_values: Vec::new(),
        }
    }

    pub fn enumeration(
        id: impl Into<ParamId>,
        default: &str,
        variants: &[&str],
    ) -> Self {
        Self {
            id: id.into(),
            socket_type: SocketType::Enum,
            default: serde_json::json!(default),
            min: None,
            max: None,
            enum_values: variants.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    pub fn json(id: impl Into<ParamId>, socket_type: SocketType, default: serde_json::Value) -> Self {
        Self {
            id: id.into(),
            socket_type,
            default,
            min: None,
            max: None,
            enum_values: Vec::new(),
        }
    }
}

/// Half-open song-time interval `[from_ms, to_ms)` analysed by an evaluation run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AnalysisRange {
    pub from_ms: f64,
    pub to_ms: f64,
}

impl AnalysisRange {
    /// Validates and constructs `[from_ms, to_ms)` in song-time milliseconds.
    pub fn new(from_ms: f64, to_ms: f64) -> Result<Self, String> {
        if !from_ms.is_finite() || !to_ms.is_finite() {
            return Err("analysis range bounds must be finite".into());
        }
        if from_ms < 0.0 {
            return Err("analysis range start must be non-negative".into());
        }
        if to_ms <= from_ms {
            return Err("analysis range end must be greater than start".into());
        }
        Ok(Self {
            from_ms: if from_ms == 0.0 { 0.0 } else { from_ms },
            to_ms,
        })
    }

    /// True if `time_ms` falls within `[from_ms, to_ms)`.
    pub fn contains(&self, time_ms: f64) -> bool {
        time_ms.is_finite() && time_ms >= self.from_ms && time_ms < self.to_ms
    }

    /// Canonical IEEE-754 bit representations with `-0.0` normalized to `+0.0`.
    pub fn canonical_bits(&self) -> (u64, u64) {
        let norm = |v: f64| if v == 0.0 { 0.0_f64.to_bits() } else { v.to_bits() };
        (norm(self.from_ms), norm(self.to_ms))
    }
}

/// Explicit normalisation scope carried by normalising nodes and cache keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NormaliseScope {
    Range,
    Song,
}

impl NormaliseScope {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "range" => Ok(Self::Range),
            "song" => Ok(Self::Song),
            other => Err(format!("unknown normaliseOver scope '{other}'")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            NormaliseScope::Range => "range",
            NormaliseScope::Song => "song",
        }
    }
}
