//! Renders an action's parameter schema as a form: one control per field kind and widget the schema
//! declares. Rendering never edits the draft; it returns the edits a person made, and the inspector
//! applies them as undoable draft commands.
//!
//! Typed text reaches the draft as it is typed, so closing the panel or selecting another node never
//! loses it. While an input has focus it shows its own text, which may not parse yet; the keystrokes
//! of one focus are one typing session, which the draft keeps as a single undo step. Discrete
//! controls (toggles, options, steppers) commit on the click.
use crate::{
    schema::{
        self, BooleanWidget, Choice, ExpressionMode, Field, Form, Hint, Kind, ListWidget,
        NumberWidget, ObjectWidget, SelectWidget, Severity, Values,
    },
    theme,
    widgets::{self, Tone},
};
use eframe::egui::{self, RichText};
use serde_json::{Map, Value, json};

/// Room a slider leaves for its value box and the gap before it. Rows that measure their own widths
/// must stay inside the panel: anything wider would widen a resizable panel on every frame.
const SLIDER_VALUE_ROOM: f32 = 110.0;
/// Room for a row's trailing remove button.
const ROW_BUTTON_ROOM: f32 = 36.0;

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
    let values = Values::of(form, entries);
    let visible: Vec<&Field> = form
        .fields
        .iter()
        .filter(|field| field.is_visible(&values))
        .collect();
    readiness(ui, &visible, entries, &values);
    let mut edits = Vec::new();
    let mut group: Option<&str> = None;
    for field in visible {
        if field.group.as_deref() != group {
            group = field.group.as_deref();
            if let Some(name) = group {
                section_header(ui, name);
            }
        }
        let required = field.is_required(&values);
        if let Some(edit) = top_field(ui, field, entries.get(&field.key), required, scope, &values)
        {
            edits.push(edit);
        }
        ui.add_space(theme::SPACE_MD);
    }
    edits
}

/// Whether the node is ready to publish as far as the form can tell: every required input visible
/// right now holds a value. Shown first, so a person knows at a glance what is left.
fn readiness(ui: &mut egui::Ui, fields: &[&Field], entries: &Map<String, Value>, values: &Values) {
    let required: Vec<&&Field> = fields
        .iter()
        .filter(|field| !matches!(field.kind, Kind::Notice { .. }) && field.is_required(values))
        .collect();
    if required.is_empty() {
        return;
    }
    let missing: Vec<String> = required
        .iter()
        .filter(|field| match entries.get(&field.key) {
            None => true,
            Some(entry) if entry["type"] == "literal" => schema::is_empty(&entry["value"]),
            Some(entry) if entry["type"] == "expression" => entry["expr"]
                .as_str()
                .is_none_or(|text| text.trim().is_empty()),
            // A template or a reference is set; the server judges it when the node runs.
            Some(_) => false,
        })
        .map(|field| field.title())
        .collect();
    let (tone, text) = match missing.len() {
        0 => (Tone::Success, "All required inputs are set.".to_owned()),
        1 => (
            Tone::Warning,
            format!("{} still needs a value.", missing[0]),
        ),
        count => (
            Tone::Warning,
            format!(
                "{count} required inputs need a value: {}.",
                missing.join(", ")
            ),
        ),
    };
    widgets::banner(ui, tone, &text);
    ui.add_space(theme::SPACE_MD);
}

/// The heading of a group of fields: small capitals over a hairline, with air above it.
fn section_header(ui: &mut egui::Ui, name: &str) {
    ui.add_space(theme::SPACE_SM);
    ui.label(
        RichText::new(name.to_uppercase())
            .size(theme::SIZE_OVERLINE)
            .strong()
            .extra_letter_spacing(1.2)
            .color(theme::TEXT_MUTED),
    );
    let rect = ui.available_rect_before_wrap();
    ui.painter().hline(
        rect.x_range(),
        rect.top(),
        egui::Stroke::new(1.0, theme::BORDER),
    );
    ui.add_space(theme::SPACE_MD);
}

/// A top-level field: its label row with the fixed/expression switch, then its control.
fn top_field(
    ui: &mut egui::Ui,
    field: &Field,
    entry: Option<&Value>,
    required: bool,
    scope: egui::Id,
    values: &Values,
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
    let literal = kind_of_entry == "literal" && !expression;
    let mut edit = None;
    ui.horizontal(|ui| {
        label(ui, field, required);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
            // Read right to left: Expression is added first so Fixed sits on its left, then Reset.
            if field.expression == ExpressionMode::Allowed {
                if widgets::link(ui, "Expression", expression)
                    .on_hover_text("Compute the value when the node runs")
                    .clicked()
                    && !expression
                {
                    let start = entry
                        .filter(|entry| entry["type"] == "literal")
                        .map(|entry| schema::display(&entry["value"]))
                        .unwrap_or_default();
                    edit = Some(FormEdit::Expression(field.key.clone(), start));
                }
                if widgets::link(ui, "Fixed", literal)
                    .on_hover_text("Enter the value itself")
                    .clicked()
                    && !literal
                {
                    edit = Some(fixed(&field.key, field.initial()));
                }
            }
            if entry.is_some()
                && field.expression != ExpressionMode::Required
                && widgets::link(ui, "Reset", false)
                    .on_hover_text("Remove the value, so the action's default applies")
                    .clicked()
            {
                edit = Some(FormEdit::Clear(field.key.clone()));
            }
        });
    });
    if edit.is_some() {
        return edit;
    }
    let result = match kind_of_entry {
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
            let typed = expression_input(ui, id, current);
            widgets::caption(ui, note);
            if required && current.trim().is_empty() {
                needed(ui);
            }
            typed.map(|text| FormEdit::Expression(field.key.clone(), text))
        },
        "literal" => {
            let mut value = entry.map_or_else(|| field.initial(), |entry| entry["value"].clone());
            let changed = control(ui, field, &mut value, id, values);
            // An untouched field is only reminded that it needs a value; checks apply once one is set.
            match (entry, field.problem(&value, required)) {
                (None, Some(_)) if required => needed(ui),
                (Some(_), Some(text)) => problem(ui, &text),
                _ => {
                    if let Some(text) = field.advice(&value) {
                        hint(ui, &text);
                    }
                },
            }
            changed.then(|| fixed(&field.key, value))
        },
        other => {
            widgets::caption(
                ui,
                format!(
                    "This parameter is a {other}, which this form does not edit. Reset it to \
                     enter a value here."
                ),
            );
            None
        },
    };
    // Help reads under the input, where the eye goes after typing.
    if let Some(description) = &field.description {
        hint(ui, description);
    }
    result
}

/// A fixed value for a parameter. An empty value (`null`) is no value: the parameter is removed, since
/// the server rejects a literal null for a typed input and treats an absent one as unset.
fn fixed(key: &str, value: Value) -> FormEdit {
    if value.is_null() {
        FormEdit::Clear(key.to_owned())
    } else {
        FormEdit::Literal(key.to_owned(), value)
    }
}

/// A full-width ghost button in the accent colour, for growing a list. Disabled at the list's limit.
fn add_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> bool {
    let button = egui::Button::new(RichText::new(text).color(theme::ACCENT))
        .frame_when_inactive(false)
        .min_size(egui::vec2(ui.available_width(), 30.0));
    ui.add_enabled(enabled, button)
        .on_disabled_hover_text("The list is at its maximum size")
        .clicked()
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(theme::SIZE_SMALL)
            .color(theme::TEXT_MUTED),
    );
}

/// An expression input: an `fx` mark and an accent-tinted field, so a computed value never passes
/// for a fixed one.
fn expression_input(ui: &mut egui::Ui, id: egui::Id, current: &str) -> Option<String> {
    egui::Frame::new()
        .fill(theme::ACCENT_SOFT)
        .stroke(egui::Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.5)))
        .corner_radius(theme::RADIUS_SM)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("fx").italics().strong().color(theme::ACCENT));
                buffered(ui, id, current, |edit| {
                    edit.font(egui::TextStyle::Monospace)
                        .hint_text("{{ $input.value }}")
                        .frame(egui::Frame::NONE)
                })
            })
            .inner
        })
        .inner
}

fn label(ui: &mut egui::Ui, field: &Field, required: bool) {
    ui.label(RichText::new(field.title()).strong());
    if required {
        ui.label(RichText::new("*").color(theme::DANGER).strong())
            .on_hover_text("Required");
    }
}

fn problem(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .color(theme::DANGER)
            .size(theme::SIZE_SMALL),
    );
}

/// A required field still waiting for its value: a reminder, not yet an error.
fn needed(ui: &mut egui::Ui) {
    ui.label(
        RichText::new("Needs a value before the workflow is published.")
            .color(theme::WARNING)
            .size(theme::SIZE_SMALL),
    );
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

/// The control for one value. Returns true when the value changed and should be committed. `values`
/// are the node's values from the root, which conditions of nested fields read.
fn control(
    ui: &mut egui::Ui,
    field: &Field,
    value: &mut Value,
    id: egui::Id,
    values: &Values,
) -> bool {
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
        Kind::Object { fields, widget } => object_control(ui, fields, *widget, value, id, values),
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
            values,
        ),
        Kind::Mode {
            variants,
            default_variant: _,
        } => mode_control(ui, variants, value, id, values),
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
            // A free-form value: the field's own description says what shape the action expects.
            if let Some(loader) = loader {
                hint(
                    ui,
                    &format!("The server's “{loader}” loader shapes this input; enter it as JSON."),
                );
            }
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

/// Where the form leaves the typing session of the edit it returns this frame, for the inspector.
const TYPING: &str = "form-typing-session";

/// The typing session of the edit the form returned this frame, if it came from typing. Taking it
/// clears it, so a later click is not mistaken for a keystroke.
pub(crate) fn take_typing_session(ctx: &egui::Context) -> Option<u64> {
    ctx.data_mut(|data| data.remove_temp::<u64>(egui::Id::new(TYPING)))
}

/// A text input that returns its text whenever it changes. While it has focus it shows its own
/// buffer, so text that does not parse yet stays on screen; without focus it shows `current`, so undo
/// and server reads stay visible.
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
    let session_id = id.with("typing-session");
    if response.gained_focus() {
        // Distinct per focus, so typing into the field again later is a new undo step.
        let session = id.with(ui.input(|input| input.time).to_bits()).value();
        ui.data_mut(|data| data.insert_temp(session_id, session));
    }
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(buffer_id, text.clone()));
    } else {
        ui.data_mut(|data| data.remove::<String>(buffer_id));
    }
    if text == current || !(response.changed() || response.lost_focus()) {
        return None;
    }
    if response.has_focus()
        && let Some(session) = ui.data(|data| data.get_temp::<u64>(session_id))
    {
        ui.data_mut(|data| data.insert_temp(egui::Id::new(TYPING), session));
    }
    Some(text)
}

/// A typed tag as the list's item type: a number list takes numbers, so text that is not one is not
/// added. Other items are text.
fn tag_value(item: &Field, text: &str) -> Option<Value> {
    match item.kind {
        Kind::Number { integer: true, .. } => text.parse::<i64>().ok().map(Value::from),
        Kind::Number { .. } => text
            .replace(',', ".")
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number),
        _ => Some(Value::from(text)),
    }
}

/// A box for a new list entry, added when Enter is pressed or focus leaves. Unlike `buffered`, its
/// text is not a value until then.
fn entry_box(ui: &mut egui::Ui, id: egui::Id, hint: &str) -> Option<String> {
    let buffer_id = id.with("buffer");
    let mut text = ui
        .data(|data| data.get_temp::<String>(buffer_id))
        .unwrap_or_default();
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .hint_text(hint)
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(8, 6)),
    );
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(buffer_id, text));
        return None;
    }
    ui.data_mut(|data| data.remove::<String>(buffer_id));
    (response.lost_focus() && !text.trim().is_empty()).then(|| text.trim().to_owned())
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
    let declared_step = step;
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
            // The slider takes the row; egui otherwise draws a short default track.
            // The track takes the row minus the value box beside it and the gap between them.
            ui.spacing_mut().slider_width = (ui.available_width() - SLIDER_VALUE_ROOM).max(80.0);
            let mut slider = egui::Slider::new(&mut number, min..=max);
            // Without a declared step a fractional range stays continuous, so 0–1 is not just 0 or 1.
            if let Some(step) = declared_step {
                slider = slider.step_by(step);
            }
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
                        schema::format_number(number),
                        egui::FontId::proportional(theme::SIZE_LABEL),
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
            let suffix = match widget {
                // The schema names no currency or time unit, so none is invented here.
                NumberWidget::Percent => "%",
                NumberWidget::Bytes => "bytes",
                _ => "",
            };
            let shown = current.map(schema::format_number).unwrap_or_default();
            ui.horizontal(|ui| {
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
                                ui.data_mut(|data| data.remove::<bool>(error_id));
                            },
                            // Half-typed text such as `1e` is not an error until the input is left.
                            Err(_) if ui.memory(|memory| memory.has_focus(id)) => {},
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
    painter.circle_filled(egui::pos2(x, rect.center().y), 7.0, theme::ON_ACCENT);
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
        let current = if known || value.is_null() {
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
                ui.label(RichText::new(text).size(theme::SIZE_BODY));
                remove = ui.small_button("×").on_hover_text("Remove").clicked();
            });
        });
    remove
}

/// Nested fields of an object. Their conditions name paths from the root of the node's values, as the
/// server reads them. Returns true on any change.
fn fields_block(
    ui: &mut egui::Ui,
    fields: &[&Field],
    map: &mut Map<String, Value>,
    id: egui::Id,
    values: &Values,
) -> bool {
    let mut changed = false;
    for field in fields.iter().filter(|field| field.is_visible(values)) {
        if let Kind::Notice { severity } = field.kind {
            notice(ui, field, severity);
            continue;
        }
        let required = field.is_required(values);
        ui.horizontal(|ui| label(ui, field, required));
        let mut value = map
            .get(&field.key)
            .cloned()
            .unwrap_or_else(|| field.initial());
        if control(
            ui,
            field,
            &mut value,
            id.with(("field", &field.key)),
            values,
        ) {
            map.insert(field.key.clone(), value.clone());
            changed = true;
        }
        // An empty required value is a reminder; anything typed that breaks a rule is an error.
        if required && schema::is_empty(&value) {
            needed(ui);
        } else if let Some(text) = field.problem(&value, required) {
            problem(ui, &text);
        } else if let Some(text) = field.advice(&value) {
            hint(ui, &text);
        }
        if let Some(description) = &field.description {
            hint(ui, description);
        }
        ui.add_space(theme::SPACE_SM);
    }
    changed
}

/// A group of inputs that form one value, set off from its neighbours.
fn nested<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    // A quiet card one step lighter than the panel: the group reads as one value without shouting.
    let frame = egui::Frame::new()
        .fill(theme::SURFACE)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(theme::RADIUS_MD)
        .inner_margin(egui::Margin::same(theme::SPACE_MD as i8));
    // Measured before the frame and net of its margins, so the card never widens its panel.
    let width = (ui.available_width() - frame.total_margin().sum().x).max(0.0);
    frame
        .show(ui, |ui| {
            ui.set_width(width);
            add(ui)
        })
        .inner
}

fn object_control(
    ui: &mut egui::Ui,
    fields: &[Field],
    widget: ObjectWidget,
    value: &mut Value,
    id: egui::Id,
    values: &Values,
) -> bool {
    let mut map = value.as_object().cloned().unwrap_or_default();
    let all: Vec<&Field> = fields.iter().collect();
    let changed = match widget {
        ObjectWidget::Inline => nested(ui, |ui| fields_block(ui, &all, &mut map, id, values)),
        ObjectWidget::Collapsed => egui::CollapsingHeader::new(match fields.len() {
            1 => "1 field".to_owned(),
            count => format!("{count} fields"),
        })
        .id_salt(id.with("collapsed"))
        .default_open(false)
        .show(ui, |ui| fields_block(ui, &all, &mut map, id, values))
        .body_returned
        .unwrap_or(false),
        ObjectWidget::Sections => nested(ui, |ui| {
            let mut changed = false;
            for (group, members) in grouped(fields) {
                ui.label(RichText::new(group).strong().color(theme::TEXT_MUTED));
                changed |= fields_block(ui, &members, &mut map, id, values);
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
                nested(ui, |ui| fields_block(ui, members, &mut map, id, values))
            })
        },
        ObjectWidget::PickFields => nested(ui, |ui| {
            let present: Vec<&Field> = fields
                .iter()
                .filter(|field| map.contains_key(&field.key))
                .collect();
            let mut changed = fields_block(ui, &present, &mut map, id, values);
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
    values: &Values,
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
    // Key/value rows need object items to have columns; any other item is listed plainly.
    let widget = if spec.widget == ListWidget::KeyValue && !matches!(item.kind, Kind::Object { .. })
    {
        ListWidget::Plain
    } else {
        spec.widget
    };
    match widget {
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
                && let Some(text) = entry_box(ui, id.with("new"), "Type and press Enter")
                && let Some(entry) = tag_value(item, &text)
            {
                items.push(entry);
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
                    // Columns share the row after the remove button and the gaps between them.
                    let gaps = ui.spacing().item_spacing.x * columns.len() as f32;
                    let width = ((ui.available_width() - ROW_BUTTON_ROOM - gaps)
                        / columns.len().max(1) as f32)
                        .max(40.0);
                    for column in &columns {
                        ui.scope(|ui| {
                            ui.set_width(width);
                            // Each column keeps its own type, so a number column stores numbers.
                            let mut cell = map
                                .get(&column.key)
                                .cloned()
                                .unwrap_or_else(|| column.initial());
                            let cell_id = id.with(("cell", index, &column.key));
                            if control(ui, column, &mut cell, cell_id, values) {
                                map.insert(column.key.clone(), cell);
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
            if add_button(ui, "+ Add row", can_add) {
                items.push(item.initial());
            }
        },
        ListWidget::Plain | ListWidget::Sortable | ListWidget::Accordion => {
            let mut action = None;
            let last = items.len().saturating_sub(1);
            let sortable = spec.widget == ListWidget::Sortable;
            // An item that holds several inputs gets its own block under a header row; a single value
            // shares one row with its buttons.
            let compound = matches!(
                item.kind,
                Kind::Object { .. } | Kind::List { .. } | Kind::Mode { .. } | Kind::Code { .. }
            );
            for (index, entry) in items.iter_mut().enumerate() {
                let row_id = id.with(index);
                let mut body = |ui: &mut egui::Ui| {
                    if compound {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("Item {}", index + 1))
                                    .size(theme::SIZE_SMALL)
                                    .color(theme::TEXT_MUTED),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    // Right to left: remove sits at the edge, the arrows before it.
                                    if let Some(chosen) =
                                        row_buttons(ui, index, last, sortable, can_remove)
                                    {
                                        action = Some(chosen);
                                    }
                                },
                            );
                        });
                        if control(ui, item, entry, row_id, values) {
                            action = action.take().or(Some(ListAction::Edited));
                        }
                        ui.add_space(theme::SPACE_SM);
                        return;
                    }
                    ui.horizontal(|ui| {
                        let room = ROW_BUTTON_ROOM * if sortable { 3.0 } else { 1.0 };
                        let width =
                            (ui.available_width() - room - ui.spacing().item_spacing.x).max(40.0);
                        ui.scope(|ui| {
                            ui.set_width(width);
                            if control(ui, item, entry, row_id, values) {
                                action = action.take().or(Some(ListAction::Edited));
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if let Some(chosen) = row_buttons(ui, index, last, sortable, can_remove)
                            {
                                action = Some(chosen);
                            }
                        });
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
            if add_button(ui, "+ Add item", can_add) {
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

/// Remove, then move down and up, added right to left so they read ↑ ↓ × at the row's end.
fn row_buttons(
    ui: &mut egui::Ui,
    index: usize,
    last: usize,
    sortable: bool,
    can_remove: bool,
) -> Option<ListAction> {
    let mut chosen = ui
        .add_enabled(can_remove, egui::Button::new("×").small())
        .on_hover_text("Remove item")
        .on_disabled_hover_text("The list is at its minimum size")
        .clicked()
        .then_some(ListAction::Remove(index));
    if sortable {
        if ui
            .add_enabled(index < last, egui::Button::new("↓").small())
            .on_hover_text("Move down")
            .clicked()
        {
            chosen = Some(ListAction::Down(index));
        }
        if ui
            .add_enabled(index > 0, egui::Button::new("↑").small())
            .on_hover_text("Move up")
            .clicked()
        {
            chosen = Some(ListAction::Up(index));
        }
    }
    chosen
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
    values: &Values,
) -> bool {
    let current_key = value["mode"].as_str().unwrap_or_default().to_owned();
    let mut chosen = current_key.clone();
    if variants.len() <= 4 {
        let options: Vec<(usize, &str)> = variants
            .iter()
            .enumerate()
            .map(|(index, variant)| (index, variant.label.as_str()))
            .collect();
        let mut index = variants
            .iter()
            .position(|variant| variant.key == chosen)
            .unwrap_or(usize::MAX);
        widgets::segmented(ui, &mut index, &options);
        if let Some(variant) = variants.get(index) {
            chosen.clone_from(&variant.key);
        }
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
    // A variant whose payload is never shown, such as "none", has nothing more to fill in.
    if variant.field.visible == schema::Condition::Never || !variant.field.is_visible(values) {
        return false;
    }
    let mut payload = value["value"].clone();
    if payload.is_null() {
        payload = variant.field.initial();
    }
    let changed = nested(ui, |ui| {
        control(
            ui,
            &variant.field,
            &mut payload,
            id.with(("variant", &variant.key)),
            values,
        )
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
            ui.data_mut(|data| data.remove::<bool>(error_id));
        } else {
            match serde_json::from_str(&text) {
                Ok(parsed) => {
                    *value = parsed;
                    changed = true;
                    ui.data_mut(|data| data.remove::<bool>(error_id));
                },
                // JSON is rarely valid halfway through typing, so it is judged once the input is left.
                Err(_) if ui.memory(|memory| memory.has_focus(id)) => {},
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_fixed_value_removes_the_parameter() {
        assert!(matches!(fixed("count", Value::Null), FormEdit::Clear(key) if key == "count"));
        assert!(matches!(
            fixed("count", json!(0)),
            FormEdit::Literal(key, value) if key == "count" && value == json!(0)
        ));
    }

    #[test]
    fn a_tag_takes_the_list_item_type() {
        let numbers = Form::parse(&json!({"fields": [
            {"type": "number", "key": "item", "integer": true}
        ]}));
        let item = &numbers.fields[0];
        assert_eq!(tag_value(item, "42"), Some(json!(42)));
        assert_eq!(tag_value(item, "many"), None);
        let words = Form::parse(&json!({"fields": [{"type": "string", "key": "item"}]}));
        assert_eq!(tag_value(&words.fields[0], "42"), Some(json!("42")));
    }
}
