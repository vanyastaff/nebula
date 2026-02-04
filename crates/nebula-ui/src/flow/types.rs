//! Core types for the flow editor.

use egui::{Color32, Pos2, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub use super::connection::EdgeType;

/// Unique identifier for a node
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub Uuid);

impl NodeId {
    /// Create a new node ID from a UUID
    pub fn new(id: Uuid) -> Self {
        Self(id)
    }

    /// Generate a random ID
    pub fn random() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Unique identifier for a pin
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PinId {
    /// Node this pin belongs to
    pub node: NodeId,
    /// Pin index within the node
    pub index: u32,
    /// Whether this is an input or output pin
    pub kind: PinKind,
}

impl PinId {
    /// Create a new pin ID
    pub fn new(node: NodeId, index: u32, kind: PinKind) -> Self {
        Self { node, index, kind }
    }
}

/// Unique identifier for a connection
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub Uuid);

impl ConnectionId {
    /// Create a new connection ID from a UUID
    pub fn new(id: Uuid) -> Self {
        Self(id)
    }

    /// Generate a random ID
    pub fn random() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Unique identifier for a layer
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerId(pub Uuid);

impl LayerId {
    /// Create a new layer ID from a UUID
    pub fn new(id: Uuid) -> Self {
        Self(id)
    }

    /// Generate a random ID
    pub fn random() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Pin kind (input or output)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PinKind {
    /// Input pin (receives data)
    Input,
    /// Output pin (sends data)
    Output,
}

/// Data type for pins
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DataType {
    /// Execution/control flow
    Execution,
    /// String data
    String,
    /// Numeric data
    Number,
    /// Boolean data
    Boolean,
    /// Array of a specific type
    Array(Box<DataType>),
    /// Object/map data
    Object,
    /// Generic (any type)
    Generic,
    /// Structured type with schema
    Struct(String),
    /// Binary data
    Bytes,
}

impl DataType {
    /// Check if this type is compatible with another
    pub fn is_compatible(&self, other: &DataType) -> bool {
        match (self, other) {
            // Generic is compatible with everything
            (DataType::Generic, _) | (_, DataType::Generic) => true,

            // Execution only connects to execution
            (DataType::Execution, DataType::Execution) => true,
            (DataType::Execution, _) | (_, DataType::Execution) => false,

            // Exact match
            (a, b) if a == b => true,

            // Arrays with compatible inner types
            (DataType::Array(a), DataType::Array(b)) => a.is_compatible(b),

            // Number can connect to generic number contexts
            (DataType::Number, DataType::String) => true, // Numbers can be stringified

            _ => false,
        }
    }

    /// Get display name
    pub fn name(&self) -> &str {
        match self {
            DataType::Execution => "Execution",
            DataType::String => "String",
            DataType::Number => "Number",
            DataType::Boolean => "Boolean",
            DataType::Array(_) => "Array",
            DataType::Object => "Object",
            DataType::Generic => "Any",
            DataType::Struct(name) => name,
            DataType::Bytes => "Bytes",
        }
    }
}

/// Value type modifier (single value, array, etc.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ValueType {
    /// Single value
    #[default]
    Normal,
    /// Array of values
    Array,
    /// HashMap
    HashMap,
    /// HashSet
    HashSet,
}

/// A pin definition
#[derive(Clone, Debug)]
pub struct Pin {
    /// Pin ID
    pub id: PinId,
    /// Display name
    pub name: String,
    /// Data type
    pub data_type: DataType,
    /// Value type modifier
    pub value_type: ValueType,
    /// Whether this pin is required
    pub required: bool,
    /// Default value (for inputs)
    pub default_value: Option<String>,
    /// Current value (for runtime)
    pub value: Option<String>,
    /// Whether this pin is connected
    pub connected: bool,
}

impl Pin {
    /// Create a new input pin
    pub fn input(node: NodeId, index: u32, name: impl Into<String>, data_type: DataType) -> Self {
        Self {
            id: PinId::new(node, index, PinKind::Input),
            name: name.into(),
            data_type,
            value_type: ValueType::Normal,
            required: false,
            default_value: None,
            value: None,
            connected: false,
        }
    }

    /// Create a new output pin
    pub fn output(node: NodeId, index: u32, name: impl Into<String>, data_type: DataType) -> Self {
        Self {
            id: PinId::new(node, index, PinKind::Output),
            name: name.into(),
            data_type,
            value_type: ValueType::Normal,
            required: false,
            default_value: None,
            value: None,
            connected: false,
        }
    }

    /// Set as required
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Set default value
    pub fn with_default(mut self, value: impl Into<String>) -> Self {
        self.default_value = Some(value.into());
        self
    }

    /// Set value type
    pub fn value_type(mut self, vt: ValueType) -> Self {
        self.value_type = vt;
        self
    }
}

/// A node in the flow graph
#[derive(Clone, Debug)]
pub struct Node {
    /// Node ID
    pub id: NodeId,
    /// Node type identifier
    pub type_id: String,
    /// Display name
    pub name: String,
    /// Category
    pub category: String,
    /// Position on canvas
    pub position: Pos2,
    /// Size (calculated from content)
    pub size: Vec2,
    /// Input pins
    pub inputs: Vec<Pin>,
    /// Output pins
    pub outputs: Vec<Pin>,
    /// Parent layer (None = root)
    pub layer: Option<LayerId>,
    /// Whether this is an event/trigger node
    pub is_event: bool,
    /// Custom color override
    pub color: Option<Color32>,
    /// Node-specific data (JSON or similar)
    pub data: HashMap<String, String>,
}

impl Node {
    /// Create a new node
    pub fn new(type_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: NodeId::random(),
            type_id: type_id.into(),
            name: name.into(),
            category: "default".into(),
            position: Pos2::ZERO,
            size: Vec2::new(200.0, 100.0),
            inputs: Vec::new(),
            outputs: Vec::new(),
            layer: None,
            is_event: false,
            color: None,
            data: HashMap::new(),
        }
    }

    /// Set position
    pub fn at(mut self, pos: Pos2) -> Self {
        self.position = pos;
        self
    }

    /// Set category
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }

    /// Add input pin
    pub fn input(mut self, name: impl Into<String>, data_type: DataType) -> Self {
        let index = self.inputs.len() as u32;
        self.inputs
            .push(Pin::input(self.id, index, name, data_type));
        self
    }

    /// Add output pin
    pub fn output(mut self, name: impl Into<String>, data_type: DataType) -> Self {
        let index = self.outputs.len() as u32;
        self.outputs
            .push(Pin::output(self.id, index, name, data_type));
        self
    }

    /// Mark as event node
    pub fn event(mut self) -> Self {
        self.is_event = true;
        self
    }

    /// Get all pins
    pub fn pins(&self) -> impl Iterator<Item = &Pin> {
        self.inputs.iter().chain(self.outputs.iter())
    }

    /// Get pin by ID
    pub fn pin(&self, id: PinId) -> Option<&Pin> {
        match id.kind {
            PinKind::Input => self.inputs.get(id.index as usize),
            PinKind::Output => self.outputs.get(id.index as usize),
        }
    }

    /// Get mutable pin by ID
    pub fn pin_mut(&mut self, id: PinId) -> Option<&mut Pin> {
        match id.kind {
            PinKind::Input => self.inputs.get_mut(id.index as usize),
            PinKind::Output => self.outputs.get_mut(id.index as usize),
        }
    }
}

/// A connection between two pins
#[derive(Clone, Debug)]
pub struct Connection {
    /// Connection ID
    pub id: ConnectionId,
    /// Source pin (output)
    pub source: PinId,
    /// Target pin (input)
    pub target: PinId,
    /// Data type (for styling)
    pub data_type: DataType,
    /// Edge type (path style)
    pub edge_type: EdgeType,
}

impl Connection {
    /// Create a new connection
    pub fn new(source: PinId, target: PinId, data_type: DataType) -> Self {
        Self {
            id: ConnectionId::random(),
            source,
            target,
            data_type,
            edge_type: EdgeType::default(),
        }
    }

    /// Create a new connection with specific edge type
    pub fn with_edge_type(
        source: PinId,
        target: PinId,
        data_type: DataType,
        edge_type: EdgeType,
    ) -> Self {
        Self {
            id: ConnectionId::random(),
            source,
            target,
            data_type,
            edge_type,
        }
    }

    /// Set the edge type
    pub fn edge_type(mut self, edge_type: EdgeType) -> Self {
        self.edge_type = edge_type;
        self
    }
}

/// A layer for hierarchical grouping
#[derive(Clone, Debug)]
pub struct Layer {
    /// Layer ID
    pub id: LayerId,
    /// Display name
    pub name: String,
    /// Parent layer
    pub parent: Option<LayerId>,
    /// Nodes in this layer
    pub nodes: HashSet<NodeId>,
    /// Position when collapsed
    pub position: Pos2,
    /// Size when collapsed
    pub size: Vec2,
    /// Boundary pins (interface)
    pub boundary_pins: Vec<Pin>,
}

impl Layer {
    /// Create a new layer
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: LayerId::random(),
            name: name.into(),
            parent: None,
            nodes: HashSet::new(),
            position: Pos2::ZERO,
            size: Vec2::new(200.0, 100.0),
            boundary_pins: Vec::new(),
        }
    }
}

/// A variable in the board
#[derive(Clone, Debug)]
pub struct Variable {
    /// Variable ID
    pub id: String,
    /// Display name
    pub name: String,
    /// Data type
    pub data_type: DataType,
    /// Default value
    pub default_value: String,
    /// Current value
    pub current_value: String,
}

impl Variable {
    /// Create a new variable
    pub fn new(id: impl Into<String>, name: impl Into<String>, data_type: DataType) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            data_type,
            default_value: String::new(),
            current_value: String::new(),
        }
    }

    /// Set default value
    pub fn with_default(mut self, value: impl Into<String>) -> Self {
        self.default_value = value.into();
        self.current_value = self.default_value.clone();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_type_compatibility() {
        assert!(DataType::Generic.is_compatible(&DataType::String));
        assert!(DataType::String.is_compatible(&DataType::Generic));
        assert!(DataType::String.is_compatible(&DataType::String));
        assert!(!DataType::String.is_compatible(&DataType::Number));
        assert!(!DataType::Execution.is_compatible(&DataType::String));
        assert!(DataType::Execution.is_compatible(&DataType::Execution));
    }

    #[test]
    fn test_node_builder() {
        let node = Node::new("test", "Test Node")
            .category("utility")
            .at(Pos2::new(100.0, 200.0))
            .input("input1", DataType::String)
            .output("output1", DataType::String);

        assert_eq!(node.name, "Test Node");
        assert_eq!(node.inputs.len(), 1);
        assert_eq!(node.outputs.len(), 1);
    }
}
