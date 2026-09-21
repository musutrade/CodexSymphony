//! Validate deployment input before opening a database or listening socket.
use sqlx::postgres::PgConnectOptions;
use std::{env, net::SocketAddr};

pub struct Config {
    pub database_url: String,
    pub bind_address: SocketAddr,
    pub web_origin: String,
}

impl Config {
    pub fn from_env() -> Result<Self, &'static str> {
        Self::parse(
            env::var("DATABASE_URL").or(Err("DATABASE_URL is required"))?,
            &env::var("BIND_ADDRESS").unwrap_or("127.0.0.1:3081".into()),
            env::var("WEB_ORIGIN").or(Err("WEB_ORIGIN is required"))?,
        )
    }

    pub fn parse(
        database_url: String,
        bind_address: &str,
        web_origin: String,
    ) -> Result<Self, &'static str> {
        database_url
            .parse::<PgConnectOptions>()
            .or(Err("DATABASE_URL must be a PostgreSQL URL"))?;
        let bind_address = loopback_address(bind_address)?;
        crate::security::validate_web_origin(&web_origin)?;
        Ok(Self {
            database_url,
            bind_address,
            web_origin,
        })
    }
}

pub fn loopback_address(value: &str) -> Result<SocketAddr, &'static str> {
    let address: SocketAddr = value.parse().or(Err("BIND_ADDRESS must be an IP:port"))?;
    if !address.ip().is_loopback() {
        return Err("Phase 0a BIND_ADDRESS must be loopback");
    }
    Ok(address)
}
