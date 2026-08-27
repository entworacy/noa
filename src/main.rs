#![cfg_attr(test, allow(dead_code))]

mod asset;
mod audit;
mod device;
mod failure;
mod intercept;
mod kakao;
mod model;
mod reconcile;
mod settings;
mod web;

use std::sync::Arc;

use actix_web::{App, HttpServer, web as actix_web_data};
use tokio::sync::{RwLock, broadcast};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::{
    audit::AuditLog,
    device::KakaoRelay,
    kakao::RoomCatalog,
    model::{Room, RoomEvent},
    settings::Settings,
    web::AppState,
};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("noa=info")),
        )
        .init();

    let config = Arc::new(Settings::from_env());
    tokio::fs::create_dir_all(&config.data_dir).await?;
    tokio::fs::create_dir_all(&config.upload_dir).await?;
    config.iris_hook.publish().await?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        revision = env!("NOA_BUILD_REVISION"),
        "noa 시작"
    );
    if config.api_token.is_none() {
        warn!("NOA_API_TOKEN이 비어 있어 HTTP API 인증이 비활성화되었습니다");
    }
    if config.iris_hook.enabled {
        info!(
            bridge = %config.iris_hook.bridge_url,
            endpoint_prefix = %config.iris_hook.endpoint_prefix,
            config = %config.iris_hook.config_path.display(),
            types = ?config.iris_hook.types,
            kakao_hook_enabled = config.kakao_hook_enabled,
            "Iris 네이티브 후킹 브리지 활성화"
        );
    }

    let audit = AuditLog::open_archive(&config.audit_db_path())
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let catalog = match RoomCatalog::mount(&config) {
        Ok(catalog) => {
            info!("KakaoTalk 데이터베이스 연결 완료");
            Some(Arc::new(catalog))
        }
        Err(error) => {
            warn!(%error, "KakaoTalk 데이터베이스 없이 제한 모드로 시작합니다");
            None
        }
    };
    let relay = match KakaoRelay::connect(&config) {
        Ok(relay) => {
            info!("Android JNI 전송 계층 준비 완료");
            Some(relay)
        }
        Err(error) => {
            warn!(%error, "Android 전송 기능 없이 시작합니다");
            None
        }
    };

    let rooms = Arc::new(RwLock::new(Vec::<Room>::new()));
    let (live_events, _) = broadcast::channel::<RoomEvent>(256);
    if let Some(catalog) = catalog.clone() {
        reconcile::launch(
            catalog,
            audit.clone(),
            rooms.clone(),
            live_events.clone(),
            config.clone(),
        );
    }

    asset::schedule_reaping(config.upload_dir.clone());
    let state = AppState {
        config: config.clone(),
        catalog,
        audit,
        relay,
        rooms: rooms.clone(),
        live_events,
    };
    let bind = config.bind.clone();
    info!(%bind, "HTTP 대시보드 수신 대기");

    let server = HttpServer::new(move || {
        App::new()
            .app_data(actix_web_data::Data::new(state.clone()))
            .app_data(actix_web_data::PayloadConfig::new(
                state.config.max_upload_bytes,
            ))
            .app_data(
                actix_web_data::JsonConfig::default()
                    .limit(state.config.max_upload_bytes.saturating_mul(2)),
            )
            .configure(web::configure)
    })
    .bind(&bind)?
    .run();
    intercept::launch(config.clone());
    intercept::launch_chatonroom_rotation(rooms, config);
    server.await
}
