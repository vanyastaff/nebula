//! UI components for nebula-ui.
//!
//! This module provides reusable UI components built on egui,
//! following consistent design patterns and theming.
//!
//! ## Component Categories
//!
//! - **Buttons**: [`Button`], [`IconButton`], [`Toggle`], [`IconToggle`]
//! - **Inputs**: [`TextInput`], [`NumberInput`], [`TextArea`], [`Slider`], [`ColorPicker`]
//! - **Containers**: [`Card`], [`Badge`], [`Accordion`], [`Collapsible`], [`Tabs`], [`Carousel`]
//! - **Overlays**: [`Dialog`], [`AlertDialog`], [`Tooltip`], [`Popover`], [`Sheet`], [`HoverCard`]
//! - **Selection**: [`Select`], [`Checkbox`], [`Switch`], [`RadioGroup`], [`Radio`]
//! - **Feedback**: [`Spinner`], [`ProgressBar`], [`Progress`], [`Skeleton`], [`Alert`], [`Toast`], [`Toaster`]
//! - **Navigation**: [`Breadcrumb`], [`ContextMenu`], [`DropdownMenu`], [`CommandPalette`]
//! - **Layout**: [`ScrollArea`], [`Table`], [`ResizablePanels`], [`ResizableBox`]

// UI components naturally have deeper nesting due to egui's builder pattern and callbacks
#![allow(clippy::excessive_nesting)]
//! - **Data**: [`DataTable`], [`LineChart`], [`BarChart`], [`PieChart`], [`Sparkline`]
//! - **Display**: [`Avatar`], [`Label`], [`EmptyState`], [`Calendar`]
//!
//! ## Usage Pattern
//!
//! All components follow a builder pattern and implement the egui `Widget` trait:
//!
//! ```rust,ignore
//! use nebula_ui::components::Button;
//!
//! // Using Widget trait
//! if ui.add(Button::new("Click me").primary()).clicked() {
//!     println!("Clicked!");
//! }
//!
//! // Or using show method
//! if Button::new("Click me")
//!     .primary()
//!     .show(ui)
//!     .clicked()
//! {
//!     println!("Clicked!");
//! }
//! ```

mod accordion;
mod alert;
mod avatar;
mod badge;
mod breadcrumb;
mod button;
mod calendar;
mod card;
mod carousel;
mod chart;
mod checkbox;
mod collapsible;
mod color_picker;
mod command_palette;
mod context_menu;
mod data_table;
mod dialog;
mod dropdown_menu;
mod empty_state;
mod hover_card;
mod input;
mod label;
mod popover;
mod progress;
mod radio_group;
mod resizable;
mod scroll_area;
mod select;
mod separator;
mod sheet;
mod skeleton;
mod slider;
mod spinner;
mod table;
mod tabs;
mod toast;
mod toggle;
mod tooltip;

pub use accordion::{Accordion, AccordionItem};
pub use alert::{Alert, AlertVariant};
pub use avatar::{Avatar, AvatarShape, AvatarSize, AvatarStatus};
pub use badge::{Badge, BadgeVariant};
pub use breadcrumb::{Breadcrumb, BreadcrumbItem};
pub use button::{Button, ButtonSize, ButtonVariant, IconButton};
pub use calendar::{Calendar, CalendarMode, CalendarResponse, DateRangePicker, DateRangeResponse};
pub use card::Card;
pub use carousel::{Carousel, CarouselOrientation, CarouselResponse, ImageCarousel, ImageFit};
pub use chart::{BarChart, DataPoint, LineChart, PieChart, Series, Sparkline};
pub use checkbox::{Checkbox, Switch};
pub use collapsible::Collapsible;
pub use color_picker::{ColorPicker, ColorPickerSize};
pub use command_palette::{CommandItem, CommandPalette, CommandPaletteResponse, QuickActions};
pub use context_menu::{ContextMenuItem, ContextSubMenu, context_menu, context_menu_separator};
pub use data_table::{
    ColumnAlign, DataColumn, DataTable, DataTableResponse, DataTableState, SortDirection,
};
pub use dialog::{AlertDialog, Dialog, DialogResponse};
pub use dropdown_menu::{DropdownMenu, DropdownMenuItem};
pub use empty_state::EmptyState;
pub use hover_card::HoverCard;
pub use input::{NumberInput, TextArea, TextInput};
pub use label::Label;
pub use popover::{Popover, PopoverPlacement};
pub use progress::{CircularProgress, Progress, ProgressSize};
pub use radio_group::{Radio, RadioGroup, RadioOption, RadioOrientation};
pub use resizable::{
    Panel, ResizableBox, ResizablePanels, ResizeDirection, ResizeEdges, ResizeHandle,
};
pub use scroll_area::{ScrollArea, ScrollDirection};
pub use select::{Select, SelectOption};
pub use separator::Separator;
pub use sheet::{Sheet, SheetSide};
pub use skeleton::{Skeleton, SkeletonGroup, SkeletonShape};
pub use slider::{RangeSlider, Slider, SliderOrientation};
pub use spinner::{ProgressBar, Spinner};
pub use table::{RowBuilder, Table, TableAlign, TableBuilder, TableColumn, TableSort};
pub use tabs::{Tab, Tabs, TabsVariant};
pub use toast::{
    Toast, ToastAction, ToastPosition, ToastPromise, ToastVariant, Toaster, ToasterResponse,
};
pub use toggle::{IconToggle, Toggle, ToggleGroup, ToggleGroupItem, ToggleSize};
pub use tooltip::Tooltip;

/// Prelude for common component imports
pub mod prelude {
    pub use super::{
        Accordion, AccordionItem, Alert, AlertDialog, AlertVariant, Avatar, AvatarShape,
        AvatarSize, AvatarStatus, Badge, BadgeVariant, BarChart, Breadcrumb, BreadcrumbItem,
        Button, ButtonSize, ButtonVariant, Calendar, CalendarMode, CalendarResponse, Card,
        Carousel, CarouselOrientation, CarouselResponse, Checkbox, CircularProgress, Collapsible,
        ColorPicker, ColorPickerSize, ColumnAlign, CommandItem, CommandPalette,
        CommandPaletteResponse, ContextMenuItem, ContextSubMenu, DataColumn, DataPoint, DataTable,
        DataTableResponse, DataTableState, DateRangePicker, DateRangeResponse, Dialog,
        DialogResponse, DropdownMenu, DropdownMenuItem, EmptyState, HoverCard, IconButton,
        IconToggle, ImageCarousel, ImageFit, Label, LineChart, NumberInput, Panel, PieChart,
        Popover, PopoverPlacement, Progress, ProgressBar, ProgressSize, QuickActions, Radio,
        RadioGroup, RadioOption, RadioOrientation, RangeSlider, ResizableBox, ResizablePanels,
        ResizeDirection, ResizeEdges, ResizeHandle, RowBuilder, ScrollArea, ScrollDirection,
        Select, SelectOption, Separator, Series, Sheet, SheetSide, Skeleton, SkeletonGroup,
        SkeletonShape, Slider, SliderOrientation, SortDirection, Sparkline, Spinner, Switch, Tab,
        Table, TableAlign, TableBuilder, TableColumn, TableSort, Tabs, TabsVariant, TextArea,
        TextInput, Toast, ToastAction, ToastPosition, ToastPromise, ToastVariant, Toaster,
        ToasterResponse, Toggle, ToggleGroup, ToggleGroupItem, ToggleSize, Tooltip, context_menu,
        context_menu_separator,
    };
}
