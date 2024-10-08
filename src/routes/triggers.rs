use actix_web::{
    get, post,
    web::{Data, Json, Path, Query},
    HttpRequest, HttpResponse, Responder, Result,
};
use actix_web_httpauth::extractors::basic::BasicAuth;
use std::sync::Arc;
use tracing::debug;

use crate::{
    db::models::{FoundStatus, NewScanEvent},
    service::{manager::PulseManager, triggers::manual::ManualQueryParams, webhooks::EventType},
    utils::{check_auth::check_auth, rewrite::rewrite_path, settings::Trigger},
};

#[post("/triggers/{trigger}")]
pub async fn trigger_post(
    trigger: Path<String>,
    manager: Data<Arc<PulseManager>>,
    auth: BasicAuth,
    body: Json<serde_json::Value>,
) -> Result<impl Responder> {
    if !check_auth(&auth, &manager.settings) {
        return Ok(HttpResponse::Unauthorized().body("Unauthorized"));
    }

    let trigger_settings = manager.settings.triggers.get(&trigger.to_string());

    if trigger_settings.is_none() {
        return Ok(HttpResponse::NotFound().body("Trigger not found"));
    }

    let trigger_settings = trigger_settings.unwrap();

    match trigger_settings {
        Trigger::Manual(_) | Trigger::Notify(_) => {
            Ok(HttpResponse::BadRequest().body("Invalid request"))
        }
        _ => {
            let rewrite = trigger_settings.get_rewrite();
            let paths = trigger_settings.paths(body.into_inner());

            if paths.is_err() {
                return Ok(HttpResponse::BadRequest().body("Invalid request"));
            }

            let paths = paths.unwrap();

            let mut scan_events = vec![];

            for path in &paths {
                let (mut path, search) = path.clone();

                if let Some(rewrite) = &rewrite {
                    path = rewrite_path(path, rewrite);
                }

                let new_scan_event = NewScanEvent {
                    event_source: trigger.to_string(),
                    file_path: path.clone(),
                    found_status: if !search {
                        FoundStatus::Found.into()
                    } else {
                        FoundStatus::NotFound.into()
                    },
                    ..Default::default()
                };

                let scan_event = manager.add_event(&new_scan_event);

                if let Ok(scan_event) = scan_event {
                    scan_events.push(scan_event);
                }
            }

            manager
                .webhooks
                .add_event(
                    EventType::New,
                    Some(trigger.to_string()),
                    &paths
                        .clone()
                        .into_iter()
                        .map(|p| p.0)
                        .collect::<Vec<String>>(),
                )
                .await;

            trigger_settings.tick();

            debug!(
                "added {} file{} from {} trigger",
                scan_events.len(),
                if scan_events.len() > 1 { "s" } else { "" },
                trigger
            );

            if scan_events.len() != paths.len() {
                return Ok(HttpResponse::InternalServerError().body("Failed to add all events"));
            }

            Ok(HttpResponse::Ok().json(scan_events))
        }
    }
}

#[get("/triggers/{trigger}")]
pub async fn trigger_get(
    req: HttpRequest,
    trigger: Path<String>,
    manager: Data<Arc<PulseManager>>,
    auth: BasicAuth,
) -> Result<impl Responder> {
    if !check_auth(&auth, &manager.settings) {
        return Ok(HttpResponse::Unauthorized().body("Unauthorized"));
    }

    let trigger_settings = manager.settings.triggers.get(&trigger.to_string());

    if trigger_settings.is_none() {
        return Ok(HttpResponse::NotFound().body("Trigger not found"));
    }

    let trigger_settings = trigger_settings.unwrap();

    match &trigger_settings {
        Trigger::Manual(trigger_settings) => {
            let query = Query::<ManualQueryParams>::from_query(req.query_string())?;

            let mut file_path = query.path.clone();

            if let Some(rewrite) = &trigger_settings.rewrite {
                file_path = rewrite_path(file_path, rewrite);
            }

            let new_scan_event = NewScanEvent {
                event_source: trigger.to_string(),
                file_path: file_path.clone(),
                file_hash: query.hash.clone(),
                ..Default::default()
            };

            let scan_event = manager.add_event(&new_scan_event);

            if let Err(e) = scan_event {
                return Ok(HttpResponse::InternalServerError().body(e.to_string()));
            }

            manager
                .webhooks
                .add_event(EventType::New, Some(trigger.to_string()), &[file_path])
                .await;

            trigger_settings.timer.tick();

            debug!("added 1 file from {} trigger", trigger);

            let scan_event = scan_event.unwrap();

            Ok(HttpResponse::Ok().json(scan_event))
        }
        _ => Ok(HttpResponse::Ok().body("Not implemented")),
    }
}
