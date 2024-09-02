use actix_web::{middleware::Logger, web::Data, App, HttpServer};
use actix_web_httpauth::extractors::basic;
use anyhow::Context;
use diesel::r2d2;
use diesel::PgConnection;
use routes::status::status;
use routes::triggers::trigger_post;
use routes::{index::hello, triggers::trigger_get};
use service::PulseService;
use tracing::info;
use tracing::Level;
use utils::settings::get_settings;

pub mod routes {
    pub mod index;
    pub mod status;
    pub mod triggers;
}
pub mod utils {
    pub mod check_auth;
    pub mod checksum;
    pub mod join_path;
    pub mod settings;
}
pub mod db {
    pub mod models;
    pub mod schema;
}
pub mod service;

pub type DbPool = r2d2::Pool<r2d2::ConnectionManager<PgConnection>>;

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let settings = get_settings().with_context(|| "Failed to get settings")?;

    let hostname = settings.hostname.clone();
    let port = settings.port;
    let database_url = settings.database_url.clone();

    info!("💫 autopulse starting up...");

    let manager = r2d2::ConnectionManager::<PgConnection>::new(database_url);
    let pool = r2d2::Pool::builder()
        .build(manager)
        .with_context(|| "Failed to create connection pool")?;

    let service = PulseService::new(settings.clone(), pool.clone());

    service.start();

    HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .service(hello)
            .service(trigger_get)
            .service(trigger_post)
            .service(status)
            .app_data(basic::Config::default().realm("Restricted area"))
            .app_data(Data::new(settings.clone()))
            .app_data(Data::new(pool.clone()))
            .app_data(Data::new(service.clone()))
    })
    .bind((hostname, port))?
    .run()
    .await
    .with_context(|| "Failed to start server")?;

    Ok(())
}
