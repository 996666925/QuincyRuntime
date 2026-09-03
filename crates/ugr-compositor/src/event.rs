use serde::Serialize;
use ugr_ui::UiEventTarget;

#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

impl From<winit::keyboard::ModifiersState> for Modifiers {
    fn from(value: winit::keyboard::ModifiersState) -> Self {
        Self {
            shift: value.shift_key(),
            control: value.control_key(),
            alt: value.alt_key(),
            meta: value.super_key(),
        }
    }
}

/// Native input normalized to the subset of DOM event fields supported by the
/// runtime facade. A missing target dispatches on `document`.
#[derive(Debug, Clone)]
pub struct UiEvent {
    pub kind: &'static str,
    pub target: Option<UiEventTarget>,
    pub client_x: Option<f32>,
    pub client_y: Option<f32>,
    pub button: Option<i16>,
    pub buttons: u16,
    pub detail: u8,
    pub key: Option<String>,
    pub code: Option<String>,
    pub repeat: bool,
    pub data: Option<String>,
    pub input_type: Option<&'static str>,
    pub value: Option<String>,
    pub modifiers: Modifiers,
}

impl UiEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn pointer(
        kind: &'static str,
        target: Option<UiEventTarget>,
        client_x: f32,
        client_y: f32,
        button: Option<i16>,
        buttons: u16,
        detail: u8,
        modifiers: Modifiers,
    ) -> Self {
        Self {
            kind,
            target,
            client_x: Some(client_x),
            client_y: Some(client_y),
            button,
            buttons,
            detail,
            key: None,
            code: None,
            repeat: false,
            data: None,
            input_type: None,
            value: None,
            modifiers,
        }
    }

    pub fn keyboard(
        kind: &'static str,
        target: Option<UiEventTarget>,
        key: String,
        code: String,
        repeat: bool,
        modifiers: Modifiers,
    ) -> Self {
        Self {
            kind,
            target,
            client_x: None,
            client_y: None,
            button: None,
            buttons: 0,
            detail: 0,
            key: Some(key),
            code: Some(code),
            repeat,
            data: None,
            input_type: None,
            value: None,
            modifiers,
        }
    }

    pub fn input(
        kind: &'static str,
        target: Option<UiEventTarget>,
        data: Option<String>,
        input_type: &'static str,
        value: Option<String>,
    ) -> Self {
        Self {
            kind,
            target,
            client_x: None,
            client_y: None,
            button: None,
            buttons: 0,
            detail: 0,
            key: None,
            code: None,
            repeat: false,
            data,
            input_type: Some(input_type),
            value,
            modifiers: Modifiers::default(),
        }
    }

    pub fn focus(kind: &'static str, target: Option<UiEventTarget>) -> Self {
        Self::input(kind, target, None, "", None)
    }

    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&EventPayload {
            event_type: self.kind,
            client_x: self.client_x,
            client_y: self.client_y,
            button: self.button,
            buttons: self.buttons,
            detail: self.detail,
            key: self.key.as_deref(),
            code: self.code.as_deref(),
            repeat: self.repeat,
            data: self.data.as_deref(),
            input_type: self.input_type,
            value: self.value.as_deref(),
            shift_key: self.modifiers.shift,
            ctrl_key: self.modifiers.control,
            alt_key: self.modifiers.alt,
            meta_key: self.modifiers.meta,
            bubbles: true,
            cancelable: true,
        })
    }
}

#[derive(Serialize)]
struct EventPayload<'a> {
    #[serde(rename = "type")]
    event_type: &'static str,
    #[serde(rename = "clientX")]
    client_x: Option<f32>,
    #[serde(rename = "clientY")]
    client_y: Option<f32>,
    button: Option<i16>,
    buttons: u16,
    detail: u8,
    key: Option<&'a str>,
    code: Option<&'a str>,
    repeat: bool,
    data: Option<&'a str>,
    #[serde(rename = "inputType")]
    input_type: Option<&'static str>,
    value: Option<&'a str>,
    #[serde(rename = "shiftKey")]
    shift_key: bool,
    #[serde(rename = "ctrlKey")]
    ctrl_key: bool,
    #[serde(rename = "altKey")]
    alt_key: bool,
    #[serde(rename = "metaKey")]
    meta_key: bool,
    bubbles: bool,
    cancelable: bool,
}

#[derive(Debug, Default)]
pub struct UiEventResult {
    pub markup: Option<String>,
    pub default_prevented: bool,
}

pub type UiEventHandler = Box<dyn FnMut(UiEvent) -> Result<UiEventResult, String>>;
