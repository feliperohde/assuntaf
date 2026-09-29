//! Google Analytics 4 over the Measurement Protocol: events are POSTed from the
//! Rust core (no tracking script in the web view).
//! https://developers.google.com/analytics/devguides/collection/protocol/ga4

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::time::Duration;

pub const DEFAULT_ENDPOINT: &str = "https://www.google-analytics.com";

/// GA4 limits (names ≤ 40 chars, values ≤ 100 chars, ≤ 25 params per event).
const MAX_NAME: usize = 40;
const MAX_VALUE: usize = 100;
const MAX_PARAMS: usize = 25;
const MAX_USER_PROPERTIES: usize = 25;

pub struct Ga4Client {
    http: reqwest::Client,
    endpoint: String,
    measurement_id: String,
    api_secret: String,
}

/// GA4 names: letters, digits and underscores, starting with a letter.
pub fn ga4_name(raw: &str) -> String {
    let mut name: String = raw
        .trim_start_matches('$')
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    if !name.chars().next().map_or(false, |c| c.is_ascii_alphabetic()) {
        name = format!("e_{name}");
    }
    name.truncate(MAX_NAME);
    name
}

/// Numbers are sent as numbers (usable as metrics), everything else as text.
fn ga4_value(value: &str) -> Value {
    if let Ok(n) = value.parse::<i64>() {
        return json!(n);
    }
    if let Ok(f) = value.parse::<f64>() {
        if f.is_finite() {
            return json!(f);
        }
    }
    json!(value.chars().take(MAX_VALUE).collect::<String>())
}

impl Ga4Client {
    pub fn new(endpoint: Option<&str>, measurement_id: &str, api_secret: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint: endpoint.unwrap_or(DEFAULT_ENDPOINT).trim_end_matches('/').to_string(),
            measurement_id: measurement_id.trim().to_string(),
            api_secret: api_secret.trim().to_string(),
        }
    }

    /// The request body for one event.
    pub fn body(
        client_id: &str,
        event_name: &str,
        params: &HashMap<String, String>,
        session_number: Option<i64>,
        user_properties: &HashMap<String, String>,
    ) -> Value {
        let mut event_params = Map::new();
        // Needed for the event to count as engaged and to be tied to a session
        event_params.insert("engagement_time_msec".into(), json!(100));
        if let Some(session) = session_number {
            event_params.insert("session_id".into(), json!(session));
        }
        let mut keys: Vec<&String> = params.keys().collect();
        keys.sort();
        for key in keys {
            if event_params.len() >= MAX_PARAMS {
                break;
            }
            let name = ga4_name(key);
            // Our own session id is a string; GA4's session_id must stay numeric
            let name = if name == "session_id" { "assunta_session".to_string() } else { name };
            event_params.entry(name).or_insert_with(|| ga4_value(&params[key]));
        }

        let mut props = Map::new();
        let mut keys: Vec<&String> = user_properties.keys().collect();
        keys.sort();
        for key in keys.into_iter().take(MAX_USER_PROPERTIES) {
            props.insert(ga4_name(key), json!({ "value": ga4_value(&user_properties[key]) }));
        }

        let mut body = json!({
            "client_id": client_id,
            "events": [{ "name": ga4_name(event_name), "params": event_params }],
        });
        if !props.is_empty() {
            body["user_properties"] = Value::Object(props);
        }
        body
    }

    pub async fn send(
        &self,
        client_id: &str,
        event_name: &str,
        params: &HashMap<String, String>,
        session_number: Option<i64>,
        user_properties: &HashMap<String, String>,
    ) -> Result<(), String> {
        let url = format!(
            "{}/mp/collect?measurement_id={}&api_secret={}",
            self.endpoint, self.measurement_id, self.api_secret
        );
        let response = self
            .http
            .post(url)
            .timeout(Duration::from_secs(5))
            .json(&Self::body(client_id, event_name, params, session_number, user_properties))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        // The collect endpoint answers 2xx even for malformed events (use /debug/mp/collect to validate)
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("GA4 returned HTTP {}", response.status()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_ga4_rules() {
        assert_eq!(ga4_name("page_view_home"), "page_view_home");
        assert_eq!(ga4_name("$identify"), "identify");
        assert_eq!(ga4_name("button-click.copy"), "button_click_copy");
        assert_eq!(ga4_name("1st"), "e_1st");
        assert_eq!(ga4_name(&"x".repeat(60)).len(), 40);
    }

    #[test]
    fn body_has_client_session_params_and_user_properties() {
        let mut params = HashMap::new();
        params.insert("duration_seconds".to_string(), "125".to_string());
        params.insert("model_name".to_string(), "parakeet".to_string());
        params.insert("session_id".to_string(), "session_abc".to_string());
        let mut user = HashMap::new();
        user.insert("platform".to_string(), "macos".to_string());

        let body = Ga4Client::body("user-1", "recording-stopped", &params, Some(1_700_000_000), &user);
        assert_eq!(body["client_id"], "user-1");
        let event = &body["events"][0];
        assert_eq!(event["name"], "recording_stopped");
        assert_eq!(event["params"]["duration_seconds"], 125);
        assert_eq!(event["params"]["model_name"], "parakeet");
        assert_eq!(event["params"]["session_id"], 1_700_000_000i64);
        assert_eq!(event["params"]["assunta_session"], "session_abc");
        assert_eq!(event["params"]["engagement_time_msec"], 100);
        assert_eq!(body["user_properties"]["platform"]["value"], "macos");

        let many: HashMap<String, String> = (0..40).map(|i| (format!("p{i}"), "v".to_string())).collect();
        let body = Ga4Client::body("u", "e", &many, None, &HashMap::new());
        assert_eq!(body["events"][0]["params"].as_object().unwrap().len(), MAX_PARAMS);
        assert!(body.get("user_properties").is_none());
    }
}
