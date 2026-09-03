use serde::{Deserialize, Serialize};
use ugr_compositor::{UiEvent, UiEventHandler, UiEventResult};
use ugr_runtime::{Runtime, V8Engine};

#[derive(Serialize)]
struct EventTarget<'a> {
    key: &'a str,
    path: &'a [usize],
}

#[derive(Deserialize)]
struct EventResponse {
    #[serde(default)]
    markup: Option<String>,
    #[serde(default)]
    #[serde(rename = "defaultPrevented")]
    default_prevented: bool,
}

/// Adapts compositor input events to the persistent JavaScript DOM runtime.
/// Keeping the bridge stateful preserves event listeners and other JS state
/// across native frames and window events.
pub struct UiEventBridge {
    runtime: Runtime<V8Engine>,
}

impl UiEventBridge {
    pub fn new(runtime: Runtime<V8Engine>) -> Self {
        Self { runtime }
    }

    pub fn into_handler(mut self) -> UiEventHandler {
        Box::new(move |event| self.dispatch(event))
    }

    fn dispatch(&mut self, event: UiEvent) -> Result<UiEventResult, String> {
        let target = event
            .target
            .as_ref()
            .map(|target| {
                serde_json::to_string(&EventTarget {
                    key: &target.key,
                    path: &target.path,
                })
            })
            .transpose()
            .map_err(|error| format!("could not encode UI event target: {error}"))?
            .unwrap_or_else(|| "null".to_owned());
        let event_json = event
            .to_json_string()
            .map_err(|error| format!("could not encode UI event: {error}"))?;
        let encoded = self
            .runtime
            .evaluate(&format!("__ugr_dispatch_ui_event({target}, {event_json})"))
            .map_err(|error| format!("could not dispatch UI event: {error}"))?;
        let value: EventResponse = serde_json::from_str(&encoded)
            .map_err(|error| format!("invalid UI event result: {error}"))?;
        Ok(UiEventResult {
            markup: value.markup.filter(|markup| !markup.is_empty()),
            default_prevented: value.default_prevented,
        })
    }
}
