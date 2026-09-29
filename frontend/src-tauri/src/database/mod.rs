pub mod commands;
pub mod manager;
pub mod models;
pub mod repositories;
pub mod setup;

#[cfg(test)]
mod legacy_compat_tests {
    //! scripts/restore-meetily-data.sh hands an Assunta database back to Meetily.
    //! This checks that, after the script's cleanup, a migrator that only knows
    //! Meetily's migrations accepts the database.
    use sqlx::migrate::Migrator;
    use sqlx::sqlite::SqlitePoolOptions;

    const ASSUNTA_FIRST_MIGRATION: i64 = 20260929000000;

    #[tokio::test]
    async fn assunta_database_opens_with_meetily_migrations_after_cleanup() {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        // Meetily's migrations = ours minus the Assunta ones
        let dir = tempfile::tempdir().unwrap();
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations")).unwrap() {
            let path = entry.unwrap().path();
            let version: i64 = path.file_name().unwrap().to_string_lossy()[..14].parse().unwrap();
            if version < ASSUNTA_FIRST_MIGRATION {
                std::fs::copy(&path, dir.path().join(path.file_name().unwrap())).unwrap();
            }
        }
        let meetily = Migrator::new(dir.path()).await.unwrap();

        // Without cleanup Meetily refuses the database
        assert!(meetily.run(&pool).await.is_err());

        sqlx::query("DELETE FROM _sqlx_migrations WHERE version >= ?")
            .bind(ASSUNTA_FIRST_MIGRATION)
            .execute(&pool)
            .await
            .unwrap();
        meetily.run(&pool).await.unwrap();

        // Meetily-style queries still work with the extra columns present
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, folder_path) VALUES ('m', 't', 'x', 'x', NULL)")
            .execute(&pool)
            .await
            .unwrap();
    }
}
