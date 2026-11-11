//! Queries the display asks Moonraker for: files, history, Spoolman, power devices and more.

use super::session::Session;
use super::{Event, Query, QUERY_TIMEOUT};
use crate::moonraker::access::{self};
use crate::moonraker::{Call, Connection, Endpoint};
use crate::sources::moonraker_server::files::{self, FileMetadata, ThumbnailImage};
use crate::sources::moonraker_server::gcode_store;
use crate::sources::moonraker_server::history;
use crate::sources::moonraker_server::machine;
use crate::sources::moonraker_server::power;
use crate::sources::moonraker_server::temperature_store;
use crate::sources::moonraker_server::update_manager;
use crate::sources::spoolman;
use serde_json::Value;

impl Session<'_> {
    pub(super) fn answer(&self, query: Query) {
        if query == Query::PrintHistory && !self.components.history {
            log::debug!("no job history: Moonraker has no history component");
            return;
        }
        if matches!(query, Query::Spoolman | Query::SpoolList) && !self.components.spoolman {
            log::debug!("no Spoolman: Moonraker has no spoolman component");
            return;
        }
        let connection = self.connection.clone();
        let downloads = Downloads {
            http: self.http.clone(),
            endpoint: self.endpoint.clone(),
            authorized: self.access.is_authorized(),
        };
        let events = self.outputs.events.clone();
        let lane_spools = self.spoolman_lane_spools.clone().unwrap_or_default();
        let permits = matches!(query, Query::FileDetails { .. }).then(|| self.file_details.clone());
        tokio::spawn(async move {
            // The timeout starts once it is this query's turn.
            let _permit = match permits {
                Some(permits) => permits.acquire_owned().await.ok(),
                None => None,
            };
            let fetched =
                tokio::time::timeout(QUERY_TIMEOUT, fetch(&connection, &downloads, &query, &lane_spools)).await;
            match fetched {
                Ok(Ok(results)) => results.into_iter().for_each(|event| {
                    let _ = events.send(event);
                }),
                Ok(Err(error)) => log::info!("query {query:?} failed: {error}"),
                Err(_) => log::info!("query {query:?} timed out"),
            }
        });
    }
}

/// Files over HTTP (thumbnails); an authorized display shows a one-time token from its connection with each.
struct Downloads {
    http: reqwest::Client,
    endpoint: Endpoint,
    authorized: bool,
}

impl Downloads {
    async fn get(&self, connection: &Connection, path: &str) -> Result<Vec<u8>, String> {
        let mut url = self.endpoint.http_url(path);
        if self.authorized {
            let token = access::oneshot_token(connection).await.map_err(|error| error.to_string())?;
            url = format!("{url}?token={token}");
        }
        let response = self.http.get(url).send().await.map_err(|error| error.to_string())?;
        let response = response.error_for_status().map_err(|error| error.to_string())?;
        Ok(response.bytes().await.map_err(|error| error.to_string())?.to_vec())
    }
}

async fn fetch(
    connection: &Connection,
    downloads: &Downloads,
    query: &Query,
    lane_spools: &[u32],
) -> Result<Vec<Event>, String> {
    let call =
        |call: Call| async move { connection.call(call.method, call.params).await.map_err(|error| error.to_string()) };
    let events = match query {
        Query::Files => vec![Event::Files(files::parse_file_list(&call(files::list_call()).await?))],
        Query::FileDetails { filename } => {
            let result = call(files::metadata_call(filename)).await?;
            let metadata = files::parse_metadata(&result).ok_or("unreadable metadata")?;
            let mut events = Vec::new();
            if let Some(path) = metadata.thumbnail_path() {
                match downloads.get(connection, &path).await {
                    Ok(data) => match ThumbnailImage::decode(&data) {
                        Some(image) => events.push(Event::Thumbnail { filename: filename.clone(), image }),
                        None => log::info!("thumbnail of {filename} cannot be decoded"),
                    },
                    Err(error) => log::info!("thumbnail of {filename} unavailable: {error}"),
                }
            }
            events.insert(0, Event::FileMetadata(Box::new(FileMetadata { filename: filename.clone(), ..metadata })));
            events
        }
        Query::PrintHistory => {
            vec![Event::PrintHistory(history::parse_print_history(&call(history::list_call()).await?))]
        }
        Query::Spoolman => vec![Event::Spoolman(Box::new(spoolman::fetch_state(connection, lane_spools).await?))],
        Query::SpoolList => vec![Event::SpoolList(spoolman::fetch_spool_list(connection).await?)],
        Query::PowerDevices => {
            vec![Event::PowerDevices(power::parse_power_devices(&call(power::devices_call()).await?))]
        }
        Query::Updates => update_status_events(&call(update_manager::status_call()).await?),
        Query::ConsoleHistory => {
            vec![Event::ConsoleHistory(gcode_store::parse_gcode_store(&call(gcode_store::history_call()).await?))]
        }
        Query::HostNetwork => {
            vec![Event::HostNetwork(machine::parse_host_network(&call(machine::system_info_call()).await?))]
        }
        Query::TemperatureHistory => vec![Event::TemperatureHistory(temperature_store::parse_temperature_store(
            &call(temperature_store::store_call()).await?,
        ))],
    };
    Ok(events)
}

/// The versions and whether the update manager is busy, from `machine.update.status` or `notify_update_refreshed`.
pub(super) fn update_status_events(status: &Value) -> Vec<Event> {
    vec![
        Event::Updates(update_manager::parse_update_status(status)),
        Event::UpdateManagerBusy(update_manager::is_busy(status)),
    ]
}
