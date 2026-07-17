use sqlx::{PgPool, postgres::PgPoolOptions};

#[derive(Debug, Clone)]
pub struct DbPool(PgPool);

impl DbPool {
    pub fn connect_lazy(url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new().max_connections(50).connect_lazy(url)?;
        Ok(Self(pool))
    }
}

impl std::ops::Deref for DbPool {
    type Target = PgPool;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
