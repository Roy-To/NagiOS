use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use nagi_home_search::actions::{ActionAvailability, CapabilityContext, TypedAction};
use nagi_home_search::fixtures::create_preview_controller;
use nagi_home_search::home::{ContinuationReference, HomeApp, HomeQuickAction, HomeSnapshot};
use nagi_home_search::localization::{Locale, LocalizationCatalog};
use nagi_home_search::registry::AppAvailability;
use nagi_home_search::search::{
    ProviderIssueKind, SearchContext, SearchIdentity, SearchQuery, SearchRequestId, SearchResponse,
    SearchResult,
};
use nagi_home_search::HomeController;
use serde::Serialize;

const HTML: &str = include_str!("../../preview/index.html");
const CSS: &str = include_str!("../../preview/app.css");
const JS: &str = include_str!("../../preview/app.js");
const MAX_REQUEST_LINE_BYTES: usize = 16 * 1024;

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 4173))?;
    let controller = create_preview_controller();
    println!("Home + Search host preview: http://127.0.0.1:4173/");
    println!("Preview data is in-memory fixture data; no Nagi or host user data is read.");
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let controller = Arc::clone(&controller);
                let _ = thread::Builder::new()
                    .name("nagi-home-preview-http".to_owned())
                    .spawn(move || handle_connection(stream, controller));
            }
            Err(error) => eprintln!("preview accept failed: {error}"),
        }
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream, controller: Arc<HomeController>) {
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let reader = BufReader::new(clone);
    let mut request_line = String::new();
    if reader
        .take((MAX_REQUEST_LINE_BYTES + 1) as u64)
        .read_line(&mut request_line)
        .is_err()
        || request_line.len() > MAX_REQUEST_LINE_BYTES
    {
        respond(
            &mut stream,
            414,
            "application/json; charset=utf-8",
            "{\"error\":\"request_too_large\"}",
        );
        return;
    }
    let mut fields = request_line.split_whitespace();
    let Some(method) = fields.next() else {
        respond(
            &mut stream,
            400,
            "application/json; charset=utf-8",
            "{\"error\":\"invalid_request\"}",
        );
        return;
    };
    let Some(target) = fields.next() else {
        respond(
            &mut stream,
            400,
            "application/json; charset=utf-8",
            "{\"error\":\"invalid_request\"}",
        );
        return;
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let params = match parse_query(query) {
        Ok(params) => params,
        Err(()) => {
            respond(
                &mut stream,
                400,
                "application/json; charset=utf-8",
                "{\"error\":\"invalid_query\"}",
            );
            return;
        }
    };

    let locale = params
        .get("locale")
        .map(|value| Locale::parse(value))
        .unwrap_or(Locale::EnUs);
    let response = match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => {
            respond(&mut stream, 200, "text/html; charset=utf-8", HTML);
            return;
        }
        ("GET", "/app.css") => {
            respond(&mut stream, 200, "text/css; charset=utf-8", CSS);
            return;
        }
        ("GET", "/app.js") => {
            respond(&mut stream, 200, "text/javascript; charset=utf-8", JS);
            return;
        }
        ("GET", "/api/home") => match controller.snapshot(locale, demo_preview_capabilities()) {
            Ok(snapshot) => json_response(&home_payload(snapshot, locale)),
            Err(_) => api_error(503, "home_unavailable", locale),
        },
        ("GET", "/api/search") => {
            let request_id = params
                .get("request_id")
                .and_then(|value| value.parse::<u64>().ok())
                .map(SearchRequestId)
                .unwrap_or_else(|| controller.allocate_search_request_id());
            let text = params.get("q").cloned().unwrap_or_default();
            let query = SearchQuery::new(
                text,
                request_id,
                SearchContext {
                    current_workspace: Some(nagi_model::WorkspaceId(42)),
                    current_app: Some(nagi_model::AppId::from_identifier(b"com.nagi.home")),
                    now_unix_seconds: 1_800_000_000,
                },
                demo_preview_capabilities(),
                locale,
            );
            match controller.search(query) {
                Ok(response) => json_response(&search_payload(response, locale)),
                Err(nagi_home_search::SearchError::StaleRequest) => {
                    (409, json_body(&StalePayload { stale: true }))
                }
                Err(nagi_home_search::SearchError::Cancelled) => {
                    (409, json_body(&StalePayload { stale: true }))
                }
            }
        }
        ("POST", "/api/cancel") => {
            let request_id = params
                .get("request_id")
                .and_then(|value| value.parse::<u64>().ok());
            let cancelled = request_id
                .map(|id| controller.cancel_search(SearchRequestId(id)))
                .unwrap_or(false);
            json_response(&CancelPayload { cancelled })
        }
        _ => (
            404,
            json_body(&ErrorPayload {
                error: "not_found".to_owned(),
            }),
        ),
    };
    let (status, body) = response;
    respond(
        &mut stream,
        status,
        "application/json; charset=utf-8",
        &body,
    );
}

fn demo_preview_capabilities() -> CapabilityContext {
    nagi_home_search::fixtures::demo_capabilities()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HomePayload {
    locale: &'static str,
    strings: BTreeMap<&'static str, String>,
    current_workspace: Option<WorkspacePayload>,
    continuations: Vec<ContinuationPayload>,
    apps: Vec<AppPayload>,
    quick_actions: Vec<QuickActionPayload>,
    source_is_fixture: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspacePayload {
    workspace_id: u64,
    title: String,
    related_object_count: usize,
    action: ActionPayload,
    action_availability: &'static str,
    action_label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContinuationPayload {
    object_id: u64,
    app_id: String,
    title: String,
    subtitle: Option<String>,
    action: ActionPayload,
    action_availability: &'static str,
    action_label: String,
    is_fixture: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppPayload {
    app_id: String,
    title: String,
    description: String,
    icon_asset_key: String,
    icon_glyph: String,
    accent_color: String,
    availability: &'static str,
    availability_label: String,
    action: ActionPayload,
    action_availability: &'static str,
    action_label: String,
    preview_route: Option<String>,
    is_fixture: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QuickActionPayload {
    localization_key: &'static str,
    label: String,
    action: Option<ActionPayload>,
    action_availability: &'static str,
    action_label: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ActionPayload {
    LaunchApp { app_id: String },
    OpenObject { object_id: u64, app_id: String },
    OpenWorkspace { workspace_id: u64 },
    InvokeAction { action_id: String },
    OpenSearch,
    OpenIntentEntry,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchPayload {
    request_id: u64,
    results: Vec<SearchResultPayload>,
    provider_issues: Vec<ProviderIssuePayload>,
    strings: BTreeMap<&'static str, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResultPayload {
    result_id: String,
    provider_id: String,
    identity_kind: &'static str,
    identity_id: String,
    category: &'static str,
    category_label: String,
    title: String,
    subtitle: Option<String>,
    score: i32,
    match_reason: &'static str,
    match_reason_label: String,
    action: Option<ActionPayload>,
    action_availability: Option<&'static str>,
    action_label: Option<String>,
    preview: Option<String>,
    is_fixture: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderIssuePayload {
    provider_id: String,
    kind: &'static str,
    message: String,
}

#[derive(Serialize)]
struct CancelPayload {
    cancelled: bool,
}

#[derive(Serialize)]
struct StalePayload {
    stale: bool,
}

#[derive(Serialize)]
struct ErrorPayload {
    error: String,
}

fn home_payload(snapshot: HomeSnapshot, locale: Locale) -> HomePayload {
    let catalog = LocalizationCatalog;
    let current_workspace = snapshot.current_workspace.map(|workspace| {
        let action = ActionPayload::OpenWorkspace {
            workspace_id: workspace.workspace_id.0,
        };
        let quick_action = snapshot
            .quick_actions
            .iter()
            .find(|quick| quick.localization_key == "home.open_workspace");
        let availability = quick_action
            .map(|quick| availability_key(&quick.availability))
            .unwrap_or("availability.unavailable");
        let label = quick_action
            .map(|quick| availability_label(&catalog, &quick.availability, locale))
            .unwrap_or_else(|| catalog.resolve("availability.unavailable", locale));
        WorkspacePayload {
            workspace_id: workspace.workspace_id.0,
            title: workspace.title,
            related_object_count: workspace.related_object_count,
            action,
            action_availability: availability,
            action_label: label,
        }
    });
    let continuations = snapshot
        .continuations
        .into_iter()
        .map(|item| continuation_payload(item, locale, &catalog))
        .collect();
    let apps = snapshot
        .apps
        .into_iter()
        .map(|app| app_payload(app, locale, &catalog))
        .collect();
    let quick_actions = snapshot
        .quick_actions
        .into_iter()
        .map(|quick| quick_action_payload(quick, locale, &catalog))
        .collect();
    HomePayload {
        locale: locale.as_str(),
        strings: catalog.all_strings(locale),
        current_workspace,
        continuations,
        apps,
        quick_actions,
        source_is_fixture: snapshot.source_is_fixture,
    }
}

fn continuation_payload(
    item: ContinuationReference,
    locale: Locale,
    catalog: &LocalizationCatalog,
) -> ContinuationPayload {
    let action = ActionPayload::OpenObject {
        object_id: item.object_id.0,
        app_id: format!("app:{:016x}", item.app_id.0),
    };
    ContinuationPayload {
        object_id: item.object_id.0,
        app_id: format!("app:{:016x}", item.app_id.0),
        title: item.title,
        subtitle: item.subtitle,
        action,
        action_availability: availability_key(&item.availability),
        action_label: availability_label(catalog, &item.availability, locale),
        is_fixture: true,
    }
}

fn app_payload(app: HomeApp, locale: Locale, catalog: &LocalizationCatalog) -> AppPayload {
    let status = match &app.descriptor.availability {
        AppAvailability::NagiRuntime => "nagi_runtime",
        AppAvailability::HostPreview => "host_preview",
        AppAvailability::ComingSoon => "coming_soon",
        AppAvailability::Unavailable { .. } => "unavailable",
        AppAvailability::Unlaunchable { .. } => "unlaunchable",
    };
    let [red, green, blue] = app.descriptor.icon.accent_rgb;
    AppPayload {
        app_id: format!("app:{:016x}", app.descriptor.app_id.0),
        title: app.descriptor.display_name(catalog, locale),
        description: app.descriptor.description(catalog, locale),
        icon_asset_key: app.descriptor.icon.asset_key,
        icon_glyph: app.descriptor.icon.fallback_glyph.to_string(),
        accent_color: format!("#{red:02x}{green:02x}{blue:02x}"),
        availability: status,
        availability_label: catalog.resolve(app.descriptor.availability.message_key(), locale),
        action: action_payload(app.descriptor.launch_action),
        action_availability: availability_key(&app.action_availability),
        action_label: availability_label(catalog, &app.action_availability, locale),
        preview_route: app.descriptor.preview_route,
        is_fixture: true,
    }
}

fn quick_action_payload(
    quick: HomeQuickAction,
    locale: Locale,
    catalog: &LocalizationCatalog,
) -> QuickActionPayload {
    QuickActionPayload {
        localization_key: quick.localization_key,
        label: catalog.resolve(quick.localization_key, locale),
        action: quick.action.map(action_payload),
        action_availability: availability_key(&quick.availability),
        action_label: availability_label(catalog, &quick.availability, locale),
    }
}

fn search_payload(response: SearchResponse, locale: Locale) -> SearchPayload {
    let catalog = LocalizationCatalog;
    let results = response
        .results
        .into_iter()
        .map(|result| search_result_payload(result, locale, &catalog))
        .collect();
    let provider_issues = response
        .provider_issues
        .into_iter()
        .map(|issue| {
            let (kind, message_key) = match issue.kind {
                ProviderIssueKind::Failed(_) => ("failed", "search.provider_failed"),
                ProviderIssueKind::TimedOut => ("timed_out", "search.provider_timeout"),
                ProviderIssueKind::StartFailed => ("start_failed", "search.provider_failed"),
                ProviderIssueKind::PermissionDenied => {
                    ("permission_denied", "search.provider_denied")
                }
                ProviderIssueKind::DuplicateProvider => ("duplicate", "search.provider_failed"),
            };
            ProviderIssuePayload {
                provider_id: issue.provider_id.as_str().to_owned(),
                kind,
                message: catalog.resolve(message_key, locale),
            }
        })
        .collect();
    SearchPayload {
        request_id: response.request_id.0,
        results,
        provider_issues,
        strings: catalog.all_strings(locale),
    }
}

fn search_result_payload(
    result: SearchResult,
    locale: Locale,
    catalog: &LocalizationCatalog,
) -> SearchResultPayload {
    let (identity_kind, identity_id) = match &result.identity {
        SearchIdentity::App(app) => ("app", format!("app:{:016x}", app.0)),
        SearchIdentity::Object(object) => ("object", format!("object:{:016x}", object.0)),
        SearchIdentity::Workspace(workspace) => {
            ("workspace", format!("workspace:{:016x}", workspace.0))
        }
        SearchIdentity::Action(action) => ("action", format!("action:{action}")),
    };
    let category = result.category.localization_key();
    let availability = result.action_availability.as_ref();
    SearchResultPayload {
        result_id: format!("result:{:016x}", result.result_id),
        provider_id: result.provider_id.as_str().to_owned(),
        identity_kind,
        identity_id,
        category: category
            .strip_prefix("search.category.")
            .unwrap_or("everything"),
        category_label: catalog.resolve(category, locale),
        title: result.title,
        subtitle: result.subtitle,
        score: result.score,
        match_reason: result.match_reason.localization_key(),
        match_reason_label: catalog.resolve(result.match_reason.localization_key(), locale),
        action: result.action.map(action_payload),
        action_availability: availability.map(availability_key),
        action_label: availability.map(|state| availability_label(catalog, state, locale)),
        preview: result.preview,
        is_fixture: result.is_fixture,
    }
}

fn action_payload(action: TypedAction) -> ActionPayload {
    match action {
        TypedAction::LaunchApp { app_id } => ActionPayload::LaunchApp {
            app_id: format!("app:{:016x}", app_id.0),
        },
        TypedAction::OpenObject { object_id, app_id } => ActionPayload::OpenObject {
            object_id: object_id.0,
            app_id: format!("app:{:016x}", app_id.0),
        },
        TypedAction::OpenWorkspace { workspace_id } => ActionPayload::OpenWorkspace {
            workspace_id: workspace_id.0,
        },
        TypedAction::InvokeAction { action_id } => ActionPayload::InvokeAction { action_id },
        TypedAction::OpenSearch => ActionPayload::OpenSearch,
        TypedAction::OpenIntentEntry => ActionPayload::OpenIntentEntry,
    }
}

fn availability_key(availability: &ActionAvailability) -> &'static str {
    match availability {
        ActionAvailability::Ready => "available",
        ActionAvailability::HostPreviewOnly => "host_preview",
        ActionAvailability::ComingSoon { .. } => "coming_soon",
        ActionAvailability::Unavailable { .. } => "unavailable",
        ActionAvailability::PermissionRequired { .. } => "permission_required",
        ActionAvailability::Unlaunchable { .. } => "unlaunchable",
    }
}

fn availability_label(
    catalog: &LocalizationCatalog,
    availability: &ActionAvailability,
    locale: Locale,
) -> String {
    let key = match availability {
        ActionAvailability::Ready => "availability.available",
        ActionAvailability::HostPreviewOnly => "availability.host_preview",
        ActionAvailability::ComingSoon { .. } => "availability.coming_soon",
        ActionAvailability::Unavailable { .. } => "availability.unavailable",
        ActionAvailability::PermissionRequired { .. } => "availability.permission_required",
        ActionAvailability::Unlaunchable { .. } => "availability.unlaunchable",
    };
    catalog.resolve(key, locale)
}

fn json_response<T: Serialize>(value: &T) -> (u16, String) {
    match serde_json::to_string(value) {
        Ok(json) => (200, json),
        Err(_) => api_error(500, "serialization_failed", Locale::EnUs),
    }
}

fn json_body<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_owned())
}

fn api_error(status: u16, error: &str, locale: Locale) -> (u16, String) {
    let _ = locale;
    (
        status,
        json_body(&ErrorPayload {
            error: error.to_owned(),
        }),
    )
}

fn parse_query(query: &str) -> Result<BTreeMap<String, String>, ()> {
    let mut values = BTreeMap::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(key)?;
        let value = percent_decode(value)?;
        if key.len() > 64 || value.len() > 8_192 {
            return Err(());
        }
        values.entry(key).or_insert(value);
    }
    Ok(values)
}

fn percent_decode(value: &str) -> Result<String, ()> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let high = hex(bytes[index + 1]).ok_or(())?;
                let low = hex(bytes[index + 2]).ok_or(())?;
                decoded.push((high << 4) | low);
                index += 3;
            }
            b'%' => return Err(()),
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).map_err(|_| ())
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        414 => "URI Too Long",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; connect-src 'self'; style-src 'self'; script-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(headers.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::percent_decode;

    #[test]
    fn query_decoder_preserves_utf8_and_rejects_malformed_escapes() {
        assert_eq!(
            percent_decode("Home+%E6%A4%9C%E7%B4%A2").unwrap(),
            "Home 検索"
        );
        assert!(percent_decode("bad%2").is_err());
        assert!(percent_decode("%ff").is_err());
    }
}
