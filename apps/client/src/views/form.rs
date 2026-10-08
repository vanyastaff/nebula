//! Renders an action's parameter schema as a form: one control per field kind and widget the schema
//! declares. Rendering never edits the draft; it returns the edits a person made, and the inspector
//! applies them as undoable draft commands.
//!
//! Typed text is held in a per-field buffer while the input has focus and committed when it loses
//! focus or Enter is pressed, so a word is one undo step rather than one per keystroke. Discrete
//! controls (toggles, options, steppers) commit on the click.
use crate::{
    schema::{
        self, BooleanWidget, Choice, ExpressionMode, Field, Form, Hint, Kind, ListWidget,
        NumberWidget, ObjectWidget, SelectWidget, Severity,
    },
    theme,
    widgets::{self, Tone},
};
use eframe::egui::{self, Color32, RichText};
use serde_json::{Map, Value, json};

/// A change to one top-level parameter of the node.
pub(crate) enum FormEdit {
    Literal(String, Value),
    Expression(String, String),
    /// Removes the parameter so the action's default applies.
    Clear(String),
}

/// Draws every visible field of `form` for the node whose parameter entries are `entries`.
pub(crate) fn show(
    ui: &mut egui::Ui,
    form: &Form,
    entries: &Map<String, Value>,
    scope: egui::Id,
) -> Vec<FormEdit> {
    // Conditions read the fixed values of the other parameters, or their defaults.
    let siblings: Map<String, Value> = form
        .fields
        .iter()
        .filter_map(|field| {
            let value = match entries.get(&field.key) {
                Some(entry) if entry["type"] == "literal" => entry["value"].clone(),
                Some(_) => return None,
                None => field.initial(),
            };
            Some((field.key.clone(), value))
        })
        .collect();
    let mut edits = Vec::new();
    let mut group: Option<&str> = None;
    for field in form
        .fields
        .iter()
        .filter(|field| field.is_visible(&siblings))
    {
        if field.group.as_deref() != group {
            group = field.group.as_deref();
            if let Some(name) = group {
                ui.add_space(theme::SPACE_SM);
                ui.label(
                    RichText::new(name.to_uppercase())
                        .size(11.0)
                        .color(theme::TEXT_MUTED),
                );
                ui.separator();
            }
        }
        let required = field.is_required(&siblings);
        if let Some(edit) = top_field(ui, field, entries.get(&field.key), required, scope) {
            edits.push(edit);
        }
        ui.add_space(theme::SPACE_MD);
    }
    edits
}

/// A top-level field: its label row with the fixed/expression switch, then its control.
fn top_field(
    ui: &mut egui::Ui,
    field: &Field,
    entry: Option<&Value>,
    required: bool,
    scope: egui::Id,
) -> Option<FormEdit> {
    if let Kind::Notice { severity } = field.kind {
        notice(ui, field, severity);
        return None;
    }
    let id = scope.with(&field.key);
    let kind_of_entry = entry
        .and_then(|entry| entry["type"].as_str())
        .unwrap_or("literal");
    let expression = kind_of_entry == "expression" || field.expression == ExpressionMode::Required;
    let mut edit = None;
    ui.horizontal(|ui| {
        label(ui, field, required);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if entry.is_some()
                && field.expression != ExpressionMode::Required
                && ui
                    .small_button("Reset")
                    .on_hover_text("Remove the value, so the action's default applies")
                    .clicked()
            {
                edit = Some(FormEdit::Clear(field.key.clone()));
            }
            if field.expression == ExpressionMode::Allowed {
                // Read right to left: Expression is added first so Fixed sits on its left.
                if ui.selectable_label(expression, "Expression").clicked() && !expression {
                    let start = entry
                        .filter(|entry| entry["type"] == "literal")
                        .map(|entry| schema::display(&entry["value"]))
                        .unwrap_or_default();
                    edit = Some(FormEdit::Expression(field.key.clone(), start));
                }
                if ui.selectable_label(!expression, "Fixed").clicked() && expression {
                    edit = Some(FormEdit::Literal(field.key.clone(), field.initial()));
                }
            }
        });
    });
    if let Some(description) = &field.description {
        widgets::caption(ui, description.as_str());
    }
    if edit.is_some() {
        return edit;
    }
    match kind_of_entry {
        _ if expression => {
            let current = entry
                .and_then(|entry| entry["expr"].as_str())
                .unwrap_or_default();
            let note = match &field.kind {
                Kind::Computed { returns } => {
                    format!("Evaluated when the node runs; returns a {returns}.")
                },
                _ => "Evaluated when the node runs, for example {{ $input.name }}.".to_owned(),
            };
            let typed = buffered(ui, id, current, |edit| {
                edit.font(egui::TextStyle::Monospace).hint_text("{{ }}")
            });
            widgets::caption(ui, note);
            if required && current.trim().is_empty() {
                problem(ui, "Required.");
            }
            typed.map(|text| FormEdit::Expression(field.key.clone(), text))
        },
        "literal" => {
            let mut value = entry.map_or_else(|| field.initial(), |entry| entry["value"].clone());
            let changed = control(ui, field, &mut value, id);
            if let Some(text) = field.problem(&value, required) {
                problem(ui, &text);
            }
            changed.then(|| FormEdit::Literal(field.key.clone(), value))
        },
        other => {
            widgets::caption(
                ui,
                format!("This parameter is a {other}; edit it as JSON in the Settings tab."),
            );
            None
        },
    }
}

fn label(ui: &mut egui::Ui, field: &Field, required: bool) {
    ui.label(RichText::new(field.title()).strong());
    if required {
        ui.label(RichText::new("*").color(theme::DANGER).strong())
            .on_hover_text("Required");
    }
}

fn problem(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::DANGER).size(12.0));
}

fn notice(ui: &mut egui::Ui, field: &Field, severity: Severity) {
    let tone = match severity {
        Severity::Info => Tone::Accent,
        Severity::Warning => Tone::Warning,
        Severity::Danger => Tone::Danger,
        Severity::Success => Tone::Success,
    };
    let text = match (&field.label, &field.description) {
        (Some(label), Some(description)) => format!("{label}: {description}"),
        (Some(text), None) | (None, Some(text)) => text.clone(),
        (None, None) => field.key.clone(),
    };
    widgets::banner(ui, tone, &text);
}

/// The control for one value. Returns true when the value changed and should be committed.
fn control(ui: &mut egui::Ui, field: &Field, value: &mut Value, id: egui::Id) -> bool {
    match &field.kind {
        Kind::Text { hint, multiline } => text_control(ui, field, *hint, *multiline, value, id),
        Kind::Secret { multiline } => secret_control(ui, field, *multiline, value, id),
        Kind::Number {
            integer,
            widget,
            step,
        } => number_control(ui, field, *integer, *widget, *step, value, id),
        Kind::Boolean { widget } => boolean_control(ui, *widget, value, id),
        Kind::Select {
            options,
            multiple,
            allow_custom,
            searchable,
            widget,
            loader,
        } => select_control(
            ui,
            &SelectSpec {
                options,
                multiple: *multiple,
                allow_custom: *allow_custom,
                searchable: *searchable,
                widget: *widget,
                loader: loader.as_deref(),
                placeholder: field.placeholder.as_deref(),
            },
            value,
            id,
        ),
        Kind::Object { fields, widget } => object_control(ui, fields, *widget, value, id),
        Kind::List {
            item,
            min_items,
            max_items,
            unique,
            widget,
        } => list_control(
            ui,
            item.as_deref(),
            &ListSpec {
                min: min_items.unwrap_or(0),
                max: max_items.unwrap_or(u64::MAX),
                unique: *unique,
                widget: *widget,
            },
            value,
            id,
        ),
        Kind::Mode {
            variants,
            default_variant: _,
        } => mode_control(ui, variants, value, id),
        Kind::Code { language, simple } => code_control(ui, language, *simple, value, id),
        Kind::File {
            accept,
            max_size,
            multiple,
        } => {
            file_note(ui, accept.as_deref(), *max_size, *multiple);
            false
        },
        Kind::Computed { .. } => false,
        Kind::Dynamic { loader } => {
            widgets::caption(
                ui,
                format!(
                    "The server builds this input with its {} loader; enter its value as JSON.",
                    loader.as_deref().unwrap_or("dynamic")
                ),
            );
            json_control(ui, value, id)
        },
        Kind::Notice { .. } => false,
        Kind::Unknown { type_name } => {
            widgets::caption(
                ui,
                format!("A “{type_name}” input is newer than this app; enter its value as JSON."),
            );
            json_control(ui, value, id)
        },
    }
}

/// A text input that keeps what is typed while it has focus and returns it once, when focus leaves
/// or Enter is pressed. Without focus it shows `current`, so undo and server reads stay visible.
fn buffered(
    ui: &mut egui::Ui,
    id: egui::Id,
    current: &str,
    style: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>,
) -> Option<String> {
    buffered_input(ui, id, current, false, style)
}

fn buffered_input(
    ui: &mut egui::Ui,
    id: egui::Id,
    current: &str,
    multiline: bool,
    style: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>,
) -> Option<String> {
    let buffer_id = id.with("buffer");
    let focused = ui.memory(|memory| memory.has_focus(id));
    let mut text = if focused {
        ui.data(|data| data.get_temp::<String>(buffer_id))
            .unwrap_or_else(|| current.to_owned())
    } else {
        current.to_owned()
    };
    let edit = if multiline {
        egui::TextEdit::multiline(&mut text).desired_rows(4)
    } else {
        egui::TextEdit::singleline(&mut text)
    };
    let response = ui
        .add(style(edit.id(id).desired_width(f32::INFINITY)).margin(egui::Margin::symmetric(8, 6)));
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(buffer_id, text.clone()));
        None
    } else {
        ui.data_mut(|data| data.remove::<String>(buffer_id));
        (response.lost_focus() && text != current).then_some(text)
    }
}

fn text_control(
    ui: &mut egui::Ui,
    field: &Field,
    hint: Hint,
    multiline: bool,
    value: &mut Value,
    id: egui::Id,
) -> bool {
    let current = value.as_str().unwrap_or_default().to_owned();
    let placeholder = field
        .placeholder
        .clone()
        .or_else(|| schema::hint_placeholder(hint).map(str::to_owned))
        .unwrap_or_default();
    let mut changed = false;
    ui.horizontal(|ui| {
        if hint == Hint::Color {
            let mut rgb = parse_hex(&current).unwrap_or([99, 102, 241]);
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                *value = Value::from(format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]));
                changed = true;
            }
        }
        let multiline = multiline || hint == Hint::Markdown;
        let typed = buffered_input(ui, id, &current, multiline, |edit| {
            let edit = edit.hint_text(placeholder);
            match hint {
                Hint::Password => edit.password(true),
                Hint::Regex | Hint::Cron | Hint::Uuid => edit.font(egui::TextStyle::Monospace),
                _ => edit,
            }
        });
        if let Some(text) = typed {
            *value = Value::from(text);
            changed = true;
        }
    });
    changed
}

fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range)?, 16).ok();
    Some([channel(0..2)?, channel(2..4)?, channel(4..6)?])
}

fn secret_control(
    ui: &mut egui::Ui,
    field: &Field,
    multiline: bool,
    value: &mut Value,
    id: egui::Id,
) -> bool {
    let reveal_id = id.with("reveal");
    let mut revealed = ui
        .data(|data| data.get_temp::<bool>(reveal_id))
        .unwrap_or(false);
    let current = value.as_str().unwrap_or_default().to_owned();
    let mut changed = false;
    ui.horizontal(|ui| {
        if ui.toggle_value(&mut revealed, "Show").changed() {
            ui.data_mut(|data| data.insert_temp(reveal_id, revealed));
        }
        let placeholder = field.placeholder.clone().unwrap_or_default();
        if let Some(text) = buffered_input(ui, id, &current, multiline, |edit| {
            edit.password(!revealed).hint_text(placeholder)
        }) {
            *value = Value::from(text);
            changed = true;
        }
    });
    widgets::caption(
        ui,
        "Stored in the workflow definition. Prefer a credential for long-lived secrets.",
    );
    changed
}

fn number_control(
    ui: &mut egui::Ui,
    field: &Field,
    integer: bool,
    widget: NumberWidget,
    step: Option<f64>,
    value: &mut Value,
    id: egui::Id,
) -> bool {
    let current = value.as_f64();
    let step = step.unwrap_or(1.0);
    let store = |value: &mut Value, number: f64| {
        *value = if integer {
            Value::from(number.round() as i64)
        } else {
            serde_json::Number::from_f64(number).map_or(Value::Null, Value::Number)
        };
    };
    let mut changed = false;
    match widget {
        NumberWidget::Slider => {
            let min = field.bounds.min.unwrap_or(0.0);
            let max = field.bounds.max.unwrap_or(100.0).max(min);
            // The slider moves a frame-local copy; only the release commits, as one undo step.
            let drag_id = id.with("drag");
            let mut number = ui
                .data(|data| data.get_temp::<f64>(drag_id))
                .or(current)
                .unwrap_or(min);
            let slider = egui::Slider::new(&mut number, min..=max).step_by(step);
            let response = ui.add(if integer { slider.integer() } else { slider });
            if response.dragged() {
                ui.data_mut(|data| data.insert_temp(drag_id, number));
            } else {
                ui.data_mut(|data| data.remove::<f64>(drag_id));
                if (response.drag_stopped() || response.changed()) && Some(number) != current {
                    store(value, number);
                    changed = true;
                }
            }
        },
        NumberWidget::Stepper => {
            ui.horizontal(|ui| {
                let number = current.unwrap_or(0.0);
                let within = |candidate: f64| {
                    field.bounds.min.is_none_or(|min| candidate >= min)
                        && field.bounds.max.is_none_or(|max| candidate <= max)
                };
                if ui
                    .add_enabled(within(number - step), egui::Button::new("−"))
                    .clicked()
                {
                    store(value, number - step);
                    changed = true;
                }
                ui.add_sized([72.0, 28.0], |ui: &mut egui::Ui| {
                    let response =
                        ui.allocate_response(egui::vec2(72.0, 28.0), egui::Sense::hover());
                    ui.painter().text(
                        response.rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format_number(number),
                        egui::FontId::proportional(15.0),
                        theme::TEXT,
                    );
                    response
                });
                if ui
                    .add_enabled(within(number + step), egui::Button::new("+"))
                    .clicked()
                {
                    store(value, number + step);
                    changed = true;
                }
            });
        },
        NumberWidget::Plain
        | NumberWidget::Percent
        | NumberWidget::Currency
        | NumberWidget::Duration
        | NumberWidget::Bytes => {
            let (prefix, suffix) = match widget {
                NumberWidget::Percent => ("", "%"),
                NumberWidget::Currency => ("¤", ""),
                NumberWidget::Duration => ("", "s"),
                NumberWidget::Bytes => ("", "bytes"),
                _ => ("", ""),
            };
            let shown = current.map(format_number).unwrap_or_default();
            ui.horizontal(|ui| {
                if !prefix.is_empty() {
                    ui.label(RichText::new(prefix).color(theme::TEXT_MUTED));
                }
                let error_id = id.with("unparsed");
                // Leave room for the unit after the field.
                ui.scope(|ui| {
                    ui.set_max_width(
                        ui.available_width() - if suffix.is_empty() { 0.0 } else { 52.0 },
                    );
                    if let Some(text) = buffered(ui, id, &shown, |edit| {
                        edit.hint_text(field.placeholder.clone().unwrap_or_default())
                    }) {
                        match text.trim().replace(',', ".").parse::<f64>() {
                            Ok(number) => {
                                store(value, number);
                                changed = true;
                                ui.data_mut(|data| data.remove::<bool>(error_id));
                            },
                            Err(_) if text.trim().is_empty() => {
                                *value = Value::Null;
                                changed = true;
                            },
                            Err(_) => {
                                ui.data_mut(|data| data.insert_temp(error_id, true));
                            },
                        }
                    }
                });
                if !suffix.is_empty() {
                    ui.label(RichText::new(suffix).color(theme::TEXT_MUTED));
                }
                if ui
                    .data(|data| data.get_temp::<bool>(error_id))
                    .unwrap_or(false)
                {
                    problem(ui, "Enter a number.");
                }
            });
            if widget == NumberWidget::Bytes
                && let Some(bytes) = current
            {
                widgets::caption(ui, human_bytes(bytes));
            }
        },
    }
    changed
}

fn format_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{number:.0}")
    } else {
        number.to_string()
    }
}

fn human_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size:.0} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn boolean_control(
    ui: &mut egui::Ui,
    widget: BooleanWidget,
    value: &mut Value,
    id: egui::Id,
) -> bool {
    let mut on = value.as_bool().unwrap_or(false);
    let before = on;
    match widget {
        BooleanWidget::Toggle => {
            toggle(ui, &mut on, id);
        },
        BooleanWidget::Checkbox => {
            let caption = if on { "On" } else { "Off" };
            ui.checkbox(&mut on, caption);
        },
        BooleanWidget::Radio => {
            ui.horizontal(|ui| {
                ui.radio_value(&mut on, true, "Yes");
                ui.radio_value(&mut on, false, "No");
            });
        },
    }
    if on != before {
        *value = Value::Bool(on);
        return true;
    }
    false
}

/// A switch with a sliding knob, the shape people expect for on/off settings.
fn toggle(ui: &mut egui::Ui, on: &mut bool, id: egui::Id) {
    let size = egui::vec2(36.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let position = ui.ctx().animate_bool_responsive(id.with("toggle"), *on);
    let painter = ui.painter();
    let track = if *on { theme::ACCENT } else { theme::FIELD };
    painter.rect(
        rect,
        egui::CornerRadius::same(10),
        track,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), position);
    painter.circle_filled(egui::pos2(x, rect.center().y), 7.0, Color32::WHITE);
}

struct SelectSpec<'a> {
    options: &'a [Choice],
    multiple: bool,
    allow_custom: bool,
    searchable: bool,
    widget: SelectWidget,
    loader: Option<&'a str>,
    placeholder: Option<&'a str>,
}

fn choice_label(options: &[Choice], value: &Value) -> String {
    options
        .iter()
        .find(|choice| &choice.value == value)
        .map_or_else(|| schema::display(value), |choice| choice.label.clone())
}

fn select_control(
    ui: &mut egui::Ui,
    spec: &SelectSpec<'_>,
    value: &mut Value,
    id: egui::Id,
) -> bool {
    if let Some(loader) = spec.loader
        && spec.options.is_empty()
    {
        widgets::caption(
            ui,
            format!(
                "Options come from the server's “{loader}” loader, which the editor cannot call yet. Enter the value."
            ),
        );
        return json_control(ui, value, id);
    }
    if spec.multiple {
        return multi_select(ui, spec, value, id);
    }
    let mut changed = false;
    match spec.widget {
        SelectWidget::Radio => {
            for choice in spec.options {
                let selected = &choice.value == value;
                let radio = ui.add_enabled(
                    !choice.disabled,
                    egui::RadioButton::new(selected, &choice.label),
                );
                if let Some(description) = &choice.description {
                    radio.clone().on_hover_text(description);
                }
                if radio.clicked() && !selected {
                    *value = choice.value.clone();
                    changed = true;
                }
            }
        },
        SelectWidget::Dropdown
        | SelectWidget::Combobox
        | SelectWidget::Checkboxes
        | SelectWidget::Tags => {
            let searchable = spec.searchable || spec.widget == SelectWidget::Combobox;
            let selected_text = if value.is_null() {
                spec.placeholder.unwrap_or("Choose…").to_owned()
            } else {
                choice_label(spec.options, value)
            };
            let search_id = id.with("search");
            egui::ComboBox::from_id_salt(id)
                .selected_text(selected_text)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    let mut filter = String::new();
                    if searchable {
                        filter = ui
                            .data(|data| data.get_temp::<String>(search_id))
                            .unwrap_or_default();
                        let response =
                            ui.add(egui::TextEdit::singleline(&mut filter).hint_text("Search"));
                        response.request_focus();
                        ui.data_mut(|data| data.insert_temp(search_id, filter.clone()));
                    }
                    let needle = filter.to_lowercase();
                    for choice in spec.options.iter().filter(|choice| {
                        needle.is_empty() || choice.label.to_lowercase().contains(&needle)
                    }) {
                        let selected = &choice.value == value;
                        let row = ui.add_enabled(
                            !choice.disabled,
                            egui::Button::selectable(selected, &choice.label),
                        );
                        let row = match &choice.description {
                            Some(description) => row.on_hover_text(description),
                            None => row,
                        };
                        if row.clicked() && !selected {
                            *value = choice.value.clone();
                            changed = true;
                        }
                    }
                });
        },
    }
    if spec.allow_custom {
        let known = spec.options.iter().any(|choice| &choice.value == value);
        let current = if known {
            String::new()
        } else {
            schema::display(value)
        };
        widgets::caption(ui, "Or enter another value:");
        if let Some(text) = buffered(ui, id.with("custom"), &current, |edit| {
            edit.hint_text("Custom value")
        }) {
            *value = Value::from(text);
            changed = true;
        }
    }
    changed
}

fn multi_select(ui: &mut egui::Ui, spec: &SelectSpec<'_>, value: &mut Value, id: egui::Id) -> bool {
    let mut chosen: Vec<Value> = value.as_array().cloned().unwrap_or_default();
    let before = chosen.clone();
    match spec.widget {
        SelectWidget::Tags => {
            ui.horizontal_wrapped(|ui| {
                let mut removed = None;
                for (index, item) in chosen.iter().enumerate() {
                    if chip(ui, &choice_label(spec.options, item)) {
                        removed = Some(index);
                    }
                }
                if let Some(index) = removed {
                    chosen.remove(index);
                }
                let left: Vec<&Choice> = spec
                    .options
                    .iter()
                    .filter(|choice| !choice.disabled && !chosen.contains(&choice.value))
                    .collect();
                if !left.is_empty() {
                    egui::ComboBox::from_id_salt(id.with("add"))
                        .selected_text("+ Add")
                        .show_ui(ui, |ui| {
                            for choice in left {
                                if ui.selectable_label(false, &choice.label).clicked() {
                                    chosen.push(choice.value.clone());
                                }
                            }
                        });
                }
            });
        },
        _ => {
            for choice in spec.options {
                let mut on = chosen.contains(&choice.value);
                let checkbox = ui.add_enabled(
                    !choice.disabled,
                    egui::Checkbox::new(&mut on, &choice.label),
                );
                if checkbox.changed() {
                    if on {
                        chosen.push(choice.value.clone());
                    } else {
                        chosen.retain(|item| item != &choice.value);
                    }
                }
            }
        },
    }
    if chosen != before {
        *value = Value::Array(chosen);
        return true;
    }
    false
}

/// A removable chip. Returns true when its × was clicked.
fn chip(ui: &mut egui::Ui, text: &str) -> bool {
    let mut remove = false;
    egui::Frame::new()
        .fill(theme::ACCENT_SOFT)
        .corner_radius(theme::RADIUS_SM)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
                ui.label(RichText::new(text).size(13.0));
                remove = ui.small_button("×").on_hover_text("Remove").clicked();
            });
        });
    remove
}

/// Nested fields of an object, each against the object's own values. Returns true on any change.
fn fields_block(
    ui: &mut egui::Ui,
    fields: &[&Field],
    map: &mut Map<String, Value>,
    id: egui::Id,
) -> bool {
    let siblings = map.clone();
    let mut changed = false;
    for field in fields.iter().filter(|field| field.is_visible(&siblings)) {
        if let Kind::Notice { severity } = field.kind {
            notice(ui, field, severity);
            continue;
        }
        let required = field.is_required(&siblings);
        ui.horizontal(|ui| label(ui, field, required));
        if let Some(description) = &field.description {
            widgets::caption(ui, description.as_str());
        }
        let mut value = map
            .get(&field.key)
            .cloned()
            .unwrap_or_else(|| field.initial());
        if control(ui, field, &mut value, id.with(&field.key)) {
            map.insert(field.key.clone(), value.clone());
            changed = true;
        }
        if let Some(text) = field.problem(&value, required) {
            problem(ui, &text);
        }
        ui.add_space(theme::SPACE_SM);
    }
    changed
}

/// An indented block with a rule on its left, so nesting reads at a glance.
fn nested<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let inner = ui.horizontal(|ui| {
        ui.add_space(theme::SPACE_XS);
        ui.vertical(|ui| add(ui)).inner
    });
    let rect = inner.response.rect;
    ui.painter().vline(
        rect.left(),
        rect.y_range(),
        egui::Stroke::new(2.0, theme::BORDER),
    );
    inner.inner
}

fn object_control(
    ui: &mut egui::Ui,
    fields: &[Field],
    widget: ObjectWidget,
    value: &mut Value,
    id: egui::Id,
) -> bool {
    let mut map = value.as_object().cloned().unwrap_or_default();
    let all: Vec<&Field> = fields.iter().collect();
    let changed = match widget {
        ObjectWidget::Inline => nested(ui, |ui| fields_block(ui, &all, &mut map, id)),
        ObjectWidget::Collapsed => egui::CollapsingHeader::new(format!("{} fields", fields.len()))
            .id_salt(id.with("collapsed"))
            .default_open(false)
            .show(ui, |ui| fields_block(ui, &all, &mut map, id))
            .body_returned
            .unwrap_or(false),
        ObjectWidget::Sections => nested(ui, |ui| {
            let mut changed = false;
            for (group, members) in grouped(fields) {
                ui.label(RichText::new(group).strong().color(theme::TEXT_MUTED));
                changed |= fields_block(ui, &members, &mut map, id);
            }
            changed
        }),
        ObjectWidget::Tabs => {
            let groups = grouped(fields);
            let tab_id = id.with("tab");
            let mut tab = ui
                .data(|data| data.get_temp::<usize>(tab_id))
                .unwrap_or(0)
                .min(groups.len().saturating_sub(1));
            ui.horizontal_wrapped(|ui| {
                for (index, (group, _)) in groups.iter().enumerate() {
                    if ui.selectable_label(tab == index, group.as_str()).clicked() {
                        tab = index;
                    }
                }
            });
            ui.data_mut(|data| data.insert_temp(tab_id, tab));
            groups.get(tab).is_some_and(|(_, members)| {
                nested(ui, |ui| fields_block(ui, members, &mut map, id))
            })
        },
        ObjectWidget::PickFields => nested(ui, |ui| {
            let present: Vec<&Field> = fields
                .iter()
                .filter(|field| map.contains_key(&field.key))
                .collect();
            let mut changed = fields_block(ui, &present, &mut map, id);
            let mut removed = None;
            if !present.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    widgets::caption(ui, "Remove:");
                    for field in &present {
                        if ui.small_button(field.title()).clicked() {
                            removed = Some(field.key.clone());
                        }
                    }
                });
            }
            if let Some(key) = removed {
                map.remove(&key);
                changed = true;
            }
            let absent: Vec<&Field> = fields
                .iter()
                .filter(|field| !map.contains_key(&field.key))
                .collect();
            if !absent.is_empty() {
                egui::ComboBox::from_id_salt(id.with("pick"))
                    .selected_text("+ Add field")
                    .show_ui(ui, |ui| {
                        for field in absent {
                            if ui.selectable_label(false, field.title()).clicked() {
                                map.insert(field.key.clone(), field.initial());
                                changed = true;
                            }
                        }
                    });
            }
            changed
        }),
    };
    if changed {
        *value = Value::Object(map);
    }
    changed
}

/// Fields by their `group`, in the order groups first appear; ungrouped fields form "General".
fn grouped(fields: &[Field]) -> Vec<(String, Vec<&Field>)> {
    let mut groups: Vec<(String, Vec<&Field>)> = Vec::new();
    for field in fields {
        let name = field.group.clone().unwrap_or_else(|| "General".to_owned());
        match groups.iter_mut().find(|(group, _)| *group == name) {
            Some((_, members)) => members.push(field),
            None => groups.push((name, vec![field])),
        }
    }
    groups
}

struct ListSpec {
    min: u64,
    max: u64,
    unique: bool,
    widget: ListWidget,
}

fn list_control(
    ui: &mut egui::Ui,
    item: Option<&Field>,
    spec: &ListSpec,
    value: &mut Value,
    id: egui::Id,
) -> bool {
    let Some(item) = item else {
        widgets::caption(ui, "This list declares no item shape; enter it as JSON.");
        return json_control(ui, value, id);
    };
    let mut items: Vec<Value> = value.as_array().cloned().unwrap_or_default();
    let before = items.clone();
    let count = items.len() as u64;
    let can_remove = count > spec.min;
    let can_add = count < spec.max;
    match spec.widget {
        ListWidget::Tags => {
            ui.horizontal_wrapped(|ui| {
                let mut removed = None;
                for (index, entry) in items.iter().enumerate() {
                    if chip(ui, &schema::display(entry)) && can_remove {
                        removed = Some(index);
                    }
                }
                if let Some(index) = removed {
                    items.remove(index);
                }
            });
            if can_add
                && let Some(text) = buffered(ui, id.with("new"), "", |edit| {
                    edit.hint_text("Type and press Enter")
                })
                && !text.trim().is_empty()
            {
                items.push(Value::from(text.trim()));
            }
        },
        ListWidget::KeyValue => {
            let columns: Vec<&Field> = match &item.kind {
                Kind::Object { fields, .. } => fields.iter().collect(),
                _ => Vec::new(),
            };
            let mut removed = None;
            for (index, entry) in items.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let mut map = entry.as_object().cloned().unwrap_or_default();
                    let width = (ui.available_width() - 40.0) / columns.len().max(1) as f32;
                    for column in &columns {
                        ui.scope(|ui| {
                            ui.set_width(width);
                            let current = map
                                .get(&column.key)
                                .map(schema::display)
                                .unwrap_or_default();
                            if let Some(text) =
                                buffered(ui, id.with((index, &column.key)), &current, |edit| {
                                    edit.hint_text(column.title())
                                })
                            {
                                map.insert(column.key.clone(), Value::from(text));
                                *entry = Value::Object(map.clone());
                            }
                        });
                    }
                    if ui
                        .add_enabled(can_remove, egui::Button::new("×"))
                        .on_hover_text("Remove row")
                        .clicked()
                    {
                        removed = Some(index);
                    }
                });
            }
            if let Some(index) = removed {
                items.remove(index);
            }
            if ui
                .add_enabled(can_add, egui::Button::new("+ Add row"))
                .clicked()
            {
                items.push(item.initial());
            }
        },
        ListWidget::Plain | ListWidget::Sortable | ListWidget::Accordion => {
            let mut action = None;
            let last = items.len().saturating_sub(1);
            for (index, entry) in items.iter_mut().enumerate() {
                let row_id = id.with(index);
                let mut body = |ui: &mut egui::Ui| {
                    ui.horizontal(|ui| {
                        if spec.widget == ListWidget::Sortable {
                            if ui
                                .add_enabled(index > 0, egui::Button::new("↑").small())
                                .clicked()
                            {
                                action = Some(ListAction::Up(index));
                            }
                            if ui
                                .add_enabled(index < last, egui::Button::new("↓").small())
                                .clicked()
                            {
                                action = Some(ListAction::Down(index));
                            }
                        }
                        let width = ui.available_width() - 36.0;
                        ui.scope(|ui| {
                            ui.set_width(width);
                            if control(ui, item, entry, row_id) {
                                action = action.take().or(Some(ListAction::Edited));
                            }
                        });
                        if ui
                            .add_enabled(can_remove, egui::Button::new("×"))
                            .on_hover_text("Remove item")
                            .clicked()
                        {
                            action = Some(ListAction::Remove(index));
                        }
                    });
                };
                if spec.widget == ListWidget::Accordion {
                    egui::CollapsingHeader::new(format!("Item {}", index + 1))
                        .id_salt(row_id.with("accordion"))
                        .show(ui, |ui| body(ui));
                } else {
                    body(ui);
                }
            }
            match action {
                Some(ListAction::Remove(index)) => {
                    items.remove(index);
                },
                Some(ListAction::Up(index)) => items.swap(index, index - 1),
                Some(ListAction::Down(index)) => items.swap(index, index + 1),
                Some(ListAction::Edited) | None => {},
            }
            if ui
                .add_enabled(can_add, egui::Button::new("+ Add item"))
                .clicked()
            {
                items.push(item.initial());
            }
        },
    }
    if spec.unique {
        let duplicated = items
            .iter()
            .enumerate()
            .any(|(index, entry)| items[..index].contains(entry));
        if duplicated {
            problem(ui, "Items must be unique.");
        }
    }
    if items != before {
        *value = Value::Array(items);
        return true;
    }
    false
}

enum ListAction {
    Edited,
    Remove(usize),
    Up(usize),
    Down(usize),
}

fn mode_control(
    ui: &mut egui::Ui,
    variants: &[schema::Variant],
    value: &mut Value,
    id: egui::Id,
) -> bool {
    let current_key = value["mode"].as_str().unwrap_or_default().to_owned();
    let mut chosen = current_key.clone();
    if variants.len() <= 4 {
        ui.horizontal_wrapped(|ui| {
            for variant in variants {
                if ui
                    .selectable_label(chosen == variant.key, &variant.label)
                    .clicked()
                {
                    chosen.clone_from(&variant.key);
                }
            }
        });
    } else {
        let selected = variants
            .iter()
            .find(|variant| variant.key == chosen)
            .map_or("Choose…", |variant| variant.label.as_str());
        egui::ComboBox::from_id_salt(id.with("mode"))
            .selected_text(selected)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for variant in variants {
                    ui.selectable_value(&mut chosen, variant.key.clone(), &variant.label);
                }
            });
    }
    if chosen != current_key {
        let payload = variants
            .iter()
            .find(|variant| variant.key == chosen)
            .map_or(Value::Null, |variant| variant.field.initial());
        *value = json!({"mode": chosen, "value": payload});
        return true;
    }
    let Some(variant) = variants.iter().find(|variant| variant.key == current_key) else {
        return false;
    };
    // A variant without a visible payload, such as "none", has nothing more to fill in.
    if !variant.field.is_visible(&Map::new()) {
        return false;
    }
    let mut payload = value["value"].clone();
    if payload.is_null() {
        payload = variant.field.initial();
    }
    let changed = nested(ui, |ui| {
        control(ui, &variant.field, &mut payload, id.with(&variant.key))
    });
    if changed {
        *value = json!({"mode": current_key, "value": payload});
    }
    changed
}

fn code_control(
    ui: &mut egui::Ui,
    language: &str,
    simple: bool,
    value: &mut Value,
    id: egui::Id,
) -> bool {
    widgets::badge(ui, language, Tone::Neutral);
    let current = value.as_str().unwrap_or_default().to_owned();
    let rows = if simple { 4 } else { 10 };
    let typed = buffered_input(ui, id, &current, true, |edit| {
        edit.code_editor().desired_rows(rows)
    });
    typed.is_some_and(|text| {
        *value = Value::from(text);
        true
    })
}

fn file_note(ui: &mut egui::Ui, accept: Option<&str>, max_size: Option<u64>, multiple: bool) {
    let mut limits = Vec::new();
    if let Some(accept) = accept {
        limits.push(format!("accepts {accept}"));
    }
    if let Some(bytes) = max_size {
        limits.push(format!("up to {}", human_bytes(bytes as f64)));
    }
    if multiple {
        limits.push("several files".to_owned());
    }
    widgets::caption(
        ui,
        "The workflow API takes no file uploads yet. Switch to Expression and point it at a file from an earlier node.",
    );
    if !limits.is_empty() {
        widgets::caption(ui, limits.join(" · "));
    }
}

/// Any value as JSON text, for inputs the form cannot draw natively.
fn json_control(ui: &mut egui::Ui, value: &mut Value, id: egui::Id) -> bool {
    let current = if value.is_null() {
        String::new()
    } else {
        serde_json::to_string_pretty(value).unwrap_or_default()
    };
    let error_id = id.with("invalid");
    let typed = buffered_input(ui, id, &current, true, |edit| {
        edit.code_editor().desired_rows(3)
    });
    let mut changed = false;
    if let Some(text) = typed {
        if text.trim().is_empty() {
            *value = Value::Null;
            changed = true;
        } else {
            match serde_json::from_str(&text) {
                Ok(parsed) => {
                    *value = parsed;
                    changed = true;
                    ui.data_mut(|data| data.remove::<bool>(error_id));
                },
                Err(_) => {
                    ui.data_mut(|data| data.insert_temp(error_id, true));
                },
            }
        }
    }
    if ui
        .data(|data| data.get_temp::<bool>(error_id))
        .unwrap_or(false)
    {
        problem(ui, "Not valid JSON; the last valid value is kept.");
    }
    changed
}
