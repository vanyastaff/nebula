use nebula_expression::MaybeExpression;
use nebula_value::Value;
// Working parameter types with a new trait system
pub mod checkbox;
pub mod code;
pub mod color;
pub mod date;
pub mod datetime;
pub mod expirable;
pub mod file;
pub mod group;
pub mod hidden;
pub mod list;
pub mod mode;
pub mod multi_select;
pub mod notice;
pub mod number;
pub mod object;
pub mod panel;
pub mod radio;
pub mod resource;
pub mod routing;
pub mod secret;
pub mod select;
pub mod text;
pub mod textarea;
pub mod time;
// Export working types
pub use checkbox::{CheckboxParameter, CheckboxParameterOptions};
pub use code::{CodeLanguage, CodeParameter, CodeParameterOptions};
pub use color::{ColorFormat, ColorParameter, ColorParameterOptions};
pub use date::{DateParameter, DateParameterOptions};
pub use datetime::{DateTimeParameter, DateTimeParameterOptions};
pub use expirable::{ExpirableParameter, ExpirableParameterOptions, ExpirableValue};
pub use file::{FileParameter, FileParameterOptions, FileReference};
pub use group::{GroupField, GroupFieldType, GroupParameter, GroupParameterOptions, GroupValue};
pub use hidden::HiddenParameter;
pub use list::{ListParameter, ListParameterOptions, ListValue};
pub use mode::{ModeItem, ModeParameter, ModeValue};
pub use multi_select::{MultiSelectParameter, MultiSelectParameterOptions};
pub use notice::{NoticeParameter, NoticeParameterOptions, NoticeType};
pub use number::{NumberParameter, NumberParameterOptions};
pub use object::{ObjectParameter, ObjectParameterOptions, ObjectValue};
pub use panel::{Panel, PanelParameter, PanelParameterOptions};
pub use radio::{RadioParameter, RadioParameterOptions};
pub use resource::{
    ResourceContext, ResourceLoader, ResourceParameter, ResourceParameterOptions, ResourceValue,
};
pub use routing::{RoutingParameter, RoutingParameterOptions, RoutingValue};
pub use secret::{SecretParameter, SecretParameterOptions};
pub use select::{SelectParameter, SelectParameterOptions};
pub use text::{TextParameter, TextParameterOptions};
pub use textarea::{TextareaParameter, TextareaParameterOptions};
pub use time::{TimeParameter, TimeParameterOptions};
// TODO: Update these to use new trait system (when needed)
// pub mod credential;
